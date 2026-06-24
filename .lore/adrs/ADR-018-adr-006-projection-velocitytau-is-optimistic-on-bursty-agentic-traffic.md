---
id: ADR-018
title: ADR-006 projection (velocity/tau) is optimistic on bursty agentic traffic
status: Draft
related_requirements: []
related_adrs:
  - ADR-006
  - ADR-010
  - ADR-011
related_stories: []
related_tests: []
---

# ADR-018 - ADR-006 projection (velocity/tau) is optimistic on bursty agentic traffic

## Context

**Advisory, not control.** brim's verdict is an **advisory pre-warning** that sits
*before* the host's hard auto-compaction — Claude Code's ~95%-window compaction
(named in ADR-011), which is the real safety net. A missed projective warning is
**lost graceful lead time, NOT a breach**: the user gets the abrupt host recycle
instead of the graceful one. Because the product *is* the lead time, the optimistic
estimator is the **central failure mode** for this ADR — the user reads "plenty of
room," keeps going, and hits the host's hard threshold mid-task, which is exactly
what the advisory exists to prevent. Severities below are scored against that
advisory role, not against a breach model.

brim's primary workload is **calm punctuated by bursts** — agentic coding, where
a quiet stretch of small turns is interrupted by one large edit/tool result. The
projection gate (ADR-006, re-targeted to the absolute backstop by ADR-011) keys
off `velocity` = the **upper median of past positive window-token deltas** over a
bounded last-K tail (`src/window.rs` `compute_trend`: `pos_deltas[len/2]`), and
projects `tau = (backstop − current) / velocity`. The ADR-010 OR-gate maps
`tau <= 2 -> Over`, `tau <= 5 -> Nearing`.

A third-party adversarial reimplementation of the verdict model (faithful re-spec
exercised against hand-built traces plus **250k random fuzz traces**;
`docs/recycle-verdict-model.md`) confirms the absolute gates are **sound against
breach** — no fuzzed trace with `w_n >= backstop` ever escaped `Over`, and the
cold-start `tau = None` guard held. But it reproduces two failure modes in the
projection layer, in exactly the traffic shape the projection was built for:

1. **STRUCTURAL (high).** The only sub-backstop path to projective `Over` is
   `tau <= 2`, which **requires positive past velocity**. A quiet agent has
   small/zero past deltas, so **no future spike — however large — can trip
   projective `Over`**; it is structurally invisible. Exposure: to project `Over`
   you need a *prior* median delta of `>= ~22,667` at `w_n = 60k`
   (`>= ~9,334` at 100k; `>= ~2,667` at 120k). Below those, the absolute backstop
   is the sole defense and projection contributes nothing.

2. **ESTIMATOR (high).** Upper-median **undershoots** on spiky deltas: it selects a
   small delta whenever small deltas outnumber large ones. Trace: seven `+100`
   turns then one `+79,400` turn lands at `w_n = 120,000` (8k from a 128k
   backstop) yet reports `v = 100, tau = 80` — `Nearing` with **80 turns of false
   runway**. The median was chosen to resist a single anomalous large turn, but in
   a bursty workload the large turn *is the signal*, not the outlier — so the
   projection radiates reassurance right up to the moment the absolute gate trips.

3. **Consequence — "silent band" (LOW under the advisory framing).** Any
   `w_n in [32k, 128k)` with a flat-or-shrinking recent window yields
   `velocity = None`, capping the verdict at `Nearing` across a **96k-token span**.
   The model cannot distinguish "miles of runway" from "one large edit from breach"
   anywhere in that band. This looked medium-high under a control framing, but as an
   advisory it is **low**: the downstream host hard threshold catches what the
   advisory misses here, so the only cost is a less-graceful recycle, not a breach.

The absolute gates (ADR-010 watch ~32k / backstop ~128k) remain the correct,
load-bearing safety net. **Projection is the issue**, and the median-of-positive-
deltas is optimistic in both failure modes — the wrong direction for an advisory
estimator whose entire value is lead time (it should err pessimistic / earlier).

## Decision

*(Proposed, NOT finalized — decision pending; this ADR stays Draft until a human
chooses an option.)*

Keep the **OR-gate + absolute gates as the load-bearing safety net, unchanged**
(verified sound by fuzzing). For an advisory, **direction matters more than
accuracy — it should err pessimistic** so it warns early on bursts (the correct,
harmless direction) rather than radiating false runway. Lead recommendation:

- **(a) LEAD — replace upper-median with a HIGH-QUANTILE of recent positive
  deltas (e.g. p90), NOT raw `max`.** Pessimistic estimator → earlier warning;
  directly fixes failure mode #2 and shrinks the silent band. A high quantile
  catches the burst signal while still discarding a single freak spike; **raw
  `max` over-triggers** on one large edit and is rejected as the estimator. Does
  not fix #1 on its own (still needs *some* positive past delta), which is why it
  pairs with (b).
