---
id: REQ-003
title: Orchestrator sub-agent tree
status: Draft
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-001
related_stories:
  - STORY-001
related_tests:
  - TEST-002
---

# REQ-003 - Orchestrator sub-agent tree

## Requirement

* The system shall assemble a parent -> child tree for Claude Code by joining sub-agent files to their parent on the parent session UUID (see ADR-001).
* The system shall compute window fill independently for each node, so a sub-agent may report a different fill than its parent.
* The system shall render non-Claude providers as flat nodes with no parent.
* `brim --session <id>` shall show the named session and its sub-agents only.
