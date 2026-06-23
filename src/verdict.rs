#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Which ADR-010 OR-gate fired (highest-severity gate wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictGate {
    /// (c) absolute active tokens ≥ recycle_backstop
    AbsoluteBackstop,
    /// (b) projected turns to overbound ≤ PROJECTION_RECYCLE_TURNS
    ProjectionOverbound,
    /// (c) absolute active tokens ≥ watch_tokens
    AbsoluteWatch,
    /// (b) projected turns to overbound ≤ PROJECTION_NEARING_TURNS
    ProjectionNearing,
    /// (d) cache_hit_ratio < CACHE_THRASH_THRESHOLD (Nearing only, ADR-008)
    CacheThrash,
}

impl VerdictGate {
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictGate::AbsoluteBackstop => "abs-backstop",
            VerdictGate::ProjectionOverbound => "projection",
            VerdictGate::AbsoluteWatch => "abs-watch",
            VerdictGate::ProjectionNearing => "projection",
            VerdictGate::CacheThrash => "cache-thrash",
        }
    }

    pub fn as_json_str(self) -> &'static str {
        match self {
            VerdictGate::AbsoluteBackstop => "absolute_backstop",
            VerdictGate::ProjectionOverbound => "projection_overbound",
            VerdictGate::AbsoluteWatch => "absolute_watch",
            VerdictGate::ProjectionNearing => "projection_nearing",
            VerdictGate::CacheThrash => "cache_thrash",
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

const PROJECTION_NEARING_TURNS: u32 = 5;
const PROJECTION_RECYCLE_TURNS: u32 = 2;
const CACHE_THRASH_THRESHOLD: f32 = 0.20;

/// Quality verdict: OR of ADR-010 signals (b), (c), (d).
/// Signal (a) behavioral degradation is deferred per ADR-010.
/// `watch_tokens` and `recycle_backstop` are runtime-configurable (REQ-004, ADR-010 caveat #1).
/// Returns the verdict and the highest-severity gate that fired.
pub fn absolute_verdict(
    window_tokens: u64,
    projected_turns: Option<u32>,
    cache_hit_ratio: Option<f32>,
    watch_tokens: u64,
    recycle_backstop: u64,
) -> (Verdict, Option<VerdictGate>) {
    // Over gates (highest severity first)
    if window_tokens >= recycle_backstop {
        return (Verdict::Over, Some(VerdictGate::AbsoluteBackstop));
    }
    if projected_turns.is_some_and(|t| t <= PROJECTION_RECYCLE_TURNS) {
        return (Verdict::Over, Some(VerdictGate::ProjectionOverbound));
    }
    // Nearing gates
    if window_tokens >= watch_tokens {
        return (Verdict::Nearing, Some(VerdictGate::AbsoluteWatch));
    }
    if projected_turns.is_some_and(|t| t <= PROJECTION_NEARING_TURNS) {
        return (Verdict::Nearing, Some(VerdictGate::ProjectionNearing));
    }
    // Cache-thrash: ADR-008 corroborating signal (Nearing only).
    // Guard: suppress at cold start / single-turn (no trend = projected_turns is None).
    // Full trended falling-fraction signal deferred to ADR-008 follow-up.
    if projected_turns.is_some() && cache_hit_ratio.is_some_and(|r| r < CACHE_THRASH_THRESHOLD) {
        return (Verdict::Nearing, Some(VerdictGate::CacheThrash));
    }
    (Verdict::Ok, None)
}

/// Capacity-runway thresholds: maps fill % to distance from auto-compaction (~95% of
/// advertised window). DEMOTED from quality-verdict driver (ADR-010) — used only for
/// the capacity-runway readout in output; the quality verdict uses `absolute_verdict`.
/// Also carries the runtime-configurable absolute bands (REQ-004, ADR-010 caveat #1).
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub nearing: u8,
    pub ceiling: u8,
    /// Absolute active-token watch band (default: ABSOLUTE_WATCH_TOKENS per ADR-010 research).
    pub watch_tokens: u64,
    /// Absolute recycle backstop (default: ABSOLUTE_RECYCLE_BACKSTOP per ADR-010 research).
    pub recycle_backstop: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            nearing: 70,
            ceiling: 90,
            watch_tokens: ABSOLUTE_WATCH_TOKENS,
            recycle_backstop: ABSOLUTE_RECYCLE_BACKSTOP,
        }
    }
}

