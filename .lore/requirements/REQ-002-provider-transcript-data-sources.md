---
id: REQ-002
title: Provider transcript data sources
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-001
  - ADR-005
  - ADR-015
related_stories:
  - STORY-001
  - STORY-003
related_tests:
  - TEST-002
---

# REQ-002 - Provider transcript data sources

## Requirement

* The system shall discover Claude Code transcripts under `~/.claude/projects/<encoded-cwd>/`, treating `<UUID>.jsonl` as sessions and `<UUID>/subagents/agent-*.jsonl` as sub-agents.
* The system shall support Codex (`~/.codex`) and Copilot (`~/.copilot`) sessions. brim treats these as flat (no sub-agent tree) by data-availability reality: Codex sub-agents are NOT linkable from the persisted rollout JSONL — `parent_thread_id` / `forked_from_id` / `agent_role` are absent from all real session files under `~/.codex/sessions/` (verified STORY-003); only `payload.source.subagent`, a free-text role string on `session_meta`, is present. brim therefore renders Codex flat; no sub-agent reconstruction is possible from the transcript.
* The system shall mark a provider unavailable (not error) when its data directory is absent.
* The system shall skip malformed or partial transcript lines without panicking.
* The system shall use only real token counters present in the transcript; no estimation.
