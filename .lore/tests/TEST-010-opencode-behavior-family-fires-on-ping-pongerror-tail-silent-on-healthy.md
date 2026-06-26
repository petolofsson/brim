---
id: TEST-010
title: Opencode Behavior family fires on ping-pong/error tail, silent on healthy
status: Accepted
related_requirements:
  - REQ-016
related_adrs:
  - ADR-024
  - ADR-028
related_stories:
  - STORY-012
related_tests: []
---

# TEST-010 - Opencode Behavior family fires on ping-pong/error tail, silent on healthy

## Test Case

Covered by `src/opencode.rs::tests` (LIVE-VERIFIED opencode 1.17.9 per ADR-028).
The extractor reads the last K = `TREND_TAIL_K` (= 8) tool rows from `part`
(`json_extract(data,'$.type')='tool'`), reversed to chronological, and feeds
`BehaviorSignals::from_signals`.

1. **Fires on same-args ping-pong.** `test_opencode_behavior_ping_pong_fires` —
   A→B→A→B with identical args at stride 2 surfaces a Behavior signal (the
   shared no-progress qualifier: `name[i]==name[i+2] && argshash[i]==argshash[i+2]`).
2. **Does NOT fire on differing-args alternation.**
   `test_opencode_behavior_ping_pong_no_fire_different_args` — A→B→A→B where the
   repeated names carry DIFFERENT args (`data.state.input`) does not trip the
   ping-pong qualifier; no signal fires.
3. **Fires on error streak.** `test_opencode_behavior_fires_on_error_streak` —
   consecutive `data.state.status='error'` rows raise the failure-streak signal.
4. **Fires on repetition.** `test_opencode_behavior_fires_on_repetition` —
   repeated identical tool call (same name + same args) raises the repetition
   signal.
5. **Silent on a healthy varied-tool session.**
   `test_opencode_behavior_no_fire_healthy` — >K varied tools (exercises the
   tail window) leave `behavior` Some-but-quiet; no signal fires.

## Expected Result

All cases pass under `cargo test opencode_behavior`. The Behavior family fires
for opencode only on same-args ping-pong, repetition, or error streak in the
recent tail — NOT on healthy varied tools or differing-args alternation. The
extractor fails closed (missing/malformed rows → `None`, no panic).
