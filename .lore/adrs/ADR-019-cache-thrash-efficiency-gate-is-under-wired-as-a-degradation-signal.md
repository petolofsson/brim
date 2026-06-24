---
id: ADR-019
title: Cache-thrash efficiency gate is under-wired as a degradation signal
status: Accepted
related_requirements: []
related_adrs:
  - ADR-008
  - ADR-006
  - ADR-011
  - ADR-018
related_stories: []
related_tests: []
---

# ADR-019 - Cache-thrash efficiency gate is under-wired as a degradation signal

## Context

ADR-008 exposed `cache_hit_ratio = cache_read_input_tokens / window_tokens` as a
**secondary** context-thrash signal — a falling cache-read fraction means the
cached prefix is churning. In the verdict (`src/verdict.rs` `absolute_verdict`,
gate `VerdictGate::CacheThrash`) the `ρ < CACHE_THRASH_THRESHOLD` gate is wired
weakly:

- it can only ever reach **`Nearing`**, never `Over`; and
- it is **suppressed whenever `τ = ∅`** (cold-start / no trend) — the gate guard
  is `projected_turns.is_some() && cache_hit_ratio < θ`, so with no projection it
  goes silent. The code already flags this: *"Full trended falling-fraction signal
  deferred to ADR-008 follow-up."*

The recycle-verdict critique addendum (`docs/recycle-verdict-model.md`) **upgrades
this to a MEDIUM finding once inefficient-token-usage is treated as a *primary*
degradation signal** rather than a minor corroborating guard. As wired, the
efficiency signal is a junior partner to growth: it cannot escalate on its own and
goes silent exactly when there is no trend to lean on.

**The open question is not only how to wire it — it is whether `cache_hit_ratio`
is a strong enough proxy to escalate on independently.** A low ratio also occurs
**legitimately**: early-session / cold-start, or a genuine large new-content turn.
And addendum scope-boundary A is explicit that this measures **token economics,
not reasoning quality** — a window full of stale tool outputs can degrade reasoning
while showing a healthy ratio, and a low ratio can show on a perfectly healthy
turn. `ρ` is therefore a **noisy proxy**; promoting it to an independent escalation
trigger imports that noise into the verdict.

## Decision

**Accepted outcome — (c) keep current / DEFER.** `cache_hit_ratio` (`ρ`) stays a
**corroborating, `Nearing`-only signal**, wired exactly as ADR-008 left it: it does
**NOT** fire independently of `τ`, and it is **NOT** a co-equal `Over` trigger. The
proxy is too noisy to carry breach-grade escalation alone — legitimate low-ratio
turns exist (cold-start, large new-content) and token economics ≠ reasoning quality
(addendum scope-boundary A). Growth (ADR-010 / ADR-011 / ADR-018) remains the
load-bearing primary signal.

**Strengthening is DEFERRED**, not rejected: option (b) sustained-thrash (`ρ < θ`
over N consecutive turns — a trended falling-fraction) is the preferred path **if
and when** a less-noisy signal is available. Until then no code change is made; the
"deferred to ADR-008 follow-up" note in `absolute_verdict` remains accurate.

The options weighed before this decision:

- **(a) Let `ρ` fire independently of `τ`.** Drop the `projected_turns.is_some()`
  guard so a low ratio escalates even with no trend (closes the cold-start silence).
  Trade-off: maximally noisy — fires on legitimate cold-start and large-new-content
  turns, the exact false positives ADR-008 chose `ρ` as secondary to avoid.
- **(b) LEAN — escalate only on SUSTAINED thrash across multiple turns.** Require
  `ρ < θ` over N consecutive turns (a trended falling-fraction, the deferred
  ADR-008 follow-up) before escalating. Filters the one-off legitimate dips
  (cold-start, single large input) that make (a) noisy while still catching real
  churn. Trade-off: a new trended term to specify/test; needs ≥N turns of history,
  so it is itself quiet at session start.
