use crate::model::{TimelinePoint, WindowInfo, WindowSource, WindowTrend};
use crate::verdict::CACHE_THRASH_THRESHOLD;

/// Bounded tail-read depth for trend computation (ADR-006, REQ-007).
/// Never load more than this many assistant turns per session.
pub const TREND_TAIL_K: usize = 8;

/// Recency window for velocity: max over the last W positive deltas (ADR-022).
/// Isolated spikes older than W positive-delta turns no longer pin velocity high.
const VELOCITY_WINDOW_W: usize = TREND_TAIL_K / 2;

/// Consecutive low-rho turns required to conclude sustained cache thrash (ADR-023).
/// N=3: filters one-off cold-start / large-content dips (a single bad turn is not thrash);
/// two consecutive turns are borderline; three is clearly a pattern without delaying detection.
pub const SUSTAINED_THRASH_N: usize = 3;

/// Returns true when the last SUSTAINED_THRASH_N points all have cache_hit_ratio < CACHE_THRASH_THRESHOLD.
/// Requires every checked point to have a Some ratio (non-cache providers never fire).
/// Returns false when fewer than SUSTAINED_THRASH_N points exist — the N-turn requirement
/// is the cold-start guard; no separate tau dependency needed (new ADR).
pub fn sustained_cache_thrash(points: &[TimelinePoint]) -> bool {
    if points.len() < SUSTAINED_THRASH_N {
        return false;
    }
    points[points.len() - SUSTAINED_THRASH_N..].iter().all(|p| {
        p.cache_hit_ratio
            .is_some_and(|r| r < CACHE_THRASH_THRESHOLD)
    })
}

/// Compute window occupancy from a usage record (point-in-time or aggregate).
/// window_tokens = input + cache_read + cache_create (all active tokens in the context window).
/// cache_hit_ratio = cache_read / window_tokens, bounded [0,1]; None if no cache split.
pub fn compute_window_info(
    input: u64,
    cache_read: u64,
    cache_create: u64,
    model: &str,
    source: WindowSource,
) -> WindowInfo {
    let window_tokens = input
        .saturating_add(cache_read)
        .saturating_add(cache_create);
    let cache_hit_ratio = cache_hit_ratio(cache_read, cache_create, window_tokens);
    WindowInfo {
        window_tokens,
        model: model.to_string(),
        window_source: source,
        cache_hit_ratio,
    }
}

/// cache_read / window_tokens, bounded [0,1].
/// Returns None when window_tokens is 0 or neither cache field is non-zero (no cache split).
fn cache_hit_ratio(cache_read: u64, cache_create: u64, window_tokens: u64) -> Option<f32> {
    if window_tokens == 0 || (cache_read == 0 && cache_create == 0) {
        return None;
    }
    Some((cache_read as f32 / window_tokens as f32).clamp(0.0, 1.0))
}

/// Compute a floor-trend score: last / min across all points.
/// Returns None if fewer than 2 points or floor is 0.
fn drift_score_from_slice(points: &[TimelinePoint]) -> Option<f32> {
    if points.len() < 2 {
        return None;
    }
    let floor = points.iter().map(|p| p.window_tokens).min().unwrap_or(0);
    let last = points.last()?.window_tokens;
    if floor == 0 {
        return None;
    }
    Some(last as f32 / floor as f32)
}

