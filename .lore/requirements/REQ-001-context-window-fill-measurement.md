---
id: REQ-001
title: Context-window fill measurement
status: Draft
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-002
related_stories:
  - STORY-001
related_tests:
  - TEST-001
---

# REQ-001 - Context-window fill measurement

## Requirement

* The system shall compute each session's context-window occupancy from the latest `assistant` turn only, not from cumulative usage across turns.
* The system shall define window tokens as `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` from that turn's `message.usage`.
* The system shall resolve the context-window limit from the session's model (e.g. 200k or 1M) and express fill as a percentage bounded to [0, 100].
* The system shall read transcripts read-only and shall not load an entire transcript into memory to find the last turn.
