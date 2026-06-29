---
id: REQ-018
title: "Offline Behavior calibration adapter: reads downloaded nebius trajectories, extracts BehaviorSignals, sweeps thresholds to a precision/recall recommendation"
status: Accepted
related_requirements: []
related_adrs:
  - ADR-034
  - ADR-025
related_stories:
  - STORY-014
related_tests:
  - TEST-014
---

# REQ-018 - Offline Behavior calibration adapter: reads downloaded nebius trajectories, extracts BehaviorSignals, sweeps thresholds to a precision/recall recommendation

## Requirement

The system shall provide an offline, operator-run calibration subcommand
`brim calibrate` that RECOMMENDS the ADR-025 Behavior-family vote thresholds by
sweeping them against labels derived from a public agent-trajectory dataset the
operator has downloaded locally. The subcommand PRODUCES a recommendation report
only; it does NOT change the live verdict and does NOT auto-edit any threshold
constant. Scope is the Behavior family ONLY.

* **Input — locally downloaded public dataset.** `brim calibrate --dataset
  <path> --format nebius` shall read `nebius/SWE-rebench-openhands-trajectories`
  (CC-BY-4.0) JSONL files the operator has already downloaded to a local path.
  It shall perform NO network fetch and shall never write to, mutate, or delete
  any transcript, session, or the dataset itself (read-only). `nebius` is the
  only format supported in this unit (Toolathlon / nvidia not used now;
  CAT-Instruct does not exist).

* **Signal extraction — reuse runtime BehaviorSignals.** For each trajectory the
  adapter shall map dataset fields to the SAME `BehaviorSignals` the live verdict
  uses (`verdict.rs`), so calibration and runtime logic never drift:
  - tool-call vector: per assistant message, per tool call, `(tool_name,
    stable_hash(args))` where `args` is deserialized from the JSON-string
    `function.arguments` — `Vec<(String, u64)>` as
    `BehaviorSignals::from_signals` expects;
  - error-flag vector: per tool-result message, `true` iff content matches the
    error-marker regex set (`[... exit code N]` with N >= 1, `ERROR:`,
    `Traceback`, `Exception:`), else `false` — `Vec<bool>`;
  - `stop_reason_max_tokens` proxy: `exit_status != "submit"` (conservative).

* **Positive label — FILTERED "clearly stuck" subset.** The positive class
  (should-fire Behavior) shall be the FILTERED subset where `exit_status` is not
  `submit` AND the agent demonstrably kept working (e.g. `gen_tests_correct > 0`
  yet still failed) — NOT all `resolved == 0`. Negative class = the resolved /
  submitted trajectories. This trades recall for label precision (precision over
  N), since `resolved == 0` also contains environment/localization/harness
  failures that are not "stuck/spinning."

* **Threshold sweep — precision-target.** For each Behavior sub-signal
  (`BEHAVIOR_REPETITION`, `BEHAVIOR_STREAK`, `BEHAVIOR_PING_PONG`) the adapter
  shall sweep candidate thresholds `T` over a bounded range and, per `T`, count
  TP / FP / FN against the labels and compute precision / recall / F1. The
  RECOMMENDED `T` per sub-signal shall be the lowest `T` whose precision meets
  `--precision-target` (default 0.80), ties broken by higher recall. If no `T`
  meets the target, the report shall say so and retain the candidate value
  rather than recommend a low-precision threshold.

* **Output — advisory report, text and JSON.** The adapter shall emit a
  per-sub-signal precision/recall/F1 table plus a single `RECOMMENDED:
  repetition=N streak=N ping_pong=N` line to stdout, and the same content as
  structured records under `--json`. It shall NOT edit `verdict.rs` or any
  constant; applying recommendations is a separate FUTURE operator step.

* **Content-regex boundary.** The error-marker regex shall run ONLY on the
  DOWNLOADED dataset content. This is the dataset-calibration content inspection
  ADR-025 already permits; the RUNTIME verdict and all live-session detection
  remain STRUCTURAL-only (no natural-language content inspection).

* **Determinism & robustness.** Same dataset files in ⇒ same report out (no
  random sampling). Absent, empty, or unparseable trajectories shall be skipped,
  never panic; a malformed `function.arguments` string shall drop that tool call,
  not abort the run.

