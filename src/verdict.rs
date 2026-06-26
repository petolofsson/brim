#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    Nearing,
    Over,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Nearing => "nearing",
            Verdict::Over => "over -> recycle",
        }
    }

    /// Stable enum string for JSON output (REQ-005).
    pub fn as_json_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Nearing => "nearing",
            Verdict::Over => "over_recycle",
        }
    }
}

/// Which gate fired (ADR-025 vote-counter; ADR-010 gates for single-family Nearing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictGate {
    /// (c) absolute active tokens ≥ recycle_backstop (ADR-010; preserved for backward compat)
    #[allow(dead_code)]
    AbsoluteBackstop,
    /// (b) projected turns to recycle ≤ PROJECTION_RECYCLE_TURNS (single speed family, Nearing)
    ProjectionRecycle,
    /// (c) absolute active tokens ≥ watch_tokens
    AbsoluteWatch,
    /// (b) projected turns to recycle ≤ PROJECTION_NEARING_TURNS
    ProjectionNearing,
    /// (d) sustained cache_hit_ratio < CACHE_THRASH_THRESHOLD over N consecutive turns (Nearing only, ADR-008/new ADR)
    CacheThrash,
    /// stop_reason=max_tokens or confirmed tool repetition loop (ADR-025 decisive override)
    DecisiveOverride,
    /// 2+ families fired in the vote-counter (ADR-025)
    FamilyVote,
}

impl VerdictGate {
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictGate::AbsoluteBackstop => "abs-backstop",
            VerdictGate::ProjectionRecycle => "projection_recycle",
            VerdictGate::AbsoluteWatch => "abs-watch",
            VerdictGate::ProjectionNearing => "projection",
            VerdictGate::CacheThrash => "cache-thrash",
            VerdictGate::DecisiveOverride => "decisive_override",
            VerdictGate::FamilyVote => "family_vote",
        }
    }

    pub fn as_json_str(self) -> &'static str {
        match self {
            VerdictGate::AbsoluteBackstop => "absolute_backstop",
            VerdictGate::ProjectionRecycle => "projection_recycle",
            VerdictGate::AbsoluteWatch => "absolute_watch",
            VerdictGate::ProjectionNearing => "projection_nearing",
            VerdictGate::CacheThrash => "cache_thrash",
            VerdictGate::DecisiveOverride => "decisive_override",
            VerdictGate::FamilyVote => "family_vote",
        }
    }
}

// ADR-010 §4: watch ≈ 32k — research-anchored onset of measured degradation;
// most models at or below half-baseline by this token count (absolute, model-agnostic).
pub const ABSOLUTE_WATCH_TOKENS: u64 = 32_000;

// ADR-010 §4: pragmatic recycle backstop = 4× watch. The absolute-budget gate
// does NOT fire mechanically at 32k; 128k is a conservative model-agnostic
// backstop supplementing projection (b) and cache-thrash (d) in the OR gate.
pub const ABSOLUTE_RECYCLE_BACKSTOP: u64 = 128_000;

pub const PROJECTION_NEARING_TURNS: u32 = 5;
const PROJECTION_RECYCLE_TURNS: u32 = 2;
/// Fraction below which cache-read utilisation signals context thrash (ADR-008).
/// Exposed pub so window::sustained_cache_thrash can use the shared constant.
pub const CACHE_THRASH_THRESHOLD: f32 = 0.20;

// ADR-025 Behavior family thresholds (all candidates, pending calibration).
pub const BEHAVIOR_REPETITION_THRESHOLD: u32 = 3; // candidate, pending calibration
pub const BEHAVIOR_STREAK_THRESHOLD: u32 = 3; // candidate, pending calibration
pub const BEHAVIOR_PING_PONG_THRESHOLD: u32 = 3; // candidate, pending calibration
/// Drift family: last/floor ratio above this fires Drift family (ADR-025).
pub const DRIFT_SCORE_THRESHOLD: f32 = 1.5; // candidate, pending calibration

