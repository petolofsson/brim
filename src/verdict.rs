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

/// Which ADR-010 OR-gate fired (highest-severity gate wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictGate {
    /// (c) absolute active tokens ≥ recycle_backstop
    AbsoluteBackstop,
    /// (b) projected turns to recycle ≤ PROJECTION_RECYCLE_TURNS
    ProjectionRecycle,
    /// (c) absolute active tokens ≥ watch_tokens
    AbsoluteWatch,
    /// (b) projected turns to recycle ≤ PROJECTION_NEARING_TURNS
    ProjectionNearing,
    /// (d) sustained cache_hit_ratio < CACHE_THRASH_THRESHOLD over N consecutive turns (Nearing only, ADR-008/new ADR)
    CacheThrash,
}

impl VerdictGate {
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictGate::AbsoluteBackstop => "abs-backstop",
            VerdictGate::ProjectionRecycle => "projection_recycle",
            VerdictGate::AbsoluteWatch => "abs-watch",
            VerdictGate::ProjectionNearing => "projection",
            VerdictGate::CacheThrash => "cache-thrash",
        }
    }

    pub fn as_json_str(self) -> &'static str {
        match self {
            VerdictGate::AbsoluteBackstop => "absolute_backstop",
            VerdictGate::ProjectionRecycle => "projection_recycle",
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
/// Fraction below which cache-read utilisation signals context thrash (ADR-008).
/// Exposed pub so window::sustained_cache_thrash can use the shared constant.
pub const CACHE_THRASH_THRESHOLD: f32 = 0.20;

/// Quality verdict: OR of ADR-010 signals (b), (c), (d).
/// Signal (a) behavioral degradation is deferred per ADR-010.
/// `watch_tokens` and `recycle_backstop` are runtime-configurable (REQ-004, ADR-010 caveat #1).
/// `sustained_cache_thrash`: pre-computed by window::sustained_cache_thrash; fires independently of tau.
/// Returns the verdict and the highest-severity gate that fired.
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
}