- **(b) COMPLEMENT — single-large-delta headroom guard.** If
  `(backstop − w_n) <= largest recent positive delta`, escalate. Directly targets
  the calm→spike structural blind spot (#1): one observed large turn is enough to
  flag that the next one could breach, with no dependence on the quantile estimator.
  Trade-off: a new gate term to specify/test; tune which window the "recent" delta
  is drawn from (interacts with finding #5, tail truncation).
- **(c) Accept as a documented limitation, no code change.** Projection stays
  advisory; host hard threshold remains the backstop. Cheapest; leaves the silent
  band and false-runway behavior in place. No longer the lead — under the advisory
  framing, leaving the estimator optimistic forfeits the lead time that *is* the
  product.

**Recommendation (open):** **(a) high-quantile estimator, complemented by (b)
headroom guard.** Direction matters more than accuracy; an advisory should err
pessimistic, and (a)+(b) together move both failure modes that direction without
the over-triggering of raw `max`. The choice is left **OPEN for human decision**;
this ADR remains Draft.

## Consequences

- If (a) or (b) is accepted: projective early-warning becomes useful on bursty
  traffic (ADR-006's stated purpose), at the cost of more frequent advisory
  warnings near the backstop. Absolute-gate behavior is unchanged either way.
- If (c) is accepted: the limitation is documented; consumers should treat the
  `Nearing` verdict and `projected_turns_to_recycle` as advisory, not as runway
  guarantees, and rely on the absolute backstop for breach safety.
- Either way, the fuzz-verified soundness of the absolute gates is preserved — no
  proposed option weakens the breach guarantee.

## Supersession

This ADR does **NOT** edit the Accepted ADR-006, ADR-010, or ADR-011 (recorded
here per the never-edit-an-Accepted-ADR rule). **If and when Accepted**, it
supersedes only **ADR-006's velocity/projection clause** (the
median-of-positive-deltas estimator and its projective contribution) per whichever
option is chosen. The ADR-006 bounded-tail read, negative-delta reset detection,
and `None`-with-<2-points degradation are otherwise intact, as is ADR-011's
re-targeting of the projection to the absolute backstop and the ADR-010 absolute
OR-gate.

## Alternatives Considered

- **(a) High-quantile (p90) of recent positive deltas** — pessimistic estimator,
  earlier warning; lead recommendation. Raw `max` rejected (over-triggers on one
  large edit). See Decision.
- **(b) Single-large-delta headroom guard** — escalate when remaining headroom
  `<=` largest recent positive delta; complements (a). See Decision.
- **(c) Accept as documented limitation, no code change** — projection advisory
  only; no longer the lead under the advisory framing. See Decision.

### Statistical critique (third perspective) — rejected

A third critique (`mathematical_critique.md`, statistical lens) independently
converges on "projection is the weak point — make it risk-aware," but its proposed
machinery is rejected given `n <= 8` and the determinism requirement (CLAUDE.md
goal #2):

- **Theil–Sen slope estimator** — REJECTED: it is a median-of-pairwise-slopes, the
  same robust-median family as the current upper-median, so it **reproduces** the
  burst-optimism flaw this ADR exists to fix rather than curing it. We want the
  opposite: a pessimistic p90 (option a).
- **Mann–Kendall trend test / bootstrap confidence intervals** — REJECTED: require
  a sample size brim does not have (K=8 cap; the post-reset segment is often 2–3
  points). No statistical power / unstable variance at `n <= 3`.
- **Normal-error projection `w_{n+1} ~ N(v_hat, σ²)` with credible interval for τ**
  — REJECTED parametric form: the per-turn delta distribution is heavy-tailed /
  bursty, so a Normal assumption fails exactly at the spikes that matter; a
  bootstrap form also conflicts with deterministic execution. The useful **intent**
  (express τ as risk of imminent breach) is retained and met **deterministically**
  by the headroom guard (option b).
- **Probabilistic verdict thresholds `P(w_{n+1} >= B) > 0.95`** — REJECTED: trades
  the transparent, deterministic, load-bearing absolute OR-gate (affirmed correct by
  both prior critiques) for a probability built on an unestimable ~3-point variance;
  wrong trade for an explainable CLI advisory.

Takeaway: this critique reaches the same diagnosis as the fuzzing critique, but the
right realization given `n <= 8` + determinism is the **headroom guard +
pessimistic quantile already proposed here (a)+(b)**, NOT Bayesian/bootstrap
machinery.

## References

- `docs/recycle-verdict-model.md` — source critique (250k fuzz traces;
  findings #1 structural, #2 estimator, #3 silent band).
- `src/window.rs` `compute_trend` — velocity = `pos_deltas[len/2]` (upper median),
  `tau = (backstop − current) / velocity`.
- ADR-006 (velocity/projection mechanism), ADR-010 (absolute OR-gate), ADR-011
  (projection re-targeted to absolute backstop).
