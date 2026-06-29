---
id: FEATURE-004
title: Behavior-family threshold calibration from public failure datasets
status: Draft
related_requirements:
  - REQ-018
  - REQ-017
related_adrs:
  - ADR-034
  - ADR-025
  - ADR-033
related_stories:
  - STORY-014
related_tests:
  - TEST-014
---

# FEATURE-004 - Behavior-family threshold calibration from public failure datasets

## Feature

An offline, operator-run calibration unit that recommends the Behavior-family
vote thresholds for the ADR-025 vote-counter. The ADR-025 Behavior signals
(tool-call repetition, failure streak, ping-pong) ship with CANDIDATE thresholds
that no local data can calibrate: this operator recycles on occupancy at
~10-30% full (REQ-017 / STORY-013), so the local recycle-label corpus is
occupancy-monotone — empirically the 56 harvested recycle labels all carry
`family_fires = [true, false, false, false, false]` (only Volume fires;
Speed / Thrash / Behavior / Drift get no local positives). Behavior calibration
therefore MUST source stuck/spinning examples from a PUBLIC failure dataset.

A new `brim calibrate` subcommand reads a public agent-trajectory dataset the
operator has already downloaded locally, maps each trajectory to the same
`BehaviorSignals` the live verdict uses, labels a FILTERED "clearly stuck"
positive subset, sweeps the Behavior sub-signal thresholds against those labels,
and emits a precision/recall recommendation report (text and `--json`). The tool
is ADVISORY: it RECOMMENDS thresholds; it never auto-edits `verdict.rs`. The
final threshold VALUES and the ADR that supersedes ADR-025's candidate
annotations come in a FUTURE unit, after the operator runs against the real
downloaded data.

Scope is the Behavior family ONLY. Volume is excluded (locally circular). Speed
and Thrash are DEFERRED — no public per-step-token or cache-hit data exists — and
stay at their ADR-025 candidate values; this gap is recorded explicitly
(ADR-034).

## Scope

- New subcommand `brim calibrate --dataset <path> --format nebius
  [--precision-target 0.80] [--json]`.
- Reads `nebius/SWE-rebench-openhands-trajectories` (CC-BY-4.0, ~67K) files the
  operator downloaded locally; offline, read-only, deterministic.
- Maps trajectory fields to `BehaviorSignals` (tool-call name+args hash vector,
  per-step error flags via content-regex on downloaded content, a
  `stop_reason_max_tokens` proxy), reusing the runtime `BehaviorSignals`
  extraction so calibration and runtime logic never drift.
- Positive label = FILTERED "clearly stuck" subset (`exit_status != submit` AND
  the agent kept working, e.g. `gen_tests_correct > 0` yet still failed) — NOT
  all `resolved == 0`, trading recall for label precision.
- Precision-target threshold sweep over `BEHAVIOR_REPETITION` /
  `BEHAVIOR_STREAK` / `BEHAVIOR_PING_PONG`; emits a precision/recall/F1 table
  plus a `RECOMMENDED:` line.
- Content-regex runs ONLY on downloaded dataset content (permitted by ADR-025);
  live sessions remain structural-only.
- Tests use SYNTHETIC fixtures only; no real dataset is vendored into the repo.

## Out of Scope

- Volume calibration (locally circular — occupancy re-learns occupancy).
- Speed and Thrash calibration (no public per-step-token / cache-hit data;
  deferred to a future local-capture phase; candidate values retained).
- Final threshold VALUES and the ADR superseding ADR-025's candidate
  annotations (a FUTURE unit, after the operator's real-data run).
- Automatic editing of `verdict.rs` — the tool is advisory only.
- Any network fetch at runtime; vendoring the dataset; inspecting live-session
  natural-language content.
- Toolathlon / nvidia datasets (not used now); CAT-Instruct (does not exist).

## Included Artifacts

- REQ-018 — offline calibration adapter behavior.
- STORY-014 — operator intent (local labels occupancy-monotone).
- ADR-034 — calibration methodology decisions.
- TEST-014 — synthetic-fixture coverage.
- Builds on ADR-025 (vote-counter / Behavior signals), ADR-033 + REQ-017
  (recycle-label harvester that produced the monotone local corpus).