/// Per-session behavioral degradation signals extracted from tool-call structure
/// (ADR-024 Tier-B; used by the Behavior family in the vote-counter, ADR-025).
/// All fields are Option/bool so absent signals don't vote.
/// Thresholds applied in verdict.rs; extraction is provider-specific.
#[derive(Debug, Clone)]
pub struct BehaviorSignals {
    /// Max consecutive identical (tool_name, args_hash) run across the tail window.
    /// Threshold candidate: >= 3 fires Behavior family (and qualifies for decisive override).
    pub repetition_run: Option<u32>,
    /// Max consecutive failed tool results (is_error / status=failed / exit-code != 0).
    /// Threshold candidate: >= 3.
    pub failure_streak: Option<u32>,
    /// Count of A→B→A alternations (ping-pong, no-progress qualifier) in the tool sequence.
    /// Threshold candidate: >= 3.
    pub ping_pong_count: Option<u32>,
    /// True if any assistant turn within the last TREND_TAIL_K turns carried stop_reason=max_tokens.
    pub stop_reason_max_tokens: bool,
}

impl BehaviorSignals {
    /// Compute BehaviorSignals from raw tool call sequence and error flags.
    /// Returns None when all inputs are empty/false (no tool activity in window).
    pub fn from_signals(
        tool_calls: &[(String, u64)],
        error_flags: &[bool],
        stop_reason_max_tokens: bool,
    ) -> Option<Self> {
        if tool_calls.is_empty() && error_flags.is_empty() && !stop_reason_max_tokens {
            return None;
        }

        // repetition_run: max consecutive identical (name, hash) run
        let repetition_run = if tool_calls.is_empty() {
            None
        } else {
            let mut max_run: u32 = 1;
            let mut cur_run: u32 = 1;
            for i in 1..tool_calls.len() {
                if tool_calls[i] == tool_calls[i - 1] {
                    cur_run = cur_run.saturating_add(1);
                    if cur_run > max_run {
                        max_run = cur_run;
                    }
                } else {
                    cur_run = 1;
                }
            }
            if max_run >= 2 { Some(max_run) } else { None }
        };

        // failure_streak: max consecutive true (error) flags
        let failure_streak = if error_flags.is_empty() {
            None
        } else {
            let mut max_streak: u32 = 0;
            let mut cur_streak: u32 = 0;
            for &e in error_flags {
                if e {
                    cur_streak = cur_streak.saturating_add(1);
                    if cur_streak > max_streak {
                        max_streak = cur_streak;
                    }
                } else {
                    cur_streak = 0;
                }
            }
            if max_streak >= 1 {
                Some(max_streak)
            } else {
                None
            }
        };

        // ping_pong_count: count of A→B→A windows where name[i]==name[i+2] && hash[i]==hash[i+2]
        let ping_pong_count = if tool_calls.len() < 3 {
            None
        } else {
            let mut count: u32 = 0;
            for i in 0..tool_calls.len() - 2 {
                if tool_calls[i].0 == tool_calls[i + 2].0 && tool_calls[i].1 == tool_calls[i + 2].1
                {
                    count = count.saturating_add(1);
                }
            }
            if count > 0 { Some(count) } else { None }
        };

        Some(BehaviorSignals {
            repetition_run,
            failure_streak,
            ping_pong_count,
            stop_reason_max_tokens,
        })
    }
}

/// Absolute-token thresholds for the quality verdict (REQ-004, ADR-010 caveat #1).
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Absolute active-token watch band (default: ABSOLUTE_WATCH_TOKENS per ADR-010 research).
    pub watch_tokens: u64,
    /// Absolute recycle backstop (default: ABSOLUTE_RECYCLE_BACKSTOP per ADR-010 research).
    pub recycle_backstop: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            watch_tokens: ABSOLUTE_WATCH_TOKENS,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
        }
    }
}

