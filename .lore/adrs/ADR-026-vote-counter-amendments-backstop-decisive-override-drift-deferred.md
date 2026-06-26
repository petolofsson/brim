---
id: ADR-026
title: 'Vote-counter amendments: backstop decisive override + Drift deferred'
status: Accepted
related_requirements:
  - REQ-016
related_adrs: []
related_stories:
  - STORY-012
  - STORY-011
related_tests: []
---

# ADR-026 - Vote-counter amendments: backstop decisive override + Drift deferred

## Context

ADR-025 (Accepted) settled the vote-counter recycle-verdict model: measure
independent worrisome signals, group them into FAMILIES (Volume / Speed /
Thrash / Behavior / Drift), and recommend recycle when >= 2 of 5 independent
families fire, with two decisive single-family overrides (`stop_reason=max_tokens`,
confirmed tool-call loop). ADR-024 (Draft) supplied the Behavior family.

Implementing ADR-025 against the live provider feed surfaced two facts that the
Draft model did not account for:

- **Behavior-blind providers can never reach Over.** copilot and opencode read
  `behavior:None` — their transcripts do not expose the tool-block structure the
  Behavior family needs. Under the pure family count, the strongest worrisome
  axis (Behavior, the true-onset detector) is permanently absent for them, so a
  growing-but-not-thrashing session on those providers could climb past the host
  hard-compaction point without ever assembling >= 2 firing families. That
  silently drops the STORY-011 floor ("warn me to recycle before the host
  hard-compacts") for two of the providers brim supports.
- **Drift double-counts the tail.** drift_score is computed from the same K=8
  message tail as the Speed family. Sharing the input makes Drift correlate with
  Speed (and through it Volume) — three "votes" that are not independent
  observations. Counting Drift as a fifth family lets one phenomenon (recent
  tail behavior) stuff the ballot, which is exactly the failure mode ADR-025
  rejected raw-detector counting to avoid ("count INDEPENDENT families").

This ADR AMENDS ADR-025. It does NOT supersede or edit it — ADR-025 stays
Accepted and its decision (family vote-counter, >= 2-of-N threshold, weak
occupancy, decisive overrides, tier mapping) stands. This ADR adjusts two
mechanics within that model and is itself Accepted: the amendments are
user-approved, implemented in the shipped engine, and reviewed.

## Decision

### Amendment M1 — occupancy at/above `recycle_backstop` is a DECISIVE OVERRIDE

Occupancy at or above the absolute `recycle_backstop` band now recommends Over
on its own, bypassing the family count. It is the THIRD decisive trigger,
alongside the two ADR-025 already named:

1. `stop_reason=max_tokens` (the wall was hit),
2. a confirmed tool-call loop (Behavior),
3. **occupancy >= `recycle_backstop`** (new).

Rationale: behavior-blind providers (copilot, opencode = `behavior:None`) could
otherwise NEVER reach Over through the family count — the Behavior family is
structurally unavailable to them. Making backstop occupancy a decisive override
restores the STORY-011 "warn before the host hard-compacts" floor for every
provider, behavior-blind or not. This does NOT re-inflate occupancy in general:
occupancy remains a WEAK vote in the family count everywhere below the backstop
(ADR-025's 442-log demotion is untouched). The override is a hard floor at the
top of the band, not a return to occupancy-as-decider.

### Amendment M3 — the Drift family is EXCLUDED from the family count

The family count now uses FOUR genuinely-independent families: Volume, Speed,
Thrash, Behavior. Drift is removed from the count.

Reason: drift_score is computed from the same K=8 tail as Speed, so it
correlates with Speed (and Volume) — ballot-stuffing that violates ADR-025's
"count INDEPENDENT families" rule. Drift is not an independent observation of a
distinct phenomenon; it is a re-read of the tail Speed already votes on.

Drift is NOT deleted:

- drift_score is still computed and still emitted in `--json` (informational).
- drift_score may only split **Stale vs Bloated WITHIN the >= 2 band** (a tier
  refinement, per ADR-025 decision #4's "Behavior/Drift split the top tiers").
  It may NEVER escalate the family count and NEVER reach Over by itself.

Real long-horizon Drift — EWMA / floor-trend of occupancy ACROSS compaction
resets, the genuinely-independent slow-rot signal ADR-025's Drift family was
meant to capture — is DEFERRED to roadmap #2 (long-horizon drift). The current
single-tail drift_score is a stopgap, not that signal.

Stopgap note: the rigorous fix for the correlation is not "drop Drift" but
inverse-covariance decorrelation (MEWMA) over the family vector — see
docs/recycle-research-findings.md sec 9. That is a v2 direction, not this
change. M3 removes the double-count cheaply now; MEWMA would let a decorrelated
Drift re-enter the count later.

### Invariants preserved

All ADR-025 invariants carry unchanged:

- **Deterministic** — same transcript in, same verdict out.
- **Transcript-only** — structure and `usage`/timestamps, no content inspection.
- **Advisory-only + read-only** — brim recommends, never recycles or mutates.
- **No ceiling-learning, no scaling thresholds to the advertised window**
  (ADR-011 / ADR-020 untouched). Absolute-token reasoning stays; M1's backstop
  override keys on the same absolute band ADR-010 fixed.

## Consequences

- Behavior-blind providers (copilot, opencode) regain a guaranteed Over path at
  the backstop; the STORY-011 floor holds for every provider.
- The family count is now 4 genuinely-independent families. A false majority can
  no longer be manufactured from the tail alone (Speed + Drift double vote
  removed).
- REQ-005 `--json` contract changes (coordinated separately): `family_votes`
  reports the 4-family `count` plus `drift` as an informational-only bool; the
  provisional `tool_repeat_run` spike field is subsumed; `verdict_gate` gains
  `decisive_override` and `family_vote`; a `tier` field and a `decisive_override`
  bool are added.
- Long-horizon Drift (roadmap #2) and MEWMA decorrelation (research sec 9)
  remain open v2 directions; this ADR neither closes nor blocks them.

## Alternatives Considered

- **Leave the family count at 5 (keep Drift).** Rejected — drift_score shares
  the Speed tail; counting it double-counts one phenomenon, the exact
  ballot-stuffing ADR-025 forbade.
- **Decorrelate Drift now via MEWMA instead of excluding it.** Rejected for this
  change — MEWMA is the rigorous v2 fix (research sec 9) but is a larger build;
  excluding Drift from the count removes the double-count immediately and keeps
  drift_score available for the within-band tier split.
- **Leave the backstop as a weak Volume vote only.** Rejected — under the pure
  count, behavior-blind providers never reach Over, dropping the STORY-011 floor
  for copilot/opencode. A decisive backstop override is the minimal restoration.
- **Make occupancy a strong vote everywhere.** Rejected — re-inflates the
  442-log-demoted occupancy axis ADR-025 deliberately weakened. The override is
  a top-of-band hard floor, not a general re-weighting.

<!-- AMENDS ADR-025 (Accepted, not edited, not superseded): M1 adds backstop
occupancy as a third decisive override; M3 excludes Drift from the family count
(4 independent families: Volume/Speed/Thrash/Behavior) and defers real
long-horizon Drift to roadmap #2. Consumes the Behavior family from ADR-024 and
preserves ADR-010/011/020 absolute-token invariants. lore does not support
adr<->adr links, so the ADR-025<->ADR-026 amends/amended-by relation and the
ADR-024 link are recorded here in prose. Restores the [[STORY-011]] floor for
behavior-blind providers; serves [[STORY-012]]; consumes catalog [[REQ-016]].
Rigorous decorrelation alternative: docs/recycle-research-findings.md sec 9. -->
