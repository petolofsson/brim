---
id: TEST-014
title: "Calibration adapter: nebius signal extraction, filtered-stuck label, threshold sweep, content-regex, report shape on synthetic fixtures"
status: Draft
related_requirements:
  - REQ-018
related_adrs:
  - ADR-034
related_stories:
  - STORY-014
related_tests: []
---

# TEST-014 - Calibration adapter: nebius signal extraction, filtered-stuck label, threshold sweep, content-regex, report shape on synthetic fixtures

## Test Case

Unit coverage for the REQ-018 / STORY-014 offline Behavior calibration adapter
(`brim calibrate`, `src/calibrate.rs`). Deterministic, offline, no real dataset
vendored — all inputs are SYNTHETIC nebius-shape fixtures. Five groups:

1. **nebius signal extraction** — a synthetic nebius-shape trajectory (assistant
   messages with `tool_calls[].function.{name, arguments}` where `arguments` is a
   JSON STRING; `role="tool"` result messages; `exit_status`) maps to a
   `BehaviorSignals` via the runtime extraction.
2. **filtered-stuck label rule** — the positive-class filter over synthetic
   trajectories spanning `exit_status` ∈ {`submit`, non-`submit`} and
   `gen_tests_correct` ∈ {0, >0}.
3. **threshold sweep selection** — `choose_threshold` over a fixture with a known
   precision/recall profile picks the precision-target threshold.
4. **content-regex error-marker detection** — the per-tool-result error-flag
   regex over crafted content strings.
5. **report shape** — text table + `RECOMMENDED:` line and the `--json` form.

## Expected Result

(a) **nebius extraction** — the synthetic trajectory yields a `BehaviorSignals`
whose tool-call vector is `Vec<(String, u64)>` of `(function.name,
stable_hash(deserialized args))` in message order (the JSON-STRING `arguments` is
deserialized before hashing; identical args ⇒ identical hash, so a repeated
call produces a repeated entry); the error-flag vector is one `bool` per
`role="tool"` result; and `stop_reason_max_tokens == (exit_status != "submit")`.
A malformed `function.arguments` string drops that tool call without panicking;
`role="user"` messages are ignored. Signals are windowed to the last
`TREND_TAIL_K` (= 8) tool-call / error-flag entries before `from_signals`,
matching the live providers (copilot/codex) so calibrated thresholds transfer to
runtime windowed detection: a trajectory longer than the window counts ONLY the
signals within the last `K` (earlier entries are dropped, not aggregated).

(b) **filtered-stuck label** — positive (should-fire) iff `exit_status != "submit"`
AND the agent kept working (e.g. `gen_tests_correct > 0`); a `submit` /
resolved trajectory is negative; a non-`submit` trajectory with no
kept-working evidence is NOT counted positive (the filter is stricter than
`resolved == 0`).

(c) **threshold sweep** — on a fixture where, say, positives concentrate at
sub-signal value `>= 3` and negatives below it, `sweep_thresholds` reports
TP/FP/FN/precision/recall/F1 per `T`, and `choose_threshold(table, 0.80)`
returns the LOWEST `T` with precision `>= 0.80` (ties broken by higher recall).
When no `T` reaches the target the function returns `None` and the report states
"insufficient signal; retain candidate" rather than recommending a low-precision
`T`. Selection requires at least one true positive (`tp > 0`) AND
`precision >= target`: a sub-signal with NO positive evidence yields an explicit
"insufficient signal evidence — no recommendation", NOT a bogus low threshold
(e.g. `T = 1` with `recall = 0`).

(d) **content-regex** — `error_flag == true` for content containing
`[... exit code 1]` (and any `exit code N`, N >= 1), a line starting `ERROR:`,
`Traceback`, or `Exception:`; `error_flag == false` for clean output and for
`exit code 0`.

(e) **report shape** — text output contains the per-sub-signal
precision/recall/F1 rows and a single `RECOMMENDED: repetition=N streak=N
ping_pong=N` line; `--json` emits the equivalent structured records (parseable,
same numbers). Running twice on the same fixture produces identical output
(determinism). The adapter writes NO file and modifies NO `verdict.rs` constant.

Validation: `cargo test calibrate -- --nocapture` and
`cargo clippy --all-targets -- -D warnings` clean.

<!-- Covers REQ-018 [[REQ-018]] under ADR-034 [[ADR-034]]; serves STORY-014
[[STORY-014]]; within FEATURE-004. Synthetic fixtures only — no real dataset is
vendored (ADR-034); content-regex runs on fixture content only, mirroring the
downloaded-data-only boundary. Threshold VALUES are out of scope (a FUTURE unit
after the operator's real-data run). -->
