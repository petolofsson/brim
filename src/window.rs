use crate::model::{TimelinePoint, WindowInfo, WindowSource, WindowTrend};

/// Bounded tail-read depth for trend computation (ADR-006, REQ-007).
/// Never load more than this many assistant turns per session.
pub const TREND_TAIL_K: usize = 8;

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

/// Derive velocity and projection from a bounded per-turn timeline (ADR-006).
///
/// Velocity = median of consecutive positive window-token deltas in the
/// post-reset segment.  A negative delta signals compaction/reset; we discard
/// the pre-reset segment and compute only over the post-reset tail.
/// <2 post-reset points → velocity = None.
/// Projection = (backstop − last_window) / velocity when velocity > 0;
/// None when current >= backstop (already at or past the recycle backstop).
pub fn compute_trend(points: Vec<TimelinePoint>, backstop: u64) -> WindowTrend {
    if points.len() < 2 {
        return WindowTrend {
            points,
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: None,
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
        };
    }

    pos_deltas.sort_unstable();
    // Upper-median for even-length arrays (slightly pessimistic — earlier warning).
    let velocity = pos_deltas[pos_deltas.len() / 2];

    let Some(last_pt) = post_reset.last() else {
        return WindowTrend {
            points,
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: None,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::ABSOLUTE_RECYCLE_BACKSTOP;
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
}
