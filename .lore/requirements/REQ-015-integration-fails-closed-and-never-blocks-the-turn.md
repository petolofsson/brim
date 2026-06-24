---
id: REQ-015
title: Integration fails closed and never blocks the turn
status: Draft
related_requirements:
  - FEATURE-003
related_adrs: []
related_stories: [STORY-011]
related_tests: []
---

# REQ-015 - Integration fails closed and never blocks the turn

## Requirement

* On any brim error, non-zero exit, JSON parse failure, missing/unmatched session, or unavailable window, the integration shall emit NOTHING (no nudge, no notification) and shall fall through silently.
* The Stop hook shall ALWAYS exit 0 and shall never block, delay, or fail the turn on account of the brim check.
* The statusline command shall degrade to an empty/neutral value on failure rather than erroring.

## Rationale

The advisory is best-effort and strictly additive. A health check must never become a liability: if brim is missing, slow, or returns something unparseable, the agent loop must proceed exactly as if the integration were absent. Fail-closed (emit nothing) is safer than fail-open (risk spurious nudges) and guarantees the recipe can never wedge a turn.

## Acceptance Criteria

- [ ] When brim exits non-zero or is not installed, the hook emits nothing and exits 0.
- [ ] When brim's output fails to parse, the hook emits nothing and exits 0.
- [ ] When `--session <id>` matches no session, the hook emits nothing and exits 0.
- [ ] The Stop hook never returns a blocking/deny decision based on the brim check.
- [ ] The statusline command renders empty/neutral (not an error) on any failure.