/// Quality verdict: OR of ADR-010 signals (b), (c), (d).
/// Signal (a) behavioral degradation is deferred per ADR-010.
/// `watch_tokens` and `recycle_backstop` are runtime-configurable (REQ-004, ADR-010 caveat #1).
/// `sustained_cache_thrash`: pre-computed by window::sustained_cache_thrash; fires independently of tau.
/// Returns the verdict and the highest-severity gate that fired.
/// Preserved for backward compatibility (ADR-025).
#[allow(dead_code)]
pub fn absolute_verdict(
    window_tokens: u64,
    projected_turns: Option<u32>,
    sustained_cache_thrash: bool,
    watch_tokens: u64,
    recycle_backstop: u64,
) -> (Verdict, Option<VerdictGate>) {
    // Over gates (highest severity first)
    if window_tokens >= recycle_backstop {
        return (Verdict::Over, Some(VerdictGate::AbsoluteBackstop));
    }
    if projected_turns.is_some_and(|t| t <= PROJECTION_RECYCLE_TURNS) {
        return (Verdict::Over, Some(VerdictGate::ProjectionRecycle));
    }
    // Nearing gates
    if window_tokens >= watch_tokens {
        return (Verdict::Nearing, Some(VerdictGate::AbsoluteWatch));
    }
    if projected_turns.is_some_and(|t| t <= PROJECTION_NEARING_TURNS) {
        return (Verdict::Nearing, Some(VerdictGate::ProjectionNearing));
    }
    // Sustained cache-thrash: rho < theta over N consecutive turns (Nearing only).
    // Fires independently of tau — N-turn persistence is the cold-start guard.
    if sustained_cache_thrash {
        return (Verdict::Nearing, Some(VerdictGate::CacheThrash));
    }
    (Verdict::Ok, None)
}

/// Presentation tier from the vote-counter engine (ADR-025 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Lean,
    Drift,
    Bloated,
    Stale,
    Critical,
}

impl Tier {
    pub fn as_json_str(self) -> &'static str {
        match self {
            Tier::Lean => "lean",
            Tier::Drift => "drift",
            Tier::Bloated => "bloated",
            Tier::Stale => "stale",
            Tier::Critical => "critical",
        }
    }
}

/// All inputs for the 5-family vote-counter (ADR-025 §1-§3).
pub struct FamilyVoteInputs<'a> {
    pub window_tokens: u64,
    pub watch_tokens: u64,
    /// Absolute recycle backstop; decisive override fires when window_tokens >= this.
    pub recycle_backstop: u64,
    pub projected_turns: Option<u32>,
    pub sustained_cache_thrash: bool,
    /// Behavioral degradation signals; None = provider has no tool-use extraction.
    pub behavior: Option<&'a BehaviorSignals>,
    /// Floor-trend drift score from window.rs (ADR-025 Drift family).
    pub drift_score: Option<f32>,
}

/// Result of the vote-counter verdict engine (ADR-025).
pub struct VoteResult {
    /// [volume, speed, thrash, behavior, drift]
    pub families: [bool; 5],
    pub count: u8,
    pub decisive_override: bool,
    pub tier: Tier,
    pub verdict: Verdict,
    pub verdict_gate: Option<VerdictGate>,
}

/// Volume family: context simply too big (ADR-010 absolute gates demoted to weak vote).
pub fn volume_fires(window_tokens: u64, watch_tokens: u64) -> bool {
    window_tokens >= watch_tokens
}

/// Speed family: occupancy climbing fast (velocity + projection, ADR-006/022).
pub fn speed_fires(projected_turns: Option<u32>, nearing_turns: u32) -> bool {
    projected_turns.is_some_and(|t| t <= nearing_turns)
}

/// Thrash family: compaction churn / wall-hit (ADR-008/023 + stop_reason).
pub fn thrash_fires(sustained_cache_thrash: bool, stop_reason_max_tokens: bool) -> bool {
    sustained_cache_thrash || stop_reason_max_tokens
}

/// Behavior family: agent stuck/spinning — the true-onset detector (ADR-024).
/// Candidate thresholds (pending calibration, ADR-025 §3 roadmap #3):
///   repetition >= BEHAVIOR_REPETITION_THRESHOLD
///   streak >= BEHAVIOR_STREAK_THRESHOLD
///   ping_pong >= BEHAVIOR_PING_PONG_THRESHOLD
pub fn behavior_fires(signals: Option<&BehaviorSignals>) -> bool {
    let Some(s) = signals else {
        return false;
    };
    s.repetition_run
        .is_some_and(|r| r >= BEHAVIOR_REPETITION_THRESHOLD)
        || s.failure_streak
            .is_some_and(|r| r >= BEHAVIOR_STREAK_THRESHOLD)
        || s.ping_pong_count
            .is_some_and(|r| r >= BEHAVIOR_PING_PONG_THRESHOLD)
}

