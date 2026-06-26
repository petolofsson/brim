---
id: STORY-013
title: Collect ground-truth recycle labels from operator recycle events (compaction secondary) to calibrate thresholds
status: Draft
related_requirements:
  - REQ-016
  - REQ-007
related_adrs:
  - ADR-025
related_stories: []
related_tests: []
---

# STORY-013 - Collect ground-truth recycle labels from operator recycle events (compaction secondary) to calibrate thresholds

## User Story

As the brim maintainer calibrating the recycle verdict,
I want brim to record a ground-truth label every time a session is RECYCLED —
the operator/orchestrator restarting a session at a break — and, secondarily,
whenever the host auto-compacts, capturing the occupancy at the event and which
vote-counter families had fired in the turns leading up to it,
So that the CANDIDATE vote-counter thresholds (ADR-025) can later be tuned
against what a skilled operator judged was the moment to recycle, with no human
labeling and no eval probing.

## Context

- ADR-025 settled the recycle decider as a >=2-of-5 FAMILY vote-counter
  (Volume / Speed / Thrash / Behavior / Drift), but its per-family vote
  thresholds and weights are CANDIDATE values — uncalibrated. ADR-025 itself
  records that Volume / Speed / Thrash / Drift have NO public dataset and can
  only be calibrated from LOCAL capture. This story is the operator intent
  behind collecting that label.
- PRIMARY label = an observed RECYCLE / context-restart event (the operator or
  orchestrator restarting a session at a break; detectable as a session
  boundary / fresh-session-with-reset-context). This is the POSITIVE TARGET
  label: "a skilled operator judged it time to recycle HERE" — exactly the
  judgment ADR-025's model should imitate. It is plentiful in this operator's
  data.
- Why the trigger had to change: this operator NEVER lets the host compact —
  they always RECYCLE the context BEFORE the wall (this disciplined behavior is
  the very thing brim exists to support). So host auto-compaction events are
  ~absent in their transcripts, and a harvester keyed ONLY on compaction would
  collect ZERO labels from their data — the keystone premise of a
  compaction-only harvester is broken. The positive signal lives in the recycle
  events, not in compactions.
- SECONDARY label (demoted, still captured) = host compaction event = the "too
  late, the wall was hit" NEGATIVE case. Rare for this operator but recorded
  whenever it occurs. A recycle and a compaction BOTH manifest as a window
  reset, so detection REUSES brim's existing reset-point detection (REQ-007
  timeline drop); the two are distinguished by whether the reset was
  operator-initiated (recycle) or host-initiated (compaction), carried in an
  `event_type` field on every record.
- CIRCULARITY CAVEAT — NOW CONFIRMED (the OPEN question is RESOLVED). Operator
  fact: this operator recycles on OCCUPANCY — at ~100-300k tokens on a
  1M-context model (~10-30% full), by habit / at task breaks — NOT on brim's
  behavioral recommendation and NOT on stuck-detection. Confirmed consequences:
  1. CIRCULARITY: calibrating thresholds to these occupancy-triggered recycle
     labels would re-learn OCCUPANCY — re-inflating the Volume family and
     STARVING the Behavior family (operator recycles regardless of stuck-ness),
     CONTRADICTING ADR-025 (occupancy = one weak vote). => recycle-events are
     NOT a valid calibration target for Speed / Thrash / Behavior / Drift.
  2. DATA DROUGHT: preventive early recycling means sessions rarely run to
     behavioral failure => Behavior calibration MUST source stuck / loop /
     abandonment examples from PUBLIC FAILURE datasets (nebius Unresolved;
     Toolathlon timeout / max_turn_exceeded).
  3. VALID LOCAL USES: recycle events stay valid as a healthy-occupancy baseline
     (direct evidence occupancy is non-discriminating — operator healthy at
     ~10-30%) and as per-step token curves (OpenHands-style).
  4. NON-CIRCULAR PATH (NON-GOAL here): only OUTCOME labels are non-circular —
     occasionally let brim's BEHAVIORAL recommendation drive a recycle (A/B vs
     the occupancy habit) and log whether it helped or was premature. Rigorous
     follow-up = recycle roadmap #3.
  1M-anchor corollary: brim's absolute 128k backstop fires at ~12.8% on a
  1M-context host — inside the operator's own ~10-30% healthy band, reinforcing
  ADR-025's occupancy-demotion. (Recorded in REQ-017 context; Accepted ADRs
  ADR-010/011/020 and REQ-004 not edited.)
- The hard "wall-hit" end of the spectrum, which this disciplined operator's
  data will rarely supply, can be cross-referenced from PUBLIC failure datasets
  (e.g. nebius Unresolved trajectories; Toolathlon timeout / max_turn_exceeded)
  — a cross-reference for the negative tail, NOT in scope to ingest here.
- This is data COLLECTION for OFFLINE calibration, not a live verdict change.
  brim's verdict still ships the ADR-025 candidate thresholds unchanged; the
  label corpus accrues alongside it. The algorithm that CONSUMES the labels to
  tune thresholds is a SEPARATE later unit and is NOT in scope here.
- Provider reality: the operator does not run all providers locally. The
  harvester runs PER-PROVIDER over whatever has live transcript data (Claude
  confirmed present; codex / opencode / copilot may be absent or empty). A
  provider with no local data simply yields zero labels — absence is normal,
  never an error. Calibration is additive: enabling more providers later only
  enriches the same label store.

## Acceptance Criteria

- [ ] Each time brim observes a RECYCLE / context-restart in a session (PRIMARY
      label), one ground-truth label record is appended to a local label log;
      no human labeling is involved.
- [ ] Each time brim observes a host compaction (SECONDARY label), a record is
      likewise appended; every record carries an `event_type` distinguishing
      `recycle` (operator-initiated) from `compaction` (host-initiated).
- [ ] The label captures occupancy at the event AND which of the 5 ADR-025
      vote-counter families had fired in the N turns before the reset.
- [ ] Recycle and compaction detection REUSE brim's existing reset-point
      detection (REQ-007 timeline drop); no new detection mechanism is invented.
- [ ] The harvester is read-only with respect to sessions — it appends only to
      its own label log, never mutates a transcript, never recycles a session.
- [ ] Per-provider graceful degradation: a provider with no/empty local
      transcript data produces zero labels and no error; the feature does not
      require any specific provider.
- [ ] The CONFIRMED circularity caveat (operator recycles on occupancy, not on
      behavior) and its four consequences (circularity, data drought, valid
      local uses, outcome-label path) are recorded so labels are not trusted
      blindly.
- [ ] The verdict thresholds are unchanged by this story — it produces labels
      for offline calibration only; consuming them is out of scope.

<!-- Realizes the LOCAL-capture calibration source named in ADR-025 [[ADR-025]];
serves the candidate signal catalog REQ-016 [[REQ-016]] (the families/signals
being calibrated) and reuses the reset detection of REQ-007 [[REQ-007]].
PRIMARY label = operator recycle event (positive target); SECONDARY = host
compaction (negative wall-hit). Extends the operator intent of STORY-012 (warn
on stuck/spinning) and STORY-011 (warn before the host hard-compacts) — lore
does not support story<->story links, so those relations are recorded here in
prose. Outcome-based labeling and the recommendation-vs-judgment circularity
question are an out-of-scope follow-up (recycle roadmap #3). Behavior captured
per record is the REQ below. -->
