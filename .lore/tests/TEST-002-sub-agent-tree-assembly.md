---
id: TEST-002
title: Sub-agent tree assembly
status: Draft
related_requirements:
  - FEATURE-001
  - REQ-002
  - REQ-003
related_adrs:
  - ADR-001
related_stories:
  - STORY-001
related_tests: []
---

# TEST-002 - Sub-agent tree assembly

## Test Case

Given a Claude Code project directory containing `<PARENT-UUID>.jsonl` plus `<PARENT-UUID>/subagents/agent-A.jsonl` and `agent-B.jsonl`,

When brim assembles the tree with `--tree`,

Then the parent node shall list both sub-agents as children joined on the parent UUID, each child shall carry its own `agent_id` and its own independently-computed window fill, and a session with no `subagents/` directory shall render as a childless node rather than an error.
