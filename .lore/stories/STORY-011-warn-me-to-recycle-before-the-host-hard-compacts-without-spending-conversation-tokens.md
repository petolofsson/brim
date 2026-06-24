---
id: STORY-011
title: Warn me to recycle before the host hard-compacts, without spending conversation tokens
status: Draft
related_requirements:
  - FEATURE-003
related_adrs: [ADR-011]
related_stories: []
related_tests: []
---

# STORY-011 - Warn me to recycle before the host hard-compacts, without spending conversation tokens

## User Story

As an agent operator,
I want to be warned to wrap up and recycle the session before the host hard-compacts it,
So that I act ahead of the auto-compaction net (ADR-011) without spending conversation tokens on the check itself.

## Acceptance Criteria

- [ ] While the session is healthy (Ok), I see only the ambient statusline value and the loop stays silent (REQ-010).
- [ ] Nearing surfaces on the statusline only; no in-loop injection (FEATURE-003).
- [ ] When the session crosses into Over, I get one desktop notification and the agent gets one in-loop nudge to recycle — once per escalation, not every turn (FEATURE-003).
- [ ] The advisory is scoped to my parent session's own window, so heavy sub-agent calls do not false-alarm (FEATURE-003).
- [ ] The check runs entirely host-side; brim's JSON never enters my conversation, so the warning costs no context (FEATURE-003).
- [ ] If brim is missing or errors, my turn proceeds unaffected — nothing is emitted and nothing is blocked (REQ-015).
