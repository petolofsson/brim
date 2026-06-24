---
id: TEST-008
title: Copilot CompactionProcessor line parsing
status: Accepted
related_requirements:
  - REQ-009
related_adrs: [ADR-002, ADR-011]
related_stories: []
related_tests: []
---

# TEST-008 - Copilot CompactionProcessor line parsing

## Test Case

Covered by `src/copilot.rs::tests` (VERIFIED-LIVE per REQ-009 body).

1. **CompactionProcessor line parses `<used>`.** `test_parse_compaction_line_ok`
   — `92340/204800 tokens` → used = 92340.
2. **`<limit>`, `<pct>`, `<thresh>` ignored** (absolute-only, ADR-011) —
   `test_parse_compaction_line_limit_pct_ignored`.
3. **Malformed lines skipped, no panic** —
   `test_parse_compaction_line_malformed_skipped` (bad ts, non-numeric used).
4. **Multi-line trend from CompactionProcessor lines.**
   `test_extract_compaction_points_multi_line_trend`.
5. **Non-CompactionProcessor lines skipped.**
   `test_extract_compaction_points_non_compaction_lines_skipped`.
6. **Missing `inuse.<pid>.lock` → no window.**
   `test_process_log_occupancy_missing_lock_returns_none`; missing log file →
   `test_process_log_occupancy_missing_log_returns_none`.
7. **Process log produces a window + trend.**
   `test_process_log_produces_window_and_trend` seeds an `inuse.<pid>.lock`
   and three CompactionProcessor lines and asserts occupancy + trend.

## Expected Result

All cases pass under `cargo test copilot`. `<used>` is the point-in-time
occupancy; `<limit>`/`<pct>`/`<thresh>` never influence the verdict.
