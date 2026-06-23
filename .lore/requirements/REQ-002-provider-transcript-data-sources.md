---
id: REQ-002
title: Provider transcript data sources
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-001
related_stories:
  - STORY-001
related_tests:
  - TEST-002
---

# REQ-002 - Provider transcript data sources

## Requirement

* The system shall discover Claude Code transcripts under `~/.claude/projects/<encoded-cwd>/`, treating `<UUID>.jsonl` as sessions and `<UUID>/subagents/agent-*.jsonl` as sub-agents.
* The system shall support Codex (`~/.codex`) and Copilot (`~/.copilot`) sessions as flat data sources.
* The system shall mark a provider unavailable (not error) when its data directory is absent.
* The system shall skip malformed or partial transcript lines without panicking.
* The system shall use only real token counters present in the transcript; no estimation.
