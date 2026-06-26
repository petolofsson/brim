---
id: REQ-017
title: "Recycle-label harvester: append-only ground-truth recycle labels (recycle primary, compaction secondary)"
status: Draft
related_requirements: []
related_adrs:
  - ADR-025
related_stories:
  - STORY-013
related_tests: []
---

# REQ-017 - Recycle-label harvester: append-only ground-truth recycle labels (recycle primary, compaction secondary)

## Requirement

The system shall optionally harvest GROUND-TRUTH recycle labels from observed
window-reset events, to feed offline calibration of the ADR-025 vote-counter
thresholds. The PRIMARY label is an operator/orchestrator RECYCLE event; host
auto-compaction is a SECONDARY label. This requirement specifies WHAT is
captured and the collection invariants; it does NOT specify the calibration
algorithm that consumes the labels (out of scope — a separate later unit).

* **Trigger — reuse existing detection, two event types.** A label is produced
  when, and only when, brim observes a window reset in a session's transcript.
  Detection shall REUSE brim's existing reset-point detection (the REQ-007
  timeline drop, already computed today and whose drop magnitude is discarded
  per REQ-016 Tier-A "compaction-drop magnitude"). No new detection mechanism
  shall be invented. A window reset arises two ways and each record shall carry
  an `event_type` distinguishing them:
  - `recycle` (PRIMARY) — operator/orchestrator restarted the session at a
    break (a session boundary / fresh-session-with-reset-context). This is the
    POSITIVE TARGET label: a skilled operator judged it was time to recycle
    HERE — the judgment ADR-025's model should imitate. Plentiful in this
    operator's data, because this operator recycles BEFORE the host would
    compact.
  - `compaction` (SECONDARY) — the host auto-compacted/reset the window: the
    "too late, the wall was hit" NEGATIVE case. Rare for this operator but
    captured whenever it occurs.
  The two are distinguished by whether the reset was operator-initiated
  (recycle) or host-initiated (compaction).

* **Label record — what is captured.** For each observed reset, the harvester
  shall append ONE record containing at least:
  - **occupancy at the event** — window tokens
    (`input + cache_read + cache_creation`, per REQ-007) of the last turn
    BEFORE the reset, in absolute tokens only (no fill %, ADR-011);
  - **family-fire vector** — which of the 5 ADR-025 vote-counter families
    (Volume / Speed / Thrash / Behavior / Drift) had fired in the N turns
    immediately preceding the reset (N = a documented bounded window, consistent
    with REQ-001 / REQ-007 tail bounds);
  - **provenance** — provider id, session id, the reset turn index `s`, the drop
    magnitude, and `event_type` (`recycle` | `compaction`);
  - the record is the deterministic function of the bounded transcript tail —
    same transcript in, same record out.

* **Append-only local log — consumer-side persistence.** The harvester shall
  only APPEND records to its own local label log (e.g. a JSONL file under a
  documented path); it shall never rewrite or delete prior records. This is the
  consumer-owned persistence sink that ADR-004 explicitly permits — it is NOT
  brim mutating a session. The read-only-over-sessions invariant holds: the
  harvester never writes to, recycles, or otherwise mutates any agent
  transcript or session (ADR-002 / ADR-025).

* **Per-provider graceful degradation.** The harvester shall run PER-PROVIDER
  over whatever providers have live local transcript data. A provider with no
  data or an empty/unparseable transcript shall yield ZERO labels and NO error
  — absence of a provider is never a failure. The feature shall NOT require any
  specific provider (Claude is confirmed present; codex / opencode / copilot
  may be absent). Enabling more providers later only enriches the same store
  (additive calibration).

* **Determinism & no content inspection.** Label production shall be
  deterministic and transcript-only, derived from `usage` blocks, line/turn
  counts, timestamps, and `tool_use` / `tool_result` STRUCTURE (the same
  signals ADR-025 / REQ-016 already permit) — never natural-language content
  and never an eval probe.

* **Invariants untouched.** No ceiling-learning and no scaling of thresholds to
  the advertised window (ADR-011 / ADR-020 untouched). The live verdict is
  unchanged by this requirement — the harvester only PRODUCES labels; it does
  not alter how brim decides recycle.

* Absent or unparseable turns shall be skipped, never panic.

## Rationale

- ADR-025 records that four of the five families (Volume / Speed / Thrash /
  Drift) have NO public dataset and can be calibrated only from LOCAL capture.
  The original framing named the host compaction event as a "free ground-truth
  wall-hit label," but that premise breaks for this operator: a disciplined
  operator RECYCLES before the host ever compacts, so compaction events are
  ~absent from their transcripts and a compaction-only harvester would collect
  ZERO labels. The positive, plentiful signal is the RECYCLE event itself —
  "a skilled operator judged it time to recycle HERE" — so the recycle event is
  the PRIMARY label and host compaction is demoted to a SECONDARY negative
  ("wall hit") label, captured when it does occur.
- Both events are the SAME mechanism — a window reset already detected by
  REQ-007 (REQ-016 flags its drop magnitude as a discarded Tier-A signal).
  Harvesting reuses that detection and tags each record with `event_type`
  rather than inventing a mechanism, keeping the change deterministic and
  transcript-only.
