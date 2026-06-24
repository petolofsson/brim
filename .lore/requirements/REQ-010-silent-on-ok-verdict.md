---
id: REQ-010
title: Silent on Ok verdict
status: Draft
related_requirements:
  - FEATURE-003
related_adrs: [ADR-010]
related_stories: [STORY-011]
related_tests: []
---

# REQ-010 - Silent on Ok verdict

## Requirement

* When the parent session's own-window verdict is `ok` (ADR-010), the integration shall emit NOTHING into the conversation context — no Stop-hook `additionalContext`, no agent nudge.
* No user-facing surface shall fire beyond the ambient statusline value (the statusline always renders the current occupancy/verdict; on Ok that is its only output).
* No desktop notification shall fire on Ok.

## Rationale

The whole point of the recipe is to spend zero conversation tokens until escalation warrants them (ADR-011: advise before the hard-compaction net, but not before it matters). Ok is the steady state and must be perfectly quiet so that an injected line genuinely signals "act now."

## Acceptance Criteria

- [ ] On `ok`, the Stop hook returns no `additionalContext` (nothing enters the loop).
- [ ] On `ok`, no desktop notification is emitted.
- [ ] On `ok`, the statusline still renders the ambient occupancy/verdict value (the only surface).
