---
id: STORY-014
title: Calibrate Behavior thresholds from public failure trajectories because local recycle labels are occupancy-monotone (only Volume fires)
status: Draft
related_requirements:
  - REQ-018
related_adrs:
  - ADR-034
  - ADR-025
related_stories: []
related_tests:
  - TEST-014
---

# STORY-014 - Calibrate Behavior thresholds from public failure trajectories because local recycle labels are occupancy-monotone (only Volume fires)

## User Story

As the brim maintainer calibrating the recycle verdict,
I want an offline tool that recommends the Behavior-family vote thresholds by
sweeping them against STUCK/SPINNING examples from a public agent-trajectory
dataset I have downloaded,
So that the ADR-025 Behavior signals stop relying on uncalibrated CANDIDATE
thresholds — using real failure data that my own sessions can never supply,
because I recycle on occupancy long before any session runs to behavioral
failure.

## Context

- ADR-025 made the recycle decider a >=2-of-5 FAMILY vote-counter
  (Volume / Speed / Thrash / Behavior / Drift) with CANDIDATE, uncalibrated
  per-family thresholds. Behavior (tool-call repetition / failure streak /
  ping-pong) is the TRUE-ONSET detector — the one that should catch a
  stuck/spinning agent that occupancy alone misses.
- WHY LOCAL DATA CANNOT CALIBRATE BEHAVIOR (now empirical, not hypothetical).
  REQ-017 / STORY-013's harvester collected the local ground-truth recycle
  labels, and the corpus is occupancy-monotone: the 56 harvested recycle labels
  ALL carry `family_fires = [true, false, false, false, false]` — only Volume
  fires; Speed / Thrash / Behavior / Drift get ZERO local positives. This is the
  confirmed circularity / data-drought consequence REQ-017 predicted: the
  operator recycles at ~10-30% full by occupancy habit, so sessions never reach
  behavioral failure and there is nothing local to calibrate Behavior against.
- THE FIX: source stuck/spinning examples from a PUBLIC failure dataset, exactly
  the cross-reference ADR-025 / REQ-017 named for the wall-hit tail. This story
  is the operator intent to BUILD that calibration adapter for the Behavior
  family.
- SCOPE DISCIPLINE: Behavior ONLY. Volume is excluded (locally circular —
  calibrating occupancy to occupancy-triggered labels re-learns occupancy).
  Speed and Thrash are DEFERRED: no public dataset carries per-step tokens or
  cache-hit data, so they stay at ADR-025 candidate values and the gap is
  recorded explicitly (ADR-034). The operator accepts this known gap.
- ADVISORY, NOT AUTOMATIC: the tool RECOMMENDS thresholds (a precision/recall
  report); it never edits `verdict.rs`. The maintainer wants to inspect the
  precision/recall table and decide. The actual threshold VALUES and the ADR
  that supersedes ADR-025's candidate annotations come in a SEPARATE FUTURE unit,
  after the maintainer runs the tool against the real downloaded dataset.
- DATA HANDLING: the maintainer downloads `nebius/SWE-rebench-openhands-
  trajectories` (CC-BY-4.0) locally and points the tool at it; nothing is
  fetched at runtime and no dataset is vendored into the repo. Tests use
  synthetic fixtures only.
- LABEL QUALITY: the maintainer wants a FILTERED "clearly stuck" positive subset
  (agent kept working yet failed), not every `resolved == 0`, because raw
  unresolved trajectories mix in environment/localization failures that are not
  stuck/spinning — favoring precision over sample count, consistent with brim's
  conservative recycle posture.

## Acceptance Criteria

- [ ] The maintainer can run an offline subcommand against a locally downloaded
      public trajectory dataset and get recommended Behavior thresholds without
      any network access and without modifying any session or the dataset.
- [ ] The recommendation is grounded in STUCK/SPINNING examples — a FILTERED
      "clearly stuck" positive subset — not in the occupancy-monotone local
      recycle corpus that cannot distinguish Behavior.
- [ ] The report shows, per Behavior sub-signal, a precision/recall table and a
      recommended threshold meeting a precision target; the tool never edits
      `verdict.rs`.
- [ ] Scope is Behavior only; Volume is excluded and Speed/Thrash are explicitly
      recorded as a deferred known gap (candidate values retained).
- [ ] The occupancy-monotone local-label finding (56 labels, only Volume fires)
      is recorded as the motivation so the public-dataset detour is not
      second-guessed later.
- [ ] The live verdict and ADR-011 / ADR-020 invariants are unchanged — this
      story produces a recommendation tool for offline calibration only.

<!-- Continues the calibration arc of STORY-013 (recycle-label harvester):
that harvester produced the occupancy-monotone local corpus this story responds
to. Realizes the PUBLIC-failure-dataset Behavior path named in ADR-025
[[ADR-025]]; behavior in REQ-018 [[REQ-018]]; methodology in ADR-034
[[ADR-034]]; coverage in TEST-014 [[TEST-014]]; boundary in FEATURE-004. lore
does not support story<->story links, so the STORY-013 relation is recorded here
in prose. Threshold VALUES and the ADR-025-superseding value-ADR are an
out-of-scope FUTURE unit. -->