/// Drift family: slow session-long rot a short tail misses (ADR-025).
// deferred to roadmap #2 (long-horizon EWMA across resets); excluded from count until independent
pub fn drift_fires(drift_score: Option<f32>) -> bool {
    drift_score.is_some_and(|s| s > DRIFT_SCORE_THRESHOLD)
}

/// Vote-counter verdict engine (ADR-025).
/// Replaces the ADR-010 OR-gate as the primary verdict path.
pub fn family_vote_verdict(inputs: &FamilyVoteInputs) -> VoteResult {
    let stop_reason_max_tokens = inputs.behavior.is_some_and(|b| b.stop_reason_max_tokens);

    let families = [
        volume_fires(inputs.window_tokens, inputs.watch_tokens),
        speed_fires(inputs.projected_turns, PROJECTION_NEARING_TURNS),
        thrash_fires(inputs.sustained_cache_thrash, stop_reason_max_tokens),
        behavior_fires(inputs.behavior),
        drift_fires(inputs.drift_score),
    ];

    // Count from 4 core families only; Drift (index 4) is excluded until independent
    // (roadmap #2: long-horizon EWMA across resets).
    let count = families[..4].iter().filter(|&&f| f).count() as u8;

    // Decisive override: stop_reason_max_tokens OR confirmed tool repetition loop OR
    // occupancy at/above the absolute backstop (restores Over path for behavior-blind providers).
    let decisive_override = stop_reason_max_tokens
        || inputs.behavior.is_some_and(|b| {
            b.repetition_run
                .is_some_and(|r| r >= BEHAVIOR_REPETITION_THRESHOLD)
        })
        || inputs.window_tokens >= inputs.recycle_backstop;

    let tier = if decisive_override {
        Tier::Critical
    } else if count == 0 {
        Tier::Lean
    } else if count == 1 {
        Tier::Drift
    } else if families[3] {
        // 2+ families AND Behavior fires → Critical
        Tier::Critical
    } else if families[4] {
        // 2+ families AND Drift fires → Stale
        Tier::Stale
    } else {
        // 2+ families, neither Behavior nor Drift → Bloated
        Tier::Bloated
    };

    let verdict = match tier {
        Tier::Lean => Verdict::Ok,
        Tier::Drift => Verdict::Nearing,
        Tier::Bloated | Tier::Stale | Tier::Critical => Verdict::Over,
    };

    let verdict_gate = if decisive_override {
        Some(VerdictGate::DecisiveOverride)
    } else if count >= 2 {
        Some(VerdictGate::FamilyVote)
    } else if count == 1 {
        single_family_gate(&families, inputs)
    } else {
        None
    };

    VoteResult {
        families,
        count,
        decisive_override,
        tier,
        verdict,
        verdict_gate,
    }
}