- **(c) Keep as a corroborating `Nearing`-only signal / DEFER.** Leave the gate as
  ADR-008 wired it; accept that the efficiency signal cannot escalate alone. An
  **acceptable outcome** given the proxy's weakness — a noisy proxy is a poor sole
  basis for escalation, and growth remains the load-bearing primary signal.

**Decision: (c) defer/keep-current** — accepted as above. `ρ` does **NOT** become a
co-equal `Over` trigger and does **NOT** fire independently of `τ`; the proxy is too
noisy (legitimate low-ratio turns exist, and token economics ≠ reasoning quality) to
carry breach-grade escalation on its own. Strengthening to (b) sustained-thrash is
deferred pending a less-noisy signal.

## Consequences

- If (b): the efficiency signal becomes a genuine (if conservative) escalation
  input, catching sustained churn without the cold-start false positives of (a).
  Requires implementing the trended falling-fraction the code currently defers.
- If (a): cold-start silence closes but the verdict inherits the proxy's noise;
  likely needs a higher `θ` or hysteresis to stay usable.
- If (c): no code change; the documented limitation stands — `ρ` corroborates,
  growth (ADR-006 / ADR-018) decides. The deferral note in `absolute_verdict`
  remains accurate.
- In every option the absolute growth gates (ADR-010 / ADR-011) remain the
  load-bearing signal; `ρ` is at most an early corroborator, never the breach
  guarantee.

## Supersession

This ADR does **NOT** edit the Accepted ADR-008 (recorded here per the
never-edit-an-Accepted-ADR rule). **If and when Accepted**, it supersedes only
**ADR-008's gate-wiring clause** — the "secondary, `Nearing`-only, suppressed when
`τ = ∅`" wiring of `cache_hit_ratio` — per whichever option is chosen. ADR-008's
core decision (expose `cache_hit_ratio` per turn, `None` for non-cache providers,
content-free) is otherwise intact.

## Alternatives Considered

- **(a) `ρ` fires independently of `τ`** — closes cold-start silence; maximally
  noisy. See Decision.
- **(b) Escalate only on sustained thrash across multiple turns** — trended
  falling-fraction; lead recommendation, less noisy. See Decision.
- **(c) Keep corroborating `Nearing`-only / defer** — acceptable given proxy
  weakness; no code change. See Decision.
- **Promote `ρ` to a co-equal `Over` trigger** — rejected: the proxy is too noisy
  (legitimate cold-start / large-input low ratios; token economics ≠ reasoning
  quality, addendum scope-boundary A) to carry breach-grade escalation alone.

### Statistical critique (third perspective) — rejected

- **Beta-binomial Bayesian cache-hit ratio** (model `r_n ~ Binomial(w_n, p)`,
  posterior mean `(α + r_n) / (α + β + w_n)` plus a credible interval;
  `mathematical_critique.md`) — REJECTED: the generative model is mis-specified —
  `cache_read` / `cache_create` / `input` is a **structural token partition** from
  prompt-prefix matching, not `w_n` independent Bernoulli trials. At
  `w_n ~ tens of thousands` the posterior collapses to a razor-thin interval, so the
  surfaced "uncertainty" is fictional. The only real use (cold-start smoothing) is
  already handled by returning `None` when `window_tokens == 0` or there is no cache
  split.

## References

- `docs/recycle-verdict-model.md` — addendum upgrading this to a medium finding;
  scope-boundary A (token economics ≠ reasoning quality).
- `src/verdict.rs` `absolute_verdict` — `VerdictGate::CacheThrash` gate
  (`projected_turns.is_some() && cache_hit_ratio < CACHE_THRASH_THRESHOLD`,
  `Nearing` only); the "deferred to ADR-008 follow-up" note.
- ADR-008 (cache-hit ratio as a secondary thrash signal), ADR-006 / ADR-018
  (growth/projection primary signal), ADR-011 (absolute-token reasoning).
