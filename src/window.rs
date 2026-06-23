use crate::model::{WindowInfo, WindowSource};

/// Resolve context-window limit from model string.
/// Model ids containing "[1m]" map to 1_000_000; all others default to 200_000.
/// opencode model ids (`z-ai/glm-5.2`, etc.) carry no marker and fall through to the default.
pub fn window_limit(model: &str) -> u64 {
    if model.contains("[1m]") {
        1_000_000
    } else {
        200_000
    }
}

/// Compute window fill from a usage record (point-in-time or aggregate).
/// fill = round(window_tokens / limit * 100), bounded to [0, 100].
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
    let limit = window_limit(model);
    // Integer round-half-up: (numerator + limit/2) / limit
    let pct = window_tokens
        .saturating_mul(100)
        .saturating_add(limit / 2)
        .saturating_div(limit)
        .min(100);
    WindowInfo {
        window_tokens,
        fill_percent: pct as u8, // safe: bounded to [0, 100] above
        model: model.to_string(),
        context_limit: limit,
        window_source: source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_tokens_no_overflow_huge_values() {
        let info = compute_window_info(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.fill_percent, 100);
    }

    #[test]
    fn test_window_fill_math_oracle() {
        let info = compute_window_info(
            7_000,
            130_000,
            5_000,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.window_tokens, 142_000);
        assert_eq!(info.fill_percent, 71);
    }

    #[test]
    fn test_window_fill_bounded_at_100() {
        let info = compute_window_info(
            200_000,
            100_000,
            50_000,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.fill_percent, 100);
    }

    #[test]
    fn test_window_limit_1m_model() {
        let info =
            compute_window_info(500_000, 0, 0, "claude-opus-4-8[1m]", WindowSource::LastTurn);
        assert_eq!(info.fill_percent, 50);
    }

    #[test]
    fn test_context_limit_stored() {
        let info = compute_window_info(100_000, 0, 0, "claude-sonnet-4-6", WindowSource::LastTurn);
        assert_eq!(info.context_limit, 200_000);
    }

    #[test]
    fn test_glm_default_200k() {
        // z-ai/glm-5.2 carries no [1m] marker → falls through to the 200_000 default (ADR-005).
        let info = compute_window_info(100_000, 0, 0, "z-ai/glm-5.2", WindowSource::LastTurn);
        assert_eq!(info.context_limit, 200_000);
        assert_eq!(info.fill_percent, 50);
    }

    #[test]
    fn test_source_propagates() {
        let agg = compute_window_info(1_000, 0, 0, "z-ai/glm-5.2", WindowSource::Aggregate);
        assert_eq!(agg.window_source, WindowSource::Aggregate);
        let lt = compute_window_info(1_000, 0, 0, "z-ai/glm-5.2", WindowSource::LastTurn);
        assert_eq!(lt.window_source, WindowSource::LastTurn);
    }
}
