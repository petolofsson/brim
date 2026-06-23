---
id: FEATURE-001
title: brim context-window diagnostic
status: Accepted
related_requirements:
  - REQ-001
  - REQ-002
  - REQ-003
  - REQ-004
  - REQ-005
  - REQ-006
related_adrs:
  - ADR-001
  - ADR-002
  - ADR-003
related_stories:
  - STORY-001
related_tests:
  - TEST-001
  - TEST-002
  - TEST-003
---

# FEATURE-001 - brim context-window diagnostic

## Feature

brim is a standalone CLI that reports live context-window occupancy for AI coding-assistant sessions and their sub-agents, so an orchestrator can self-diagnose when a context has overbounded and recommend starting a fresh session.

Unlike ctop (which aggregates cumulative token spend), brim reads the point-in-time window: the latest `assistant` turn of each transcript, summing `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` and dividing by the model's context-window limit to produce a fill percentage and a verdict (ok / nearing / over -> recycle).

For Claude Code it assembles a parent -> sub-agent tree (see ADR-001); Codex and Copilot render as flat session lists. The point-in-time window read applies to Claude Code and Codex. Copilot exposes only a cumulative-spend counter (`session.shutdown` `modelMetrics.<model>.usage`), not per-turn window occupancy, so per ADR-002 Copilot sessions are discovered and listed (session id, project, recency) but report no fill % and no verdict (window unavailable).

## Scope

- CLI `brim`: default flat list of active sessions with fill % and verdict.
- `brim --tree`: orchestrator -> sub-agent tree for Claude Code sessions.
- `brim --session <id>`: scope output to one session and its sub-agents.
- `brim --once`: single plain-text snapshot (default); a watch/refresh mode may follow.
- Providers: Claude Code, Codex, Copilot — discovery for all; last-turn window read for Claude Code and Codex. Copilot is discovered and listed but reports no fill/verdict (only a cumulative counter; window unavailable per ADR-002).
- Per-model window limits (e.g. 200k / 1M) resolved from the session model.
- Machine-readable `--json` output and active/recency filtering (default active-only, `--all` for historical) for programmatic orchestration.

## Out of Scope

- Token spend, credits, daily/weekly/monthly totals — that is ctop's job.
- A TUI dashboard.
- Modifying or compacting any session; brim is read-only and advisory.
- Providers beyond Claude Code, Codex, and Copilot.
