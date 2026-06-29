---
id: ADR-034
title: "Behavior-family calibration methodology: nebius-only source, filtered-stuck label, content-regex-on-downloaded-data, synthetic-fixture tests, Speed/Thrash deferred"
status: Accepted
related_requirements:
  - REQ-018
  - REQ-017
related_adrs: []
related_stories:
  - STORY-014
related_tests:
  - TEST-014
---

# ADR-034 - Behavior-family calibration methodology: nebius-only source, filtered-stuck label, content-regex-on-downloaded-data, synthetic-fixture tests, Speed/Thrash deferred

## Context

ADR-025 made the recycle decider a >=2-of-5 FAMILY vote-counter with CANDIDATE,
uncalibrated per-family thresholds, and recorded that four families
(Volume / Speed / Thrash / Drift) have NO public dataset while Behavior CAN be
sourced from public failure trajectories. REQ-017 / ADR-033 / STORY-013 then
built the recycle-label harvester and CONFIRMED that local data cannot calibrate
Behavior: the operator recycles on occupancy at ~10-30% full, so the harvested
corpus is occupancy-monotone — the 56 recycle labels all carry `family_fires =
[true, false, false, false, false]` (only Volume fires; Speed / Thrash /
Behavior / Drift get zero local positives).

REQ-018 / STORY-014 / FEATURE-004 introduce the `brim calibrate` offline adapter
that calibrates the Behavior family from a public dataset. REQ-018 fixes WHAT the
adapter does and its invariants; this ADR records the METHODOLOGY DECISIONS made
inside that boundary, consistent with the per-feature ADR pattern (ADR-028
opencode, ADR-030 codex, ADR-032 copilot, ADR-033 harvester). It records NO
threshold numbers: the calibrated VALUES and the ADR that supersedes ADR-025's
candidate annotations are a FUTURE unit, gated on the operator's real-data run.

## Decision

**Behavior family ONLY; Volume excluded.** This unit calibrates the Behavior
sub-signals (tool-call repetition, failure streak, ping-pong). Volume is
EXCLUDED because it is locally circular — the operator recycles on occupancy, so
any occupancy-derived calibration re-learns occupancy (the harvester's
occupancy-monotone corpus is the direct evidence). Behavior is the true-onset
detector ADR-025 most wants calibrated and the only family with a viable public
source.

**Speed and Thrash DEFERRED — recorded as an explicit known gap.** No public
trajectory dataset carries per-step tokens (Speed) or cache-hit data
(`sustained_cache_thrash`, Thrash); both nebius and Toolathlon store only
aggregate token counts. Speed and Thrash therefore STAY at their ADR-025
candidate values and are NOT touched by this unit. This is logged as a known gap,
not an oversight: their calibration needs LOCAL capture (transcript `usage` +
PreCompact/PostCompact hooks), a separate future phase.

**Data source — `nebius/SWE-rebench-openhands-trajectories` ONLY (CC-BY-4.0,
~67K).** Chosen for: public + CC-BY-4.0, large N, OpenHands schema with
structured `tool_calls` (args serialized as JSON strings) and content-embedded
error markers, coding-task domain matching brim's Behavior semantics. Toolathlon
and nvidia/Open-SWE-Traces are deferred SECONDARY sources (diversity / larger
sample, different tool vocab or license-to-verify) and are not used now.
CAT-Instruct is EXCLUDED — it is unverifiable (zero results on HuggingFace /
arXiv / web) and treated as nonexistent until a citation is supplied.

**Positive label = FILTERED "clearly stuck" subset, not raw `resolved == 0`.**
The positive class is `exit_status != "submit"` AND the agent demonstrably kept
working (e.g. `gen_tests_correct > 0` yet still failed). Raw `resolved == 0`
mixes environment / localization / harness failures that are not stuck/spinning
and would inflate false positives for the Behavior sub-signals. Filtering trades
N for label precision, matching brim's conservative recycle posture (FP cost >
FN cost for an advisory).

**Content-regex on DOWNLOADED data is permitted; RUNTIME stays structural.** The
adapter regex-scans tool-result message CONTENT from the locally downloaded
dataset for error markers (`[... exit code N]`, `ERROR:`, `Traceback`,
`Exception:`). ADR-025 already draws this boundary: dataset CALIBRATION may use
content-regex while RUNTIME detection stays structural. Live Claude / codex /
opencode / copilot sessions remain STRUCTURAL-only — no natural-language content
is ever inspected at runtime. The adapter performs no network I/O and never
mutates the dataset, transcripts, or sessions (read-only).