fn single_family_gate(families: &[bool; 5], inputs: &FamilyVoteInputs) -> Option<VerdictGate> {
    if families[0] {
        return Some(VerdictGate::AbsoluteWatch);
    }
    if families[1] {
        return if inputs
            .projected_turns
            .is_some_and(|t| t <= PROJECTION_RECYCLE_TURNS)
        {
            Some(VerdictGate::ProjectionRecycle)
        } else {
            Some(VerdictGate::ProjectionNearing)
        };
    }
    if families[2] {
        return Some(VerdictGate::CacheThrash);
    }
    // Behavior (3) and Drift (4): no single-family gate
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // absolute_verdict gate tests (ADR-010 §3)

    #[test]
    fn absolute_verdict_ok_below_watch() {
        let (v, gate) = absolute_verdict(
            10_000,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Ok);
        assert_eq!(gate, None);
    }

    #[test]
    fn absolute_verdict_nearing_at_watch() {
        let (v, gate) = absolute_verdict(
            ABSOLUTE_WATCH_TOKENS,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::AbsoluteWatch));
    }

    #[test]
    fn absolute_verdict_nearing_above_watch() {
        let (v, gate) = absolute_verdict(
            50_000,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::AbsoluteWatch));
    }

    #[test]
    fn absolute_verdict_over_at_backstop() {
        let (v, gate) = absolute_verdict(
            ABSOLUTE_RECYCLE_BACKSTOP,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::AbsoluteBackstop));
    }

    #[test]
    fn absolute_verdict_over_above_backstop() {
        let (v, gate) = absolute_verdict(
            200_000,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::AbsoluteBackstop));
    }

    #[test]
    fn absolute_verdict_over_projection_recycle() {
        let (v, gate) = absolute_verdict(
            20_000,
            Some(1),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::ProjectionRecycle));
    }

    #[test]
    fn absolute_verdict_over_projection_at_boundary() {
        let (v, gate) = absolute_verdict(
            20_000,
            Some(PROJECTION_RECYCLE_TURNS),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::ProjectionRecycle));
    }

    #[test]
    fn absolute_verdict_nearing_projection_nearing() {
        let (v, gate) = absolute_verdict(
            10_000,
            Some(PROJECTION_NEARING_TURNS),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::ProjectionNearing));
    }

    #[test]
    fn absolute_verdict_nearing_cache_thrash() {
        // sustained_cache_thrash=true fires Nearing regardless of projected_turns (tau-independent).
        let (v, gate) = absolute_verdict(
            10_000,
            None,
            true,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::CacheThrash));
    }

    #[test]
    fn absolute_verdict_cache_thrash_tau_independence() {
        // tau present (projected_turns=Some) also fires — confirming no dependency on tau.
        let (v, gate) = absolute_verdict(
            10_000,
            Some(10),
            true,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::CacheThrash));
    }

    #[test]
    fn absolute_verdict_cache_thrash_false_no_fire() {
        // sustained_cache_thrash=false → Ok; single-turn dips handled by window::sustained_cache_thrash.
        let (v, gate) = absolute_verdict(
            10_000,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Ok);
        assert_eq!(gate, None);
    }

    #[test]
    fn absolute_verdict_backstop_priority_over_projection() {
        // Both backstop and projection fire; backstop is checked first (highest severity)
        let (v, gate) = absolute_verdict(
            ABSOLUTE_RECYCLE_BACKSTOP,
            Some(1),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::AbsoluteBackstop));
    }

    #[test]
    fn absolute_verdict_projection_beats_watch() {
        // projection fires Over; window_tokens alone would only give Nearing
        let (v, gate) = absolute_verdict(
            40_000,
            Some(1),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::ProjectionRecycle));
    }

    // N3: boundary tests

    #[test]
    fn absolute_verdict_ok_just_below_watch() {
        // watch_tokens - 1 = 31_999 → must be Ok (watch boundary is inclusive >=)
        let (v, gate) = absolute_verdict(
            ABSOLUTE_WATCH_TOKENS - 1,
            None,
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Ok);
        assert_eq!(gate, None);
    }

    #[test]
    fn absolute_verdict_cache_thrash_sustained_false_ok() {
        // sustained_cache_thrash=false never fires (strict gate — only bool true escalates)
        let (v, gate) = absolute_verdict(
            10_000,
            Some(10),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Ok);
        assert_eq!(gate, None);
    }

    #[test]
    fn absolute_verdict_projection_mid_band() {
        // projected_turns = 3: > PROJECTION_RECYCLE_TURNS (2) and <= PROJECTION_NEARING_TURNS (5)
        let (v, gate) = absolute_verdict(
            10_000,
            Some(3),
            false,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::ProjectionNearing));
    }

    #[test]
    fn absolute_verdict_absolute_gates_beat_cache_thrash() {
        // Absolute backstop fires Over even when sustained_cache_thrash=true (absolute wins).
        let (v, gate) = absolute_verdict(
            ABSOLUTE_RECYCLE_BACKSTOP,
            None,
            true,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::AbsoluteBackstop));
    }

    // ── family predicate tests (ADR-025) ──────────────────────────────────

    #[test]
    fn family_volume_fires_at_watch() {
        assert!(volume_fires(32_000, 32_000));
    }

    #[test]
    fn family_volume_no_fire_below_watch() {
        assert!(!volume_fires(31_999, 32_000));
    }

    #[test]
    fn family_speed_fires_at_nearing() {
        assert!(speed_fires(
            Some(PROJECTION_NEARING_TURNS),
            PROJECTION_NEARING_TURNS
        ));
    }

    #[test]
    fn family_speed_no_fire_none() {
        assert!(!speed_fires(None, PROJECTION_NEARING_TURNS));
    }

    #[test]
    fn family_thrash_fires_cache() {
        assert!(thrash_fires(true, false));
    }

    #[test]
    fn family_thrash_fires_stop_reason() {
        assert!(thrash_fires(false, true));
    }

    #[test]
    fn family_thrash_no_fire() {
        assert!(!thrash_fires(false, false));
    }

    #[test]
    fn family_behavior_fires_repetition() {
        let s = BehaviorSignals {
            repetition_run: Some(BEHAVIOR_REPETITION_THRESHOLD),
            failure_streak: None,
            ping_pong_count: None,
            stop_reason_max_tokens: false,
        };
        assert!(behavior_fires(Some(&s)));
    }

    #[test]
    fn family_behavior_fires_streak() {
        let s = BehaviorSignals {
            repetition_run: None,
            failure_streak: Some(BEHAVIOR_STREAK_THRESHOLD),
            ping_pong_count: None,
            stop_reason_max_tokens: false,
        };
        assert!(behavior_fires(Some(&s)));
    }

    #[test]
    fn family_behavior_fires_pingpong() {
        let s = BehaviorSignals {
            repetition_run: None,
            failure_streak: None,
            ping_pong_count: Some(BEHAVIOR_PING_PONG_THRESHOLD),
            stop_reason_max_tokens: false,
        };
        assert!(behavior_fires(Some(&s)));
    }

    #[test]
    fn family_behavior_no_fire_none() {
        assert!(!behavior_fires(None));
    }

    #[test]
    fn family_drift_fires() {
        assert!(drift_fires(Some(DRIFT_SCORE_THRESHOLD + 0.1)));
    }

    #[test]
    fn family_drift_no_fire_none() {
        assert!(!drift_fires(None));
    }

    // ── vote-counter engine tests (ADR-025) ───────────────────────────────

    fn lean_inputs(watch: u64) -> FamilyVoteInputs<'static> {
        FamilyVoteInputs {
            window_tokens: 1_000,
            watch_tokens: watch,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None,
            sustained_cache_thrash: false,
            behavior: None,
            drift_score: None,
        }
    }

    #[test]
    fn vote_counter_zero_families_lean() {
        let r = family_vote_verdict(&lean_inputs(32_000));
        assert_eq!(r.count, 0);
        assert_eq!(r.tier, Tier::Lean);
        assert_eq!(r.verdict, Verdict::Ok);
        assert_eq!(r.verdict_gate, None);
        assert!(!r.decisive_override);
    }

    #[test]
    fn vote_counter_one_family_drift() {
        // Volume fires (1 family) → Drift → Nearing
        let inputs = FamilyVoteInputs {
            window_tokens: 40_000,
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None,
            sustained_cache_thrash: false,
            behavior: None,
            drift_score: None,
        };
        let r = family_vote_verdict(&inputs);
        assert_eq!(r.count, 1);
        assert_eq!(r.tier, Tier::Drift);
        assert_eq!(r.verdict, Verdict::Nearing);
        assert_eq!(r.verdict_gate, Some(VerdictGate::AbsoluteWatch));
    }

    #[test]
    fn vote_counter_two_families_bloated() {
        // Volume + Speed (2 families, no Behavior/Drift) → Bloated → Over
        let inputs = FamilyVoteInputs {
            window_tokens: 40_000,
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: Some(1),
            sustained_cache_thrash: false,
            behavior: None,
            drift_score: None,
        };
        let r = family_vote_verdict(&inputs);
        assert_eq!(r.count, 2);
        assert_eq!(r.tier, Tier::Bloated);
        assert_eq!(r.verdict, Verdict::Over);
        assert_eq!(r.verdict_gate, Some(VerdictGate::FamilyVote));
    }

    #[test]
    fn vote_counter_two_families_with_drift_stale() {
        // Volume + Speed (count=2 from 4-family count) + drift_score → Stale → Over
        // Drift excluded from count; fires only as tier-split within >=2 band.
        let inputs = FamilyVoteInputs {
            window_tokens: 40_000,
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: Some(PROJECTION_NEARING_TURNS),
            sustained_cache_thrash: false,
            behavior: None,
            drift_score: Some(DRIFT_SCORE_THRESHOLD + 0.1),
        };
        let r = family_vote_verdict(&inputs);
        assert_eq!(r.count, 2);
        assert_eq!(r.tier, Tier::Stale);
        assert_eq!(r.verdict, Verdict::Over);
    }

    #[test]
    fn vote_counter_two_families_with_behavior_critical() {
        // Volume + Behavior (2 families, Behavior fires, no decisive) → Critical → Over
        static BEHAVIOR: BehaviorSignals = BehaviorSignals {
            repetition_run: None,
            failure_streak: Some(BEHAVIOR_STREAK_THRESHOLD),
            ping_pong_count: None,
            stop_reason_max_tokens: false,
        };
        let inputs = FamilyVoteInputs {
            window_tokens: 40_000,
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None,
            sustained_cache_thrash: false,
            behavior: Some(&BEHAVIOR),
            drift_score: None,
        };
        let r = family_vote_verdict(&inputs);
        assert_eq!(r.count, 2);
        assert_eq!(r.tier, Tier::Critical);
        assert_eq!(r.verdict, Verdict::Over);
    }

    #[test]
    fn vote_counter_decisive_override_fires_alone() {
        // repetition_run >= threshold → decisive_override even with count=1
        static BEHAVIOR: BehaviorSignals = BehaviorSignals {
            repetition_run: Some(BEHAVIOR_REPETITION_THRESHOLD),
            failure_streak: None,
            ping_pong_count: None,
            stop_reason_max_tokens: false,
        };
        let inputs = FamilyVoteInputs {
            window_tokens: 1_000, // below watch and below backstop
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None,
            sustained_cache_thrash: false,
            behavior: Some(&BEHAVIOR),
            drift_score: None,
        };
        let r = family_vote_verdict(&inputs);
        assert!(r.decisive_override);
        assert_eq!(r.tier, Tier::Critical);
        assert_eq!(r.verdict, Verdict::Over);
        assert_eq!(r.verdict_gate, Some(VerdictGate::DecisiveOverride));
    }

    #[test]
    fn vote_counter_stop_reason_fires_alone() {
        // stop_reason_max_tokens → thrash fires (count=1) + decisive_override → Critical
        static BEHAVIOR: BehaviorSignals = BehaviorSignals {
            repetition_run: None,
            failure_streak: None,
            ping_pong_count: None,
            stop_reason_max_tokens: true,
        };
        let inputs = FamilyVoteInputs {
            window_tokens: 1_000, // below watch and below backstop
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None,
            sustained_cache_thrash: false,
            behavior: Some(&BEHAVIOR),
            drift_score: None,
        };
        let r = family_vote_verdict(&inputs);
        assert!(r.decisive_override);
        assert!(r.families[2], "thrash fires for stop_reason_max_tokens");
        assert_eq!(r.tier, Tier::Critical);
        assert_eq!(r.verdict, Verdict::Over);
    }

    #[test]
    fn vote_counter_backstop_decisive_override() {
        // window_tokens >= recycle_backstop, no behavior/trend → decisive_override → Critical → Over
        let inputs = FamilyVoteInputs {
            window_tokens: ABSOLUTE_RECYCLE_BACKSTOP,
            watch_tokens: ABSOLUTE_WATCH_TOKENS,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None,
            sustained_cache_thrash: false,
            behavior: None,
            drift_score: None,
        };
        let r = family_vote_verdict(&inputs);
        assert!(
            r.decisive_override,
            "backstop hit must fire decisive_override"
        );
        assert_eq!(r.tier, Tier::Critical);
        assert_eq!(r.verdict, Verdict::Over);
        assert_eq!(r.verdict_gate, Some(VerdictGate::DecisiveOverride));
    }

    #[test]
    fn vote_counter_drift_excluded_from_count() {
        // Volume fires (count=1) + drift_score positive → count stays 1, NOT Stale
        // Drift cannot escalate tier alone; it only refines within >=2 band.
        let inputs = FamilyVoteInputs {
            window_tokens: 40_000,
            watch_tokens: 32_000,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
            projected_turns: None, // speed doesn't fire
            sustained_cache_thrash: false,
            behavior: None,
            drift_score: Some(DRIFT_SCORE_THRESHOLD + 0.1),
        };
        let r = family_vote_verdict(&inputs);
        assert!(r.families[4], "drift family fires");
        assert_eq!(
            r.count, 1,
            "drift excluded from count; only 4 families count"
        );
        assert_eq!(r.tier, Tier::Drift); // count=1 → Drift tier, NOT Stale
        assert_eq!(r.verdict, Verdict::Nearing);
    }
}
