---
id: ADR-022
title: Velocity uses windowed max (last W positive deltas) to decay isolated-spike stickiness; supersedes ADR-021
status: Accepted
related_requirements: []
related_adrs:
  - ADR-021
  - ADR-006
  - ADR-010
  - ADR-011
related_stories: []
related_tests: []
---

# ADR-022 - Velocity uses windowed max (last W positive deltas) to decay isolated-spike stickiness; supersedes ADR-021

## Context

ADR-021 adopted **velocity = MAX of recent post-reset positive deltas** as the
correct pessimistic estimator (p90 ≡ max at K=8; the p90 distinction was
unrealizable at this tail length). The Consequences section of ADR-021 flagged
the accepted trade-off:

> **Sticky over-warning** for up to K=8 turns after an isolated large delta —
> that delta stays in the tail window and keeps velocity high until it scrolls off.

In practice an isolated spike (one very large positive delta surrounded by normal
small deltas) keeps the `projection_recycle` gate firing `Over` for up to K−1=7
additional turns even when actual growth has returned to its baseline rate. For an
advisory whose value is lead time, this stickiness erodes user trust: the warning
fires repeatedly after the cause is gone, and operators begin to ignore it.

The fix must preserve the pessimistic direction — err **early** on genuine
sustained bursts — while shortening the stickiness period for isolated spikes.

## Decision

Replace **global max** with **windowed max over the last W positive deltas**:

```
velocity = max(pos_deltas[max(0, n−W) ..])
```

where `W = TREND_TAIL_K / 2 = 4` and `n = pos_deltas.len()`.

A new constant `VELOCITY_WINDOW_W: usize = TREND_TAIL_K / 2` is introduced in
`src/window.rs`. The rest of the velocity/projection contract (the `Option<u64>`
field, None-degradation at <2 post-reset points, reset detection) is unchanged,
as are the absolute OR-gate thresholds from ADR-010/ADR-011.

**Stickiness bound:** An isolated spike at position `i` in the positive-delta
list is aged out as soon as at least W newer positive deltas have arrived, i.e.
at most W turns of stickiness instead of K−1=7. With W=4 that is a reduction
from 7 to ≤3 additional warning turns after a burst.

**Genuine burst preserved:** If the most recent W positive deltas are all large
(a real sustained burst), `max(last W)` equals the global max — no change in
behavior. The gate still fires `Over` on the very first turn of a burst (same
as ADR-021).

## Consequences

- Isolated spike over-warning: at most `W−1 = 3` sticky turns instead of
  `K−1 = 7`. Significant reduction in false-positive stickiness.
- Genuine burst: no change — `projection_recycle` fires on turn 1 of any
  real sustained large-delta run, same as ADR-021.
- Absolute gates (`w_n >= recycle_backstop` → Over; `w_n >= watch_tokens` →
  Nearing; ADR-010 §4) are **not touched**. The safety net is unchanged.
- The velocity contract (`Option<u64>`, None-on-<2-points, reset detection) is
  preserved from ADR-006 and ADR-021.
- Minimal code change: two lines in `compute_trend` (src/window.rs); no changes
  to src/verdict.rs.

## Alternatives Considered

- **Exponentially-decayed max** (`max over i of delta[i] * alpha^(n−1−i)`).
  Rejected — the effective value of a spike decays by halves each turn but
  never reaches zero, so the spike can still dominate velocity many turns later
  at a reduced (but non-zero) effective value. The cutoff is gradual and
  harder to test deterministically. Windowed max gives a clean hard cutoff.
- **Recency-weighted max** (similar to exp-decay but with linear weights).
  Same issue: gradual bleed-through, no hard cutoff, harder to test.
- **Retain ADR-021 global max.** Rejected — the documented sticky-overwarning
  trade-off is now judged unacceptable after observing operator behavior;
  false positives persisting 7 turns after a spike erode signal value.
- **Lower W further (W=2 or W=3).** Would reduce stickiness to 1–2 turns but
  risks missing a real sustained burst whose large deltas arrive at alternating
  turns with zero-delta turns (e.g., tool-call turns). W=4 (half of K=8) is a
  conservative midpoint.
