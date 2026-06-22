---
id: ADR-001
title: Sub-agent tree from Claude transcript layout
status: Draft
related_requirements:
  - FEATURE-001
  - REQ-002
  - REQ-003
related_adrs:
  - ADR-002
  - ADR-003
related_stories: []
related_tests:
  - TEST-002
---

# ADR-001 - Sub-agent tree from Claude transcript layout

## Context

brim's headline feature is `--tree`: an orchestrator inspecting the context-window fill of itself and every sub-agent it spawned. This only works if the provider logs encode an explicit parent -> child link. A spike against real Claude Code transcripts under `~/.claude/projects/` confirmed the linkage is explicit and unambiguous.

Claude Code lays sessions out as:

```
~/.claude/projects/<encoded-cwd>/
├── <PARENT-UUID>.jsonl                 # orchestrator session (main transcript)
└── <PARENT-UUID>/subagents/
    ├── agent-<agentId>.jsonl           # sub-agent
    └── agent-<agentId>.jsonl
```

Observed on a real session (`77508558-...`, 6 sub-agents):

- The parent session UUID is the join key. It is both the directory name *and* the `sessionId` field inside every sub-agent file — two independent, corroborating signals.
- Sub-agent entries are marked `isSidechain: true`; each carries its own `agentId`.
- The parent's main `.jsonl` sits as a sibling file next to the `<PARENT-UUID>/` directory.
- Per-turn window size comes from the latest `assistant` event's `message.usage`: `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`. Each file (parent and sub-agent) has its own context window, computed from its own last turn.

Codex (`~/.codex`) and Copilot (`~/.copilot`) do not spawn sub-agents this way; they expose flat sessions only.

## Decision

Source the orchestrator -> sub-agent tree from Claude Code's transcript directory layout, joining on the parent session UUID. The Claude provider yields, per transcript file, a record of `(session_id, parent_uuid | None, agent_id | None, last_turn_window)`:

- Parents: discovered as `<cwd>/<UUID>.jsonl`, with `parent_uuid = None`.
- Children: discovered as `<cwd>/<UUID>/subagents/agent-*.jsonl`, with `parent_uuid = <UUID>` and `agent_id = <agentId>`.

The tree assembles purely from the parent-UUID join — no heuristics. Non-Claude providers return flat records with `parent_uuid = None`, so `--tree` degrades cleanly to a flat list per provider without breaking any contract.

## Consequences

* `--tree` is feasible as a first-class feature, not a best-effort fallback.
* Window fill is computed independently per file, so a sub-agent can register "at the brim" while its parent is not — exactly the orchestrator signal brim exists to surface.
* The directory layout and `subagents/` convention are local implementation details of Claude Code and may change in future releases; the provider treats a missing `subagents/` directory as "no children", not an error.
* `isSidechain` and `sessionId` give a redundant cross-check on the path-based join — useful for validation, not required.

## Alternatives Considered

- **Heuristic linkage (shared cwd + timing windows).** Rejected: unnecessary, since explicit linkage exists; would produce false edges.
- **Flat session list only (no tree).** Rejected as the default: discards the parent/child relationship that is the core value for orchestrators. Retained only as the per-provider degradation path for Codex/Copilot.