/// Derive velocity and projection from a bounded per-turn timeline (ADR-006, ADR-022).
///
/// Velocity = MAX of the most recent VELOCITY_WINDOW_W consecutive positive deltas
/// in the post-reset segment (windowed max — ADR-022 refinement over ADR-021 global max).
/// A negative delta signals compaction/reset; we discard the pre-reset segment and
/// compute only over the post-reset tail.
/// <2 post-reset points → velocity = None.
/// Projection = (backstop − last_window) / velocity when velocity > 0;
/// None when current >= backstop (already at or past the recycle backstop).
pub fn compute_trend(points: Vec<TimelinePoint>, backstop: u64) -> WindowTrend {
    let drift_score = drift_score_from_slice(&points);

    if points.len() < 2 {
        return WindowTrend {
            points,
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: None,
            drift_score,
        };
    }

    // Find post-reset start: the index of the first point AFTER the last negative delta.
    let mut post_reset_start = 0usize;
    for i in 1..points.len() {
        if points[i].window_tokens < points[i - 1].window_tokens {
            post_reset_start = i;
        }
    }

    let post_reset = &points[post_reset_start..];
    if post_reset.len() < 2 {
        return WindowTrend {
            points,
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: None,
            drift_score,
        };
    }

    // Collect consecutive positive deltas from the post-reset segment.
    let mut pos_deltas: Vec<u64> = Vec::new();
    for i in 1..post_reset.len() {
        if post_reset[i].window_tokens > post_reset[i - 1].window_tokens {
            pos_deltas.push(post_reset[i].window_tokens - post_reset[i - 1].window_tokens);
        }
    }

    if pos_deltas.is_empty() {
        return WindowTrend {
            points,
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: None,
            drift_score,
        };
    }

    let w_start = pos_deltas.len().saturating_sub(VELOCITY_WINDOW_W);
    let velocity = *pos_deltas[w_start..].iter().max().unwrap();

    let Some(last_pt) = post_reset.last() else {
        return WindowTrend {
            points,
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: None,
            drift_score,
        };
    };
    let current = last_pt.window_tokens;
    let projection = {
        let remaining = backstop.saturating_sub(current);
        let turns = remaining / velocity; // velocity > 0 (built from positive deltas only)
        Some(turns.min(u32::MAX as u64) as u32)
    };

    WindowTrend {
        points,
        velocity_tokens_per_turn: Some(velocity),
        projected_turns_to_recycle: projection,
        drift_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::{ABSOLUTE_RECYCLE_BACKSTOP, CACHE_THRASH_THRESHOLD};
    use chrono::TimeZone;

    fn ts(hour: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 1, 1, hour, 0, 0)
            .unwrap()
    }

    fn pt(hour: u32, window_tokens: u64) -> TimelinePoint {
        TimelinePoint {
            at: ts(hour),
            window_tokens,
            cache_hit_ratio: None,
        }
    }

    fn pt_ratio(hour: u32, window_tokens: u64, ratio: Option<f32>) -> TimelinePoint {
        TimelinePoint {
            at: ts(hour),
            window_tokens,
            cache_hit_ratio: ratio,
        }
    }

    #[test]
    fn test_window_tokens_no_overflow_huge_values() {
        let info = compute_window_info(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.window_tokens, u64::MAX);
    }

    #[test]
    fn test_window_tokens_math_oracle() {
        let info = compute_window_info(
            7_000,
            130_000,
            5_000,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.window_tokens, 142_000);
    }

    #[test]
    fn test_source_propagates() {
        let agg = compute_window_info(1_000, 0, 0, "z-ai/glm-5.2", WindowSource::Aggregate);
        assert_eq!(agg.window_source, WindowSource::Aggregate);
        let lt = compute_window_info(1_000, 0, 0, "z-ai/glm-5.2", WindowSource::LastTurn);
        assert_eq!(lt.window_source, WindowSource::LastTurn);
    }

    // ADR-008: cache_hit_ratio = cache_read / window_tokens, bounded [0,1].
    #[test]
    fn test_cache_ratio_none_when_no_cache_split() {
        let info = compute_window_info(10_000, 0, 0, "claude-sonnet-4-6", WindowSource::LastTurn);
        assert!(info.cache_hit_ratio.is_none());
    }

    #[test]
    fn test_cache_ratio_full_hit() {
        // window_tokens = 0 + 100 + 0 = 100; ratio = 100/100 = 1.0
        let info = compute_window_info(0, 100, 0, "claude-sonnet-4-6", WindowSource::LastTurn);
        assert_eq!(info.cache_hit_ratio, Some(1.0));
    }

    #[test]
    fn test_cache_ratio_partial_hit() {
        // window_tokens = 100 + 50 + 50 = 200; ratio = 50/200 = 0.25
        let info = compute_window_info(100, 50, 50, "claude-sonnet-4-6", WindowSource::LastTurn);
        let r = info.cache_hit_ratio.unwrap();
        assert!((r - 0.25).abs() < 1e-5);
    }

    #[test]
    fn test_cache_ratio_none_when_window_tokens_zero() {
        assert!(cache_hit_ratio(0, 0, 0).is_none());
    }

    #[test]
    fn test_cache_ratio_bounded_at_one() {
        // Adversarial: cache_read > window_tokens → clamp to 1.0
        assert_eq!(cache_hit_ratio(200, 0, 100), Some(1.0)); // 200/100 clamped
    }

    // ADR-006: velocity + projection from trend computation.
    #[test]
    fn test_velocity_simple_growth() {
        // Points: 10k, 20k, 30k → deltas: +10k, +10k → velocity = 10k
        let points = vec![pt(0, 10_000), pt(1, 20_000), pt(2, 30_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(10_000));
        // projection = (128k - 30k) / 10k = 9
        assert_eq!(trend.projected_turns_to_recycle, Some(9));
    }

    #[test]
    fn test_velocity_across_reset() {
        // Points: 50k, 80k, 20k (reset!), 30k, 40k
        // Post-reset: [20k, 30k, 40k] → deltas: +10k, +10k → velocity = 10k
        let points = vec![
            pt(0, 50_000),
            pt(1, 80_000),
            pt(2, 20_000),
            pt(3, 30_000),
            pt(4, 40_000),
        ];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(10_000));
        // projection = (128k - 40k) / 10k = 8
        assert_eq!(trend.projected_turns_to_recycle, Some(8));
    }

    #[test]
    fn test_fewer_than_2_post_reset_points_no_velocity() {
        // Reset leaves only 1 post-reset point → velocity = None
        let points = vec![pt(0, 50_000), pt(1, 80_000), pt(2, 10_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert!(trend.velocity_tokens_per_turn.is_none());
        assert!(trend.projected_turns_to_recycle.is_none());
    }

    #[test]
    fn test_single_point_no_velocity() {
        let points = vec![pt(0, 50_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert!(trend.velocity_tokens_per_turn.is_none());
        assert!(trend.projected_turns_to_recycle.is_none());
    }

    #[test]
    fn test_projection_past_backstop_yields_zero() {
        // current (200k) > backstop (128k) → remaining = 0 → projection = 0
        let points = vec![pt(0, 100_000), pt(1, 200_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(100_000));
        assert_eq!(trend.projected_turns_to_recycle, Some(0));
    }

    #[test]
    fn test_projection_targets_backstop_not_window() {
        // Verify projection uses 128k backstop, not any advertised window size.
        // velocity=10k, current=100k → remaining=128k-100k=28k → turns=2.
        // (If limit were 200k: remaining=100k → turns=10; backstop gives 2.)
        let points = vec![pt(0, 90_000), pt(1, 100_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(10_000));
        assert_eq!(trend.projected_turns_to_recycle, Some(2)); // 28k/10k=2
    }

    #[test]
    fn test_no_positive_deltas_no_velocity() {
        // All deltas are zero or negative (flat then reset)
        let points = vec![pt(0, 50_000), pt(1, 50_000), pt(2, 50_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        // No positive deltas → velocity = None
        assert!(trend.velocity_tokens_per_turn.is_none());
    }

    #[test]
    fn test_trend_points_preserved() {
        let pts = vec![pt(0, 10_000), pt(1, 20_000)];
        let len = pts.len();
        let trend = compute_trend(pts, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.points.len(), len);
    }

    // ADR-022: windowed max — spike at turn T, 4 small-delta turns after → velocity drops.
    // Canonical trace: 9 points total. First 8 deltas are small (+100 each), last 1 is the spike (+79400).
    // That's the "burst just happened" case: spike IS in last W=4 positive deltas → velocity = 79400.
    #[test]
    fn test_velocity_burst_just_happened_still_over() {
        // 9 points: t0..t8; deltas d0..d7 = [100,100,100,100,100,100,100,79400]
        // spike is d7 (most recent) → in last W=4 window → velocity = 79400
        let mut points = vec![pt(0, 40_000)];
        for i in 1..=7u32 {
            points.push(pt(i, 40_000 + (i as u64) * 100));
        }
        points.push(pt(8, 40_700 + 79_400)); // w_n = 120_100
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(79_400));
        // tau = (128000 - 120100) / 79400 = 0 → Over
        assert_eq!(trend.projected_turns_to_recycle, Some(0));
    }

    // ADR-022: isolated spike aged out after W=4 subsequent positive-delta turns.
    // Trace (K=8 tail): spike at d2, then 4 small positive deltas d3..d6.
    // Last W=4 positive deltas = [d3,d4,d5,d6] = [100,100,100,100] → velocity = 100.
    #[test]
    fn test_isolated_spike_decays_after_w_turns() {
        // 8 points → 7 deltas: d0=100, d1=100, d2=79400(spike), d3=100, d4=100, d5=100, d6=100
        let base = 10_000u64;
        let points = vec![
            pt(0, base),
            pt(1, base + 100),
            pt(2, base + 200),
            pt(3, base + 200 + 79_400), // spike
            pt(4, base + 200 + 79_400 + 100),
            pt(5, base + 200 + 79_400 + 200),
            pt(6, base + 200 + 79_400 + 300),
            pt(7, base + 200 + 79_400 + 400),
        ];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        // last W=4 pos deltas: [d3,d4,d5,d6] = [100,100,100,100] → velocity = 100
        assert_eq!(trend.velocity_tokens_per_turn, Some(100));
    }

    // ADR-022: sustained burst — all recent deltas large → velocity stays high.
    // 8 points, all deltas = 5000 → last W=4 are all 5000 → velocity = 5000.
    #[test]
    fn test_sustained_burst_velocity_stays_high() {
        let points = vec![
            pt(0, 10_000),
            pt(1, 15_000),
            pt(2, 20_000),
            pt(3, 25_000),
            pt(4, 30_000),
            pt(5, 35_000),
            pt(6, 40_000),
            pt(7, 45_000),
        ];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(5_000));
        // tau = (128k - 45k) / 5k = 83k/5k = 16 → Nearing range
        assert_eq!(trend.projected_turns_to_recycle, Some(16));
    }

    // ADR-022 trailing-edge boundary: spike at oldest position still inside window (w_start).
    // 7 positive deltas: d0-d2 small, d3=spike, d4-d6 small.
    // n=7, W=4, w_start=3 → last W=[d3,d4,d5,d6] → spike still captured → velocity=spike.
    // One more positive delta would push w_start to 4, aging the spike out.
    #[test]
    fn test_spike_at_window_boundary_still_warns() {
        let base = 10_000u64;
        let spike = 79_400u64;
        let points = vec![
            pt(0, base),
            pt(1, base + 100),
            pt(2, base + 200),
            pt(3, base + 300),
            pt(4, base + 300 + spike), // d3 = spike; w_start=3 when 7 deltas exist
            pt(5, base + 300 + spike + 100),
            pt(6, base + 300 + spike + 200),
            pt(7, base + 300 + spike + 300),
        ];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(spike));
    }

    // velocity picks single delta when only one positive delta.
    #[test]
    fn test_velocity_single_delta() {
        let points = vec![pt(0, 10_000), pt(1, 50_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(40_000));
    }

    // velocity picks larger delta when two positive deltas (both in window).
    #[test]
    fn test_velocity_picks_max_of_two_deltas() {
        let points = vec![pt(0, 10_000), pt(1, 11_000), pt(2, 21_000)];
        // deltas: [1k, 10k] → both in last W=4 → max = 10k
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(10_000));
    }

    // sustained_cache_thrash tests (new ADR)

    #[test]
    fn test_sustained_thrash_fires_n_consecutive_low() {
        // N=3 consecutive low-ratio points → true
        let low = CACHE_THRASH_THRESHOLD - 0.01;
        let points = vec![
            pt_ratio(0, 10_000, Some(low)),
            pt_ratio(1, 10_000, Some(low)),
            pt_ratio(2, 10_000, Some(low)),
        ];
        assert!(sustained_cache_thrash(&points));
    }

    #[test]
    fn test_sustained_thrash_no_fire_single_dip() {
        // Only last 1 of 3 is low — no sustained thrash
        let low = CACHE_THRASH_THRESHOLD - 0.01;
        let high = CACHE_THRASH_THRESHOLD + 0.10;
        let points = vec![
            pt_ratio(0, 10_000, Some(high)),
            pt_ratio(1, 10_000, Some(high)),
            pt_ratio(2, 10_000, Some(low)),
        ];
        assert!(!sustained_cache_thrash(&points));
    }

    #[test]
    fn test_sustained_thrash_no_fire_fewer_than_n_points() {
        // Only 2 points (< N=3) → cold-start guard → false
        let low = CACHE_THRASH_THRESHOLD - 0.01;
        let points = vec![
            pt_ratio(0, 10_000, Some(low)),
            pt_ratio(1, 10_000, Some(low)),
        ];
        assert!(!sustained_cache_thrash(&points));
    }

    #[test]
    fn test_sustained_thrash_no_fire_healthy_ratio() {
        // All high ratios → false
        let high = CACHE_THRASH_THRESHOLD + 0.50;
        let points = vec![
            pt_ratio(0, 10_000, Some(high)),
            pt_ratio(1, 10_000, Some(high)),
            pt_ratio(2, 10_000, Some(high)),
        ];
        assert!(!sustained_cache_thrash(&points));
    }

    #[test]
    fn test_sustained_thrash_no_fire_none_ratio() {
        // None ratio (non-cache provider) → false even with N points
        let points = vec![
            pt_ratio(0, 10_000, None),
            pt_ratio(1, 10_000, None),
            pt_ratio(2, 10_000, None),
        ];
        assert!(!sustained_cache_thrash(&points));
    }

    #[test]
    fn test_sustained_thrash_no_fire_at_exact_threshold() {
        // ratio == CACHE_THRASH_THRESHOLD is NOT strictly less than → false (strict boundary)
        let at = CACHE_THRASH_THRESHOLD;
        let points = vec![
            pt_ratio(0, 10_000, Some(at)),
            pt_ratio(1, 10_000, Some(at)),
            pt_ratio(2, 10_000, Some(at)),
        ];
        assert!(!sustained_cache_thrash(&points));
    }

    #[test]
    fn test_drift_score_none_insufficient() {
        let points = vec![pt(0, 10_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert!(trend.drift_score.is_none(), "single point → None");
    }

    #[test]
    fn test_drift_score_monotone_rising() {
        let points = vec![
            pt(0, 10_000),
            pt(1, 12_000),
            pt(2, 14_000),
            pt(3, 16_000),
            pt(4, 18_000),
            pt(5, 20_000),
            pt(6, 22_000),
            pt(7, 24_000),
        ];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        let s = trend.drift_score.expect("8 rising points → Some");
        assert!(s > 1.0, "last(24k)/floor(10k) = 2.4 > 1.0");
    }

    #[test]
    fn test_drift_score_fires_after_reset() {
        // floor=10k (post-reset min), last=20k → score=2.0 > threshold 1.5
        let points = vec![
            pt(0, 50_000),
            pt(1, 80_000),
            pt(2, 10_000), // reset → new floor
            pt(3, 15_000),
            pt(4, 20_000),
        ];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        let s = trend.drift_score.expect("reset scenario → Some");
        assert!(s > 1.5, "last(20k)/floor(10k) = 2.0 > 1.5");
    }

    #[test]
    fn test_drift_score_flat_no_growth() {
        let points = vec![pt(0, 30_000), pt(1, 30_000), pt(2, 30_000)];
        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        let s = trend.drift_score.expect("flat → Some(1.0)");
        assert!((s - 1.0).abs() < 1e-5, "flat series → score = 1.0");
    }

    #[test]
    fn test_sustained_thrash_checks_last_n_of_longer_series() {
        // 5 points: first 2 are healthy, last 3 are low → fires (only last N checked)
        let low = CACHE_THRASH_THRESHOLD - 0.01;
        let high = CACHE_THRASH_THRESHOLD + 0.50;
        let points = vec![
            pt_ratio(0, 10_000, Some(high)),
            pt_ratio(1, 10_000, Some(high)),
            pt_ratio(2, 10_000, Some(low)),
            pt_ratio(3, 10_000, Some(low)),
            pt_ratio(4, 10_000, Some(low)),
        ];
        assert!(sustained_cache_thrash(&points));
    }
}
