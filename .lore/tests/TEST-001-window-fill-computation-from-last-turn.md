---
id: TEST-001
title: Window-fill computation from last turn
status: Draft
related_requirements:
  - FEATURE-001
  - REQ-001
related_adrs:
  - ADR-002
related_stories:
  - STORY-001
related_tests: []
---

# TEST-001 - Window-fill computation from last turn

## Test Case

Given a Claude Code transcript whose final `assistant` event has `message.usage` of `input_tokens=7000, cache_read_input_tokens=130000, cache_creation_input_tokens=5000` and a `message.model` resolving to a 200k window,

When brim computes that session's window fill,

Then it shall report window tokens = 142000 and fill = 71% (bounded to [0, 100]), using only the last turn and ignoring earlier turns.
