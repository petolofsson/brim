---
id: TEST-011
title: Codex Behavior + occupancy expected outcomes (live-verified shapes)
status: Accepted
related_requirements: []
related_adrs: [ADR-030]
related_stories:
  - STORY-012
related_tests: []
---

# TEST-011 - Codex Behavior + occupancy expected outcomes (live-verified shapes)

## Test Case

Expected outcomes for the live-verified codex 0.142.2 parsers (ADR-030),
exercised over real-schema JSONL row shapes:

1. **Occupancy from `token_count`** — a `payload.type == "token_count"` row with
   `info.total_token_usage.{input_tokens=11975, cached_input_tokens=9088}` on an
   `event_msg` wrapper.
2. **Behavior fires on real payload-item `function_call`** — a single
   `payload.type == "function_call"` row is detected as a tool call.
3. **Repetition on real payload-item repetition** — 6 identical real-schema
   `function_call` rows (mirrors the captured session's 6 calls).
4. **`custom_tool_call` counted** — 2 identical `apply_patch` `custom_tool_call`
   rows (args in `payload.input`, not `arguments`).
5. **project_key from `payload.cwd`** — a `session_meta` row with
   `payload.cwd = "/home/pol/code/myproject"`.
6. **Failure streak from `failed` status (SYNTHETIC)** — 3 consecutive
   `function_call_output` rows with `status == "failed"`; control of 3
   `completed` rows must NOT produce a streak.

## Expected Result

- (1) `extract_window` → `window_tokens == 11_975` (= `input_tokens`, includes
  cached), `WindowSource::Aggregate`.
- (2) `extract_codex_behavior` returns `Some`; single call → `repetition_run`
  is `None` (threshold ≥2); `completed` output → `failure_streak` is `None`.
- (3) `repetition_run == Some(6)`.
- (4) `repetition_run == Some(2)`.
- (5) `project_key == "myproject"` (non-empty).
- (6) `failure_streak == Some(3)` for the failed tail; `None` for the completed
  control. **Synthetic-validated only — real-failure codex data PENDING.**
- Behavior stays silent (`None`) on a healthy session (no repetition, no
  failure streak).

## Implementing Tests

`src/codex.rs` (`cargo test`, 19/19 codex tests passing):

- `test_codex_token_count_real_event_msg_shape` — (1)
- `test_codex_behavior_real_payload_function_call_detected` — (2)
- `test_codex_behavior_real_payload_repetition_6x` — (3)
- `test_codex_behavior_custom_tool_call_counted` — (4)
- `test_codex_project_key_from_payload_cwd` — (5)
- `test_codex_behavior_failed_output_failure_streak` — (6, synthetic)