impl Thresholds {
    /// Capacity-runway readout: distance to forced auto-compaction (~95% of advertised window).
    pub fn runway(self, fill_percent: u8) -> Verdict {
        if fill_percent >= self.ceiling {
            Verdict::Over
        } else if fill_percent >= self.nearing {
            Verdict::Nearing
        } else {
            Verdict::Ok
        }
    }

    /// Capacity-runway vocab for JSON output — distinct from Verdict strings (N2).
    pub fn runway_capacity_str(self, fill_percent: u8) -> &'static str {
        if fill_percent >= self.ceiling {
            "at_compaction"
        } else if fill_percent >= self.nearing {
            "nearing_compaction"
        } else {
            "ample"
        }
    }
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::AbsoluteBackstop));
    }

    #[test]
    fn absolute_verdict_over_projection_overbound() {
        let (v, gate) = absolute_verdict(
            20_000,
            Some(1),
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::ProjectionOverbound));
    }

    #[test]
    fn absolute_verdict_over_projection_at_boundary() {
        let (v, gate) = absolute_verdict(
            20_000,
            Some(PROJECTION_RECYCLE_TURNS),
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::ProjectionOverbound));
    }

    #[test]
    fn absolute_verdict_nearing_projection_nearing() {
        let (v, gate) = absolute_verdict(
            10_000,
            Some(PROJECTION_NEARING_TURNS),
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::ProjectionNearing));
    }

    #[test]
    fn absolute_verdict_nearing_cache_thrash() {
        // projected_turns Some (trend present) required for cache-thrash to fire (N1 cold-start guard).
        let (v, gate) = absolute_verdict(
            10_000,
            Some(10),
            Some(0.10),
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::CacheThrash));
    }

    #[test]
    fn absolute_verdict_ok_cache_above_threshold() {
        let (v, gate) = absolute_verdict(
            10_000,
            None,
            Some(0.50),
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
            None,
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
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Over);
        assert_eq!(gate, Some(VerdictGate::ProjectionOverbound));
    }

    // N3: boundary tests

    #[test]
    fn absolute_verdict_ok_just_below_watch() {
        // watch_tokens - 1 = 31_999 → must be Ok (watch boundary is inclusive >=)
        let (v, gate) = absolute_verdict(
            ABSOLUTE_WATCH_TOKENS - 1,
            None,
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Ok);
        assert_eq!(gate, None);
    }

    #[test]
    fn absolute_verdict_cache_thrash_at_exact_threshold() {
        // ratio == CACHE_THRASH_THRESHOLD → boundary is strict (<), must be Ok
        let (v, gate) = absolute_verdict(
            10_000,
            Some(10),
            Some(CACHE_THRASH_THRESHOLD),
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
            None,
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Nearing);
        assert_eq!(gate, Some(VerdictGate::ProjectionNearing));
    }

    #[test]
    fn absolute_verdict_cache_thrash_suppressed_no_trend() {
        // projected_turns None (cold start / no trend) → cache-thrash suppressed (N1 guard)
        let (v, gate) = absolute_verdict(
            10_000,
            None,
            Some(0.10),
            ABSOLUTE_WATCH_TOKENS,
            ABSOLUTE_RECYCLE_BACKSTOP,
        );
        assert_eq!(v, Verdict::Ok);
        assert_eq!(gate, None);
    }

    // Runway tests (demoted %-based capacity readout, ADR-010)

    #[test]
    fn runway_ok() {
        let t = Thresholds::default();
        assert_eq!(t.runway(0), Verdict::Ok);
        assert_eq!(t.runway(69), Verdict::Ok);
    }

    #[test]
    fn runway_nearing() {
        let t = Thresholds::default();
        assert_eq!(t.runway(70), Verdict::Nearing);
        assert_eq!(t.runway(89), Verdict::Nearing);
    }

    #[test]
    fn runway_over() {
        let t = Thresholds::default();
        assert_eq!(t.runway(90), Verdict::Over);
        assert_eq!(t.runway(100), Verdict::Over);
    }

    // N2: capacity_runway vocab
    #[test]
    fn runway_capacity_str_vocab() {
        let t = Thresholds::default();
        assert_eq!(t.runway_capacity_str(0), "ample");
        assert_eq!(t.runway_capacity_str(69), "ample");
        assert_eq!(t.runway_capacity_str(70), "nearing_compaction");
        assert_eq!(t.runway_capacity_str(89), "nearing_compaction");
        assert_eq!(t.runway_capacity_str(90), "at_compaction");
        assert_eq!(t.runway_capacity_str(100), "at_compaction");
    }
}