- CIRCULARITY CAVEAT — NOW CONFIRMED (no longer conditional). The previously
  OPEN question is RESOLVED by operator fact: this operator recycles on
  OCCUPANCY — at ~100-300k tokens on a 1M-context model (i.e. ~10-30% full), by
  habit / at task breaks — NOT on brim's behavioral recommendation and NOT on
  any stuck-detection. The recycle trigger is therefore an occupancy habit, and
  the following consequences are confirmed, not hypothetical:
  1. CIRCULARITY (confirmed): because recycle is occupancy-triggered,
     calibrating the vote-counter's thresholds to these recycle-event labels
     would re-learn OCCUPANCY as the trigger — re-inflating the Volume family
     and STARVING the Behavior family (the operator recycles regardless of
     stuck-ness). That directly CONTRADICTS ADR-025's thesis (occupancy demoted
     to a single weak vote). => operator recycle-events are NOT a valid
     calibration target for the non-occupancy families
     (Speed / Thrash / Behavior / Drift).
  2. DATA DROUGHT (confirmed): preventive early recycling (~10-30% full) means
     the operator's sessions rarely run to behavioral failure => few/no local
     stuck-session examples. Behavior-trigger calibration MUST source
     stuck / loop / abandonment examples from PUBLIC FAILURE datasets (nebius
     Unresolved split; Toolathlon timeout / max_turn_exceeded), already
     cross-referenced as the negative tail.
  3. WHAT LOCAL DATA IS STILL GOOD FOR: the recycle events ARE valid as a
     healthy-occupancy baseline — direct evidence that occupancy is
     non-discriminating, since the operator is healthy at ~10-30% full — and as
     per-step token curves (OpenHands-style). Keep these as valid local uses.
  4. THE NON-CIRCULAR PATH (calibration approach; still NON-GOAL to build here):
     the only way to get non-circular LOCAL labels is OUTCOME labels —
     occasionally let brim's BEHAVIORAL recommendation drive a recycle (A/B
     against the occupancy habit) and log whether it helped or was premature.
     Recorded as the rigorous follow-up (recycle roadmap #3), out of scope here.
- The hard "wall-hit" end of the spectrum, which this operator's disciplined
  data rarely supplies, can be cross-referenced from PUBLIC failure datasets
  (nebius Unresolved trajectories; Toolathlon timeout / max_turn_exceeded) — a
  cross-reference for the negative tail, NOT in scope to ingest in this
  harvester.
- 1M-HOST ABSOLUTE-ANCHOR NOTE (answers the open 1M-host anchor question;
  recorded here because ADR-010 / ADR-011 / ADR-020 and REQ-004 are all Accepted
  and must not be edited): on a 1M-context host, brim's absolute 128k backstop
  fires at ~12.8% occupancy, and this operator is healthy recycling at ~10-30%
  full. The absolute anchor therefore sits at the low end of the operator's own
  healthy band — reinforcing ADR-025's occupancy-demotion (occupancy is a weak,
  non-discriminating vote on large-context hosts). If this belongs in an
  Accepted ADR's rationale, a NEW superseding ADR is required; flagged, not
  applied here.
- Persisting a label log appears to tension ADR-004 ("brim stays read-only and
  stateless"), but ADR-004 assigns time-series persistence to the CONSUMER and
  explicitly permits the consumer to save to "a chosen file." The harvester is
  that consumer-side sink; the read-only guarantee that matters — never
  mutating a session/transcript — is preserved.
- Provider availability is a hard local constraint: the maintainer does not run
  every provider. Graceful per-provider degradation makes calibration additive
  and avoids coupling the feature to any one provider.

## Acceptance Criteria

- [ ] A label record is appended on each observed window reset, using brim's
      EXISTING reset-point detection (REQ-007) — no new detection mechanism —
      with `event_type` set to `recycle` (PRIMARY, operator-initiated) or
      `compaction` (SECONDARY, host-initiated).
- [ ] Each record captures occupancy at the event (absolute tokens) and the
      family-fire vector over the bounded N turns before the reset, plus
      provider/session provenance, reset turn index, drop magnitude, and
      `event_type`.
- [ ] The label log is append-only and local; the harvester never mutates,
      recycles, or writes to any session/transcript (read-only-over-sessions
      preserved; consumer-side persistence per ADR-004).
- [ ] A provider with no/empty/unparseable local data yields zero labels and no
      error; no specific provider is required; more providers only enrich the
      store.
- [ ] Label production is deterministic, transcript-only, and uses no content
      inspection and no eval probing.
- [ ] The CONFIRMED circularity caveat (operator recycles on occupancy, not on
      behavior/stuck-detection) and its four consequences — circularity,
      data drought, valid local uses, non-circular outcome-label path — are
      recorded with the labels so they are not trusted blindly; the
      public-failure-dataset cross-reference for the wall-hit tail is noted as
      out of scope.
- [ ] ADR-011 / ADR-020 invariants are untouched and the live recycle verdict
      is unchanged — this requirement produces labels for offline calibration
      only; consuming them is out of scope.

<!-- Produces the LOCAL-capture calibration corpus named in ADR-025
[[ADR-025]]; serves STORY-013 [[STORY-013]]. PRIMARY label = operator recycle
event (positive target); SECONDARY = host compaction (negative wall-hit);
distinguished by `event_type`. Captures the families of ADR-025 and the Tier-A
"compaction-drop magnitude" signal catalogued in REQ-016; reuses the reset
detection and bounded tail read of REQ-007; persistence framed per ADR-004;
read-only per ADR-002; absolute-token / no-ceiling-learning invariants per
ADR-011 / ADR-020. lore does not support req<->req links, so REQ-016 / REQ-007
relations are recorded here in prose. Outcome-based labeling and the
recommendation-vs-judgment circularity question are an out-of-scope follow-up
(recycle roadmap #3); the public failure-dataset cross-reference (nebius /
Toolathlon) for the wall-hit tail is a cross-reference only, not ingested here.
The calibration algorithm that CONSUMES these labels is a separate later unit
and out of scope. -->