* **Scope limits.** The adapter shall calibrate the Behavior family ONLY. It
  shall NOT calibrate Volume (locally circular), and it shall NOT attempt Speed
  or Thrash (no public per-step-token / cache-hit data — DEFERRED; candidate
  values retained per ADR-034). Invariants ADR-011 / ADR-020 (absolute tokens,
  no ceiling-learning, no scaling-to-window) are untouched.

## Rationale

- The recycle-label harvester (REQ-017 / ADR-033 / STORY-013) confirmed that
  local labels cannot calibrate Behavior: this operator recycles on OCCUPANCY at
  ~10-30% full, so sessions rarely run to behavioral failure and the harvested
  corpus is occupancy-monotone (all positives fire Volume only). ADR-025 already
  names PUBLIC failure datasets as the cross-reference for the stuck/wall-hit
  tail. This requirement turns that cross-reference into an actual offline
  adapter for the Behavior family.
- Reusing `BehaviorSignals::from_signals` (rather than re-deriving the metrics in
  the calibrator) is the only way to guarantee the calibrated thresholds describe
  the SAME quantities the live verdict computes — no drift between calibration
  and runtime.
- The FILTERED "clearly stuck" label is chosen over raw `resolved == 0` because
  the plan's label-noise analysis shows `resolved == 0` mixes non-stuck failures;
  requiring `exit_status != submit` AND continued work raises label precision at
  the cost of N, matching brim's conservative posture (FP cost > FN cost for a
  recycle advisory).
- nebius is the sole source for this unit (CC-BY-4.0, ~67K, OpenHands schema with
  serialized `tool_calls` args and content-embedded error markers). Toolathlon /
  nvidia are deferred secondary sources; CAT-Instruct is unverifiable (does not
  exist) — both per ADR-034.
- Advisory-only output keeps the threshold VALUES and the ADR-025-superseding
  value-ADR in a FUTURE unit gated on the operator's real-data run; this unit
  ships the METHODOLOGY and the tool, not the numbers.

## Acceptance Criteria

- [ ] `brim calibrate --dataset <path> --format nebius` reads locally
      downloaded nebius JSONL, performs no network I/O, and never writes/mutates
      any transcript, session, or the dataset.
- [ ] Each trajectory maps to a `BehaviorSignals` via the SAME runtime
      extraction (tool-call name+args-hash vector, content-regex error flags,
      `exit_status != submit` ⇒ `stop_reason_max_tokens`).
- [ ] The positive class is the FILTERED "clearly stuck" subset
      (`exit_status != submit` AND agent kept working), not all `resolved == 0`.
- [ ] For each of `BEHAVIOR_REPETITION` / `BEHAVIOR_STREAK` /
      `BEHAVIOR_PING_PONG`, the sweep reports TP/FP/FN/precision/recall/F1 per
      `T` and a recommended `T` = lowest `T` meeting `--precision-target`
      (default 0.80), ties broken by recall; "insufficient signal, retain
      candidate" when none meet it.
- [ ] Output is a text table + `RECOMMENDED:` line and an equivalent `--json`
      form; the adapter never edits `verdict.rs` or any threshold constant.
- [ ] Content-regex runs only on downloaded dataset content; the runtime verdict
      stays structural-only.
- [ ] Calibration is deterministic; absent/empty/unparseable trajectories and
      malformed `function.arguments` are skipped, never panic.
- [ ] Only the Behavior family is calibrated; Volume excluded, Speed/Thrash
      deferred (candidate values retained); ADR-011 / ADR-020 untouched and the
      live verdict unchanged.

## Test Coverage

TEST-014 — synthetic-fixture coverage: nebius-shape extraction, filtered-stuck
label rule, precision-target sweep selection, content-regex error-marker
detection, and report shape (text + `--json`). No real dataset is vendored; the
operator downloads HF data locally for the actual run.

<!-- Realizes the PUBLIC-failure-dataset Behavior-calibration path named in
ADR-025 [[ADR-025]] and motivated by the occupancy-monotone local corpus from
the harvester REQ-017 [[REQ-017]] / ADR-033 [[ADR-033]] / STORY-013. Methodology
decisions in ADR-034 [[ADR-034]]; operator intent in STORY-014 [[STORY-014]];
boundary in FEATURE-004 [[FEATURE-004]]. lore does not support req<->req links,
so the REQ-017 relation is recorded here in prose. Threshold VALUES and the
ADR-025-superseding value-ADR are an out-of-scope FUTURE unit (post real-data
run). -->
