---
id: ADR-025
title: Vote-counter recycle-verdict model
status: Accepted
related_requirements:
  - REQ-016
related_adrs: []
related_stories:
  - STORY-012
related_tests: []
---

# ADR-025 - Vote-counter recycle-verdict model

## Context

ADR-010 fixed the recycle DECIDER as an OR-gate over absolute token thresholds
(watch ~32k, backstop ~128k) and DEFERRED signal (a) "behavioral degradation"
as "needs eval probing." ADR-024 (Draft) un-defers (a) and shows it is
realizable from `tool_use` / `tool_result` STRUCTURE — repetition, failure
streak, ping-pong — without eval probing. REQ-016 catalogs the candidate Tier-A
/ Tier-B variables. STORY-012 wants warning on a STUCK/SPINNING context, not
only on volume, and wants the top presentation stages split by something other
than raw occupancy.

Two problems remain with an absolute-threshold OR-gate as the DECIDER:

- **Occupancy is not discriminating.** A 442-log measurement of live healthy
  Claude sessions found **62% of healthy messages sit >200k tokens** as normal
  steady state. A gate that treats high occupancy as the trigger cries wolf on
  the majority of healthy long sessions. Occupancy cannot carry the decision.
- **A single absolute gate has no clean breakpoint** (ADR-010 already records
  this). Onset is non-uniform/cliff-like; the real onset detector is behavioral,
  but behavior alone is rare. No single axis is both sensitive and precise.

This ADR EXTENDS ADR-010 and ADR-024 — it does NOT supersede or edit them.
ADR-010 stays Accepted (never edit an Accepted ADR). ADR-011/ADR-020 (absolute
tokens, no ceiling-learning, no scaling-to-window) are untouched.

## Decision

Replace ADR-010's absolute-threshold OR-gate AS THE DECIDER with a multi-signal
**vote-counter**. Measure many independent worrisome signals, group them into
FAMILIES, and recommend recycle when **enough independent FAMILIES fire** — not
when any single absolute threshold is crossed.

- **Count FAMILIES, not raw detectors.** Volume and Speed are the same "tank
  filling" story; counting every detector would let one phenomenon stuff the
  ballot. A family casts at most one vote.
- **Occupancy becomes ONE WEAK VOTE**, not the gate — justified by the 442-log
  finding that 62% of healthy traffic sits >200k. It corroborates; it does not
  decide.
- The absolute bands from ADR-010 survive **as the Volume family's vote
  threshold**, not as the system-level decider. Projection (ADR-006/022) feeds
  Speed; cache-thrash `rho` (ADR-008/023) feeds Thrash; ADR-024's Tier-B signals
  feed Behavior.

### The five families

| Family | Detects | Source signals |
|---|---|---|
| **Volume** | context simply too big | occupancy % vs absolute band (ADR-010) — **weak vote** |
| **Speed** | occupancy climbing fast | token growth rate, projection-to-wall `tau` (ADR-006/022) |
| **Thrash** | compaction churn / wall-hits | cache churn `rho` (ADR-008/023), compaction events, `stop_reason=max_tokens` |
| **Behavior** | agent stuck/spinning — the **true-onset** detector | tool-call repetition / failure-streak / ping-pong (ADR-024 Tier-B) |
| **Drift** | slow session-long rot a short tail misses | EWMA / floor-trend of occupancy across compaction resets |

### Evidence-grounded constraints (these shape the model)

- **Occupancy is NOT discriminating** — 62% of healthy live Claude messages sit
  >200k (442-log measurement). → Volume is demoted to a weak vote.
- **Behavior is CROSS-PROVIDER** — error/structure confirmed on Claude
  (`is_error`), codex (`function_call_output.status=failed`, args = JSON string),
  opencode (`state.status=error`, args = JSON object). Repetition/ping-pong
  verified against a REAL nebius/SWE-rebench loop trajectory (idx27,
  `python reproduce_issue.py` ×8 at positions 34/36/52/58/78/86/89/93).
  Ping-pong REQUIRES a no-progress qualifier (same args / output unchanged) or it
  fires in 100% of sessions, including successful ones.
- **Dataset-vs-runtime error split** — live providers carry STRUCTURAL error
  flags (`is_error` / `status`), so RUNTIME detection stays structure-only and
  preserves brim's no-content-inspection invariant. The public OpenHands dataset
  flattened error into CONTENT text (exit-code sentinel `[... exit code 1]`,
  `ERROR:` prefix), so dataset CALIBRATION needs content-regex while RUNTIME
  detection stays structural. The invariant holds.
- **Calibration data source bifurcates:**
  - **Behavior** is calibratable from PUBLIC datasets — nebius SWE-rebench
    Unresolved split, tau2-bench cross-provider.
  - **Volume / Speed / Thrash / Drift** have NO public data: no public trajectory
    dataset carries per-turn tokens or compaction boundaries (confirmed by direct
    byte-inspection — all 201 idx27 messages share 5 keys, no `usage`). → LOCAL
    capture only: Claude/codex transcript `usage` + `PreCompact`/`PostCompact`
    hooks as a free ground-truth "wall-hit" label.