**Reuse runtime `BehaviorSignals` extraction.** The adapter maps dataset fields
to the SAME `BehaviorSignals` (`verdict.rs`) the live verdict computes — tool-call
name+args-hash vector, per-step error-flag vector, and a `stop_reason_max_tokens`
proxy (`exit_status != "submit"`, conservative) — so calibration and runtime can
never drift. Re-deriving the metrics independently is rejected for that reason.

**Precision-target threshold sweep; advisory output.** Per sub-signal, sweep `T`
over a bounded range, count TP/FP/FN against the filtered labels, and report
precision/recall/F1. Recommend the lowest `T` meeting the precision target
(default 0.80), ties broken by recall; when none meets it, report "insufficient
signal; retain candidate" rather than recommend a weak `T`. The method is chosen
as the smallest fit (no ML infra), transparent (operator inspects the table), and
aligned with brim's conservative posture. The tool is ADVISORY: it emits a
report (text + `--json`); it NEVER auto-edits `verdict.rs`.

**Synthetic-fixture testing; no real data vendored.** Tests (TEST-014) use only
SYNTHETIC nebius-shape fixtures. No real dataset is committed to the repo
(size + keeping CI offline/deterministic); the operator downloads the HF data
locally for the actual run.

**No threshold VALUES here; ADR-025 NOT edited.** This ADR records methodology
only. The calibrated threshold VALUES and a NEW ADR superseding ADR-025's
candidate annotations come in a FUTURE unit after the real-data run. ADR-025 is
Accepted and is NOT edited; the candidate-annotation comments in `verdict.rs` are
informational, not a contract.

## Consequences

* Behavior thresholds gain a real, repeatable calibration path; Volume stays a
  weak vote (ADR-025) and is deliberately not calibrated here.
* Speed and Thrash remain at candidate values — a documented gap closed only by
  future local capture, not by this unit.
* Calibration is deterministic and offline: same downloaded files in ⇒ same
  recommendation out; CI stays green on synthetic fixtures with no data download.
* The read-only-over-sessions invariant (ADR-002 / ADR-025 / REQ-017) is intact;
  content inspection is confined to downloaded research data, never live sessions.
* Reusing `BehaviorSignals::from_signals` couples the adapter to `verdict.rs`'s
  signal shape — intentional, to prevent calibration/runtime drift.
* The filtered-stuck label reduces N; if a sub-signal has too few positives the
  sweep reports insufficiency rather than a fragile threshold, deferring to the
  retained candidate.

## Alternatives Considered

- **Calibrate from local recycle labels** — REJECTED: the harvested corpus is
  occupancy-monotone (56 labels, only Volume fires); local data cannot supply
  Behavior positives (REQ-017 circularity / data-drought, now empirical).
- **Include Volume / attempt Speed / Thrash** — REJECTED: Volume is locally
  circular; Speed/Thrash have no public per-step-token or cache-hit data
  (deferred, candidate values retained).
- **Use raw `resolved == 0` as positive** — REJECTED: mixes non-stuck failures,
  inflating FP; the filtered "clearly stuck" subset favors precision.
- **Add Toolathlon / nvidia / CAT-Instruct now** — REJECTED for this unit:
  Toolathlon/nvidia are deferred secondary; CAT-Instruct is unverifiable.
- **Vendor a real-data sample for tests** — REJECTED: keep the repo data-free and
  CI deterministic; synthetic fixtures suffice.
- **Auto-apply recommended thresholds to `verdict.rs`** — REJECTED: advisory only;
  the operator applies values, and a future ADR records them.
- **Edit ADR-025's candidate annotations** — REJECTED: ADR-025 is Accepted; a
  future value-ADR supersedes it instead.

<!-- lore does not support adr<->adr links, so the relations to ADR-025
(vote-counter / Behavior signals / content-regex-for-calibration boundary;
superseded in a FUTURE value-ADR, not here) and ADR-033 (harvester that produced
the occupancy-monotone local corpus) are recorded in the prose above. Linked via
the CLI: REQ-018, REQ-017, STORY-014, TEST-014. Builds on FEATURE-004. No
threshold VALUES recorded here — a FUTURE unit after the operator's real-data
run. -->
