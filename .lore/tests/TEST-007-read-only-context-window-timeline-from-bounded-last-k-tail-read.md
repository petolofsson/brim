---
id: TEST-007
title: Read-only context-window timeline from bounded last-K tail read
status: Accepted
related_requirements:
  - REQ-007
related_adrs: [ADR-006, ADR-004]
related_stories: []
related_tests: []
---

# TEST-007 - Read-only context-window timeline from bounded last-K tail read

## Test Case

Covered by `src/window.rs::tests` (`compute_trend`, `TREND_TAIL_K`) and
`src/claude.rs::tests` (`parse_transcript` trend branch).

1. **Simple growth → median velocity + projection.**
   `test_velocity_simple_growth` — points [10k,20k,30k] → velocity 10k/turn;
   projection (128k−30k)/10k = 9 turns.
2. **Reset boundary is detected.** `test_velocity_across_reset` — pre-reset
   segment discarded; velocity from post-reset tail.
3. **<2 post-reset points → no velocity.**
   `test_fewer_than_2_post_reset_points_no_velocity`,
   `test_single_point_no_velocity`.
4. **Projection targets the absolute backstop, not an advertised window.**
   `test_projection_targets_backstop_not_window` and
   `test_projection_past_backstop_yields_zero`.
5. **No positive deltas → no velocity.** `test_no_positive_deltas_no_velocity`.
6. **Bounded tail read.** `src/claude.rs:127-128` caps the kept turns at
   `TREND_TAIL_K`; `test_trend_built_from_multiple_turns` confirms a 2-turn
   trajectory yields 2 points with velocity 20k and projection 2.
7. **Turns without timestamps are skipped.**
   `test_trend_excludes_turns_without_timestamps` (no timeline points → no trend).

## Expected Result

All cases pass under `cargo test window` and `cargo test claude`. brim emits
`--json` `trend.velocity`/`trend.proj_turns` and never persists the timeline.
