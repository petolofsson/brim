---
id: TEST-006
title: active filter and recency threshold behavior
status: Accepted
related_requirements:
  - REQ-006
related_adrs: [ADR-016]
related_stories: []
related_tests: []
---

# TEST-006 - active filter and recency threshold behavior

## Test Case

Covered by `src/main.rs::tests`.

1. **Recent timestamp is active.** `test_recency_active_recent_timestamp` — a
   node with `last_turn_at` 10 min ago is active under the 30-min threshold.
2. **Old timestamp is inactive.** `test_recency_inactive_old_timestamp` — a
   node 60 min ago is inactive under the default 30-min threshold.
3. **Missing timestamp is inactive.** `test_recency_inactive_missing_timestamp`
   — `last_turn_at = None` → inactive (never panics).
4. **Default filter excludes stale, includes active** —
   `test_default_filter_excludes_stale_includes_active` retains only the active
   session; with `--all` both are kept.
5. **Stale parent with an active child is retained** by `any_active` —
   `test_default_filter_retains_stale_parent_with_active_child`.
6. **`--session` bypasses the active filter** —
   `test_session_flag_bypasses_active_filter`.
7. **`--json` emits `last_turn_at`/`active`** — verified across the JSON
   tests (`test_json_full_id_nested_children_no_transcript_content`,
   `test_json_slim_contract_adr013`).

## Expected Result

All cases pass under `cargo test main` (and the full suite remains green).
Active label is advisory; no transcript is mutated.
