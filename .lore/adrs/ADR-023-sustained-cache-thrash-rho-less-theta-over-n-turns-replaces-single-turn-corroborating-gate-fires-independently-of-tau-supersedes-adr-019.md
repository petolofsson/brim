---
id: ADR-023
title: Sustained cache-thrash (rho < theta over N turns) replaces single-turn corroborating gate; fires independently of tau; supersedes ADR-019
status: Accepted
related_requirements: []
related_adrs: [ADR-008, ADR-019]
related_stories: []
related_tests: []
---

# ADR-023 - Sustained cache-thrash (rho < theta over N turns) replaces single-turn corroborating gate; fires independently of tau; supersedes ADR-019

## Context

ADR-019 accepted option (c) — keep `cache_hit_ratio` (ρ) as a corroborating,
`Nearing`-only gate suppressed when `τ = ∅` — because single-turn ρ is noisy:
legitimate low-ratio turns exist at cold-start and on genuine large-new-content
turns. ADR-019 explicitly named option (b) SUSTAINED-THRASH as the preferred path
**if and when a less-noisy signal becomes available**, deferring it pending that
signal.

The sustained signal — ρ < θ across N consecutive turns — is now that less-noisy
proxy. N consecutive below-threshold turns cannot be explained by a single
cold-start event or a single large-content turn; the pattern requires persistent
cache-prefix churn across multiple turns. That meets the bar ADR-019 set for
promotion from deferred to implemented.

ADR-008 introduced `cache_hit_ratio` and constrained it to a secondary,
non-driving role. This ADR supersedes the gate-wiring clause of ADR-019 (which
deferred (b)) while leaving ADR-008's core decision intact: ρ is computed per
turn, providers without a cache split yield `None`, and ρ does not become a
co-equal `Over` trigger.

## Decision

**Implement option (b): sustained-thrash detection — ρ < θ over N = 3 consecutive
recent turns.** The gate fires `Nearing` independently of `τ` (no
`projected_turns.is_some()` guard).

### N = 3 — rationale

- Filters a single cold-start turn and a single large-new-content dip (the
  one-off cases that made option (a) noisy). Two consecutive low turns remain
  borderline; three is clearly a pattern.
- Aligns with TREND_TAIL_K / 3 ≈ 2–3 convention (TREND_TAIL_K = 8,
  VELOCITY_WINDOW_W = 4).
- Not so large (e.g. 5–6) as to delay detection of real churn; TREND_TAIL_K = 8
  means 3 turns of history are available early in a session.
- Constant: `SUSTAINED_THRASH_N = 3` in `src/window.rs`.

### tau-independence

The old gate guard `projected_turns.is_some() && ρ < θ` went silent exactly when
there was no velocity trend — typically early-session and post-compaction, the
moments when cache thrash is most diagnostically useful. The N-turn persistence
requirement is itself the cold-start guard: a session with fewer than N turns
cannot satisfy it, and a single-event session produces at most 1 point with a
cache ratio (correctly returning false). No additional `τ` gate is needed.

### Severity: Nearing-only

Token economics ≠ reasoning quality (ADR-020 scope boundary A). Sustained cache
churn is a degradation proxy, not a breach guarantee. The absolute growth gates
(ADR-010 / ADR-011) remain the load-bearing primary signal and are checked first
in the OR gate; a high-token or short-projection session will hit AbsoluteWatch,
AbsoluteBackstop, or ProjectionRecycle before CacheThrash would fire. Keeping
CacheThrash at `Nearing` therefore has no practical cost in practice while
avoiding false Over verdicts on sessions with degraded cache efficiency but
comfortable headroom.

### Implementation

- `src/window.rs`: `pub fn sustained_cache_thrash(points: &[TimelinePoint]) -> bool`
  checks `points[len-N..].all(|p| p.cache_hit_ratio < CACHE_THRASH_THRESHOLD)`.
  Returns false if fewer than N points exist or any point has `None` ratio
  (non-cache providers).
- `src/verdict.rs`: `absolute_verdict` third parameter changed from
  `cache_hit_ratio: Option<f32>` to `sustained_cache_thrash: bool`. The gate
  becomes `if sustained_cache_thrash { return (Verdict::Nearing, CacheThrash) }`.
  The "deferred to ADR-008 follow-up" comment is removed.
- Call sites (`model.rs`, `output.rs`): compute
  `trend.is_some_and(|t| sustained_cache_thrash(&t.points))` before calling
  `absolute_verdict`.
- `CACHE_THRASH_THRESHOLD` promoted to `pub const` so `window.rs` can reference it
  without duplication.

## Consequences

- The efficiency signal is now a genuine (if conservative) escalation input,
  catching sustained cache-prefix churn without the cold-start false positives of
  option (a).
- Sessions shorter than N turns are still silent for CacheThrash — acceptable;
  a 1- or 2-turn session is not meaningfully diagnosable for cache efficiency.
- Non-cache providers (ρ = None) never fire CacheThrash — correct; they have no
  cache split to thrash.
- Absolute growth gates (ADR-010 / ADR-011) remain the load-bearing primary signal
  and are checked first; this change does not affect their logic.
- `WindowTrend.points` and `TimelinePoint.cache_hit_ratio` are now consumed by the
  verdict path; `#[allow(dead_code)]` annotations removed from those fields.

## Alternatives Considered

- **(a) Drop `projected_turns.is_some()` guard** — closes cold-start silence but
  fires on any single low-ratio turn; the exact false positives ADR-019 rejected.
- **(c) Keep deferred / no change** — accepted in ADR-019; superseded now that the
  N-turn sustained signal is available and implemented.
- **N = 2** — borderline; a two-turn startup or a tool-response pair can produce
  two consecutive low turns legitimately (tool-output turns tend to have low cache
  hit since the output is new content). N = 3 is more conservative.
- **N = 4 (= VELOCITY_WINDOW_W)** — would align with the velocity window but
  delays detection by one turn relative to N = 3 with no clear benefit.
- **Promote CacheThrash to Over** — rejected; ρ proxy is too noisy and token
  economics ≠ reasoning quality (ADR-020 scope-boundary A). Growth gates decide
  breach; CacheThrash corroborates.