- **Tool-repetition is rare but high-precision** → keep as a near-decisive vote.
  Other decisive candidates: `stop_reason=max_tokens`, `is_error` streaks,
  `apiErrorStatus` 429, `<synthetic>` fallback.

### Invariants preserved (carried from ADR-010 / ADR-024)

- **Deterministic**, **transcript-only**, **advisory-only + read-only** — brim
  recommends, never recycles or mutates a session.
- **No ceiling-learning, no scaling thresholds to the advertised window**
  (ADR-011 / ADR-020 untouched). Absolute-token reasoning stays.

## The decided vote rule (settled)

These four parameters were left open at Draft with a RECOMMENDED default named
for each. All four are now SETTLED — the recommended default was chosen in every
case. They are the decision, not options; the recorded rationale (the evidence
already in this ADR) is retained.

1. **Threshold — how many families must fire.**
   **DECIDED: recommend recycle when >= 2 of 5 independent families fire.**
   Rationale: simple, robust to a single noisy family; the >= 3-of-5 and
   weighted-sum-threshold alternatives are rejected (later onset / re-inflates
   the demoted occupancy vote).

2. **Per-family vote weight.**
   **DECIDED: Behavior and Thrash weighted high; occupancy (Volume) weighted
   lowest.** Rationale: matches the evidence — occupancy is non-discriminating
   (442-log: 62% of healthy messages sit >200k), Behavior is the true onset.
   Equal-weight is rejected because it re-inflates the demoted occupancy vote.

3. **Decisive single-family override.**
   **DECIDED: YES.** A single unambiguous hard signal — `stop_reason=max_tokens`
   (the wall was hit) OR a confirmed tool-call loop — recommends recycle on its
   own, bypassing the family count. All other families remain count-only.
   Rationale: ignoring an unambiguous wall-hit until a second family agrees is
   strictly worse; these two signals are high-precision.

4. **Tier mapping — families onto the 5 presentation tiers**
   (lean / drift / bloated / stale / critical).
   **DECIDED: family-vote COUNT drives the tier; Behavior/Drift signals split the
   top two tiers (stale vs critical).** Rationale: directly closes ADR-024's
   stage-4/5 pure-occupancy blindness (STORY-012 acceptance criterion). Keeping
   tiers occupancy-derived is rejected — it leaves the top-tier blindness
   unresolved.

## Consequences

- The decider stops keying on the one axis (occupancy) that the 442-log data
  shows is non-discriminating; a stuck/spinning session below the absolute
  backstop can now win a recycle recommendation on Behavior + one corroborating
  family.
- Family-counting prevents the Volume/Speed "tank filling" story from
  manufacturing a false majority on its own.
- The Behavior family ships calibratable from public data; the other four require
  local capture (transcript `usage` + PreCompact/PostCompact hooks) before their
  votes are tuned — anchors stay tunable (ADR-010 posture). The vote RULE is now
  fixed; the per-family vote ANCHORS remain to be calibrated against that capture.
- Per decision #4, the REQ-005 JSON contract gains a per-family vote /
  behavioral-state field — coordinate with the 5-tier promotion (recycle roadmap
  #4). Advisory-only / read-only trust boundary is unchanged.

## Alternatives Considered

- **Keep ADR-010's absolute-threshold OR-gate as the decider.** Rejected — the
  442-log measurement shows occupancy is non-discriminating (62% of healthy
  traffic >200k); an occupancy-keyed gate cries wolf, and no single absolute
  threshold has a clean breakpoint.
- **Count raw detectors instead of families.** Rejected — Volume and Speed tell
  the same "tank filling" story; raw-detector counting lets one phenomenon stuff
  the ballot and re-inflates the demoted occupancy signal.
- **Behavior-only decider (drop the token families).** Rejected — tool-repetition
  is rare (12–29% of samples); high precision but low recall. It must be
  corroborated, hence the multi-family vote rather than a single behavioral gate.
- **Supersede ADR-010 / ADR-024.** Rejected — both remain correct within their
  scope; this ADR re-frames how their signals COMBINE (decider = family vote, not
  OR of absolutes) and extends them rather than replacing them.

<!-- EXTENDS ADR-010 (re-frames its OR-gate decider as a family vote-counter;
demotes occupancy to a weak vote) and ADR-024 (consumes its Tier-B behavioral
signals as the Behavior family) — ADR-010 stays Accepted, ADR-024 stays Draft;
neither is edited. ADR-011/ADR-020 (absolute tokens, no ceiling-learning) and
ADR-006/022 (projection), ADR-008/023 (cache-thrash) feed the Speed/Thrash
families unchanged. lore does not support adr<->adr links, so these relations are
recorded in prose. Realizes [[STORY-012]]; consumes the candidate catalog
[[REQ-016]]. -->
