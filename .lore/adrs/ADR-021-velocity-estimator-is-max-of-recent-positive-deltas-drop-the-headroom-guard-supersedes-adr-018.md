---
id: ADR-021
title: Velocity estimator is max of recent positive deltas; drop the headroom guard (supersedes ADR-018)
status: Superseded
related_requirements: []
related_adrs:
  - ADR-018
  - ADR-006
  - ADR-010
related_stories: []
related_tests: []
---

# ADR-021 - Velocity estimator is max of recent positive deltas; drop the headroom guard (supersedes ADR-018)

## Context

ADR-018 (now Superseded) chose, for the projection velocity, a **high-quantile
(p90) of recent positive deltas — explicitly NOT raw `max`** — complemented by a
**single-large-delta headroom guard** (`(backstop − w_n) <= largest recent
positive delta` → escalate). Both choices were made to keep an advisory estimator
pessimistic (warn early on bursts) without the perceived over-triggering of raw
`max`.

Implementation and review of that design exposed two facts that make it
unrealizable in brim's data regime:

1. **p90 ≡ max at K=8.** Velocity is computed over a bounded post-reset tail of at
   most `TREND_TAIL_K = 8` window snapshots, so the positive-delta vector has
   length `m <= 7` (`<= 8` only in degenerate constructions). A nearest-rank p90
   selects index `ceil(0.9·m) − 1`, which for every realizable trace (`len <= 9`,
   `index = len − 1`) collapses to the **last order statistic = raw MAX**. The
   "p90, not max" distinction ADR-018 relied on is **unachievable** at this tail
   length — there is no trace in brim's regime where p90 and max differ.

2. **The headroom gate is dead code.** With velocity `== max`, whenever the
   headroom condition `(B − w_n) <= max_recent_delta` holds, projection
   `tau = (B − w_n) \dot\div v <= 1 <= tau_R`, so the projection-recycle gate
   already fires **Over**. The headroom gate could at best raise **Nearing**, a
   strictly lower severity, so it is **always preempted** by projection_recycle
   and can never change a verdict. It is dead by construction.

## Decision

Adopt **velocity = MAX of recent post-reset positive deltas** as the deliberate,
pessimistic estimator. This is not a fallback from p90 — at K=8 it *is* what p90
denotes, and it is the correct direction for an advisory whose entire value is
lead time: it should **err early on bursts**.

**Remove the headroom gate and its `max_recent_delta` field** as redundant (dead
by construction per Context #2). The OR-gate returns to **5 gates**:
`w_n >= B → Over`; `tau <= tau_R → Over`; `w_n >= T_w → Nearing`;
`tau <= tau_N → Nearing`; `rho_n < theta (with tau present) → Nearing`.

The **absolute gates (watch 32k / backstop 128k) are unchanged** — the
fuzz-verified load-bearing safety net is untouched.

Code: `src/window.rs` `compute_trend` (velocity = max of `D^+`);
`src/verdict.rs` `absolute_verdict` (5 gates, no headroom term).

## Consequences

- Bursts trigger **Over early** — the desired, harmless direction for an advisory
  that sits *before* the host's hard auto-compaction net (ADR-011).
- Trade-off: **sticky over-warning** for up to `K = 8` turns after an isolated
  large delta — that delta stays in the tail window and keeps velocity high until
  it scrolls off. Accepted: an advisory over-warning costs lead-time politeness,
  not safety, and the host's hard threshold remains the real backstop.
- One fewer gate term and one fewer field to specify, compute, and test;
  projection logic is simpler and the spec matches the code's actual behavior.
- Absolute-gate breach guarantee preserved (no projection change can weaken it).

## Supersession

This ADR **supersedes ADR-018** in full — both its (a) p90 estimator and (b)
headroom-guard decisions — and, through ADR-018, the **ADR-006 velocity/projection
clause** (ADR-006's median-of-positive-deltas estimator). ADR-018 status is set to
**Superseded** (frontmatter, per the ADR-005 → ADR-015 precedent). ADR-006's
bounded-tail read, negative-delta reset detection, and `None`-with-<2-points
degradation remain intact, as do ADR-010's absolute OR-gate anchors and ADR-011's
absolute-tokens-only targeting. `related_adrs` is set to [ADR-018, ADR-006,
ADR-010] in frontmatter directly (the lore CLI rejects ADR↔ADR links).

## Alternatives Considered

- **Keep p90 + headroom guard (ADR-018 as accepted).** Rejected — p90 ≡ max at
  K=8 (the distinction is unrealizable) and the headroom gate is dead code
  (always preempted by projection_recycle). The design cannot behave as specified.
- **Raw `max` but keep the headroom guard "for the calm→spike blind spot."**
  Rejected — with velocity = max the guard is provably preempted; it adds a field
  and a gate that can never fire.
- **Lower-quantile / median (ADR-006 original).** Rejected by ADR-018 already —
  optimistic on bursty traffic, radiates false runway; wrong direction for an
  advisory.
