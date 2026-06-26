---
id: TEST-012
title: Failure-streak fires on exit-code/operation failure per provider (opencode metadata.exit, claude is_error, codex limited)
status: Accepted
related_requirements:
  - REQ-016
related_adrs: [ADR-031, ADR-024, ADR-032]
related_stories:
  - STORY-012
related_tests: []
---

# TEST-012 - Failure-streak fires on exit-code/operation failure per provider (opencode metadata.exit, claude is_error, codex limited)

## Test Case

Covers the failure-streak signal redesign (ADR-031): the streak keys on
operation/exit-code failure where structurally available. Tests live in each
provider's `tests` module against LIVE-VERIFIED row shapes.

1. **opencode — non-zero exit fires.**
   `src/opencode.rs::test_opencode_behavior_metadata_exit_nonzero_fires` — two bash
   tool rows with `state.status == 'completed'` but `state.metadata.exit == 1`
   (the real failed-command shape) raise `failure_streak == Some(2)`.
2. **opencode — zero exit is silent (control).**
   `src/opencode.rs::test_opencode_behavior_metadata_exit_zero_no_fire` — a row with
   `metadata.exit == 0` raises no error flag; `failure_streak` is `None`.
3. **claude — real bash exit shape fires.**
   `src/claude.rs::test_behavior_failure_streak_claude_real_bash_exit_shape` — two
   `tool_result` rows with `is_error == true` and content "Exit code N" (real
   `~/.claude` shape) raise `failure_streak == Some(2)`. No code change — `is_error`
   already reflects exit-code failure.
4. **codex — real shape cannot fire (documented limitation).**
   `src/codex.rs::test_codex_behavior_real_shape_no_status_no_error_flag` — real
   `function_call_output` rows (keys `call_id`, `output`, `type` — NO `status` field)
   leave `failure_streak` `None`; the exit code is only in the free-text `output`,
   which r11 forbids inspecting.
5. **copilot — failing command fires (ADR-032).**
   `src/copilot.rs::test_copilot_behavior_exit_code_nonzero_triggers_failure` — a
   `tool.execution_start`/`tool.execution_complete` pair from real
   `events.jsonl` where the completion carries `data.success == true` but
   `data.toolTelemetry.metrics.exit_code == 1` raises `failure_streak == Some(1)`.
   Mirrors opencode's exit-code rule per ADR-031: the streak keys on operation/exit-code
   failure, so a non-zero exit fires even though the tool invocation "succeeded."

## Expected Result

All five pass under `cargo test`. The failure-streak fires for opencode on
`state.metadata.exit != 0` (and still on `status=='error'`), for claude on
`is_error`, and for copilot on `success==false OR toolTelemetry.metrics.exit_code != 0`
(ADR-032), but provably CANNOT fire for real codex sessions — there is no structural
exit field to read. Behavior remains fail-closed (missing/malformed fields → no error
flag, no panic).
