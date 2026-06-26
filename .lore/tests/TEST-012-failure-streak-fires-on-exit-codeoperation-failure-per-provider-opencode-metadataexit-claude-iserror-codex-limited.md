---
id: TEST-012
title: Failure-streak fires on exit-code/operation failure per provider (opencode metadata.exit, claude is_error, codex limited)
status: Accepted
related_requirements:
  - REQ-016
related_adrs: [ADR-031, ADR-024]
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

## Expected Result

All four pass under `cargo test`. The failure-streak fires for opencode on
`state.metadata.exit != 0` (and still on `status=='error'`) and for claude on
`is_error`, but provably CANNOT fire for real codex sessions — there is no structural
exit field to read. Behavior remains fail-closed (missing/malformed fields → no error
flag, no panic).
