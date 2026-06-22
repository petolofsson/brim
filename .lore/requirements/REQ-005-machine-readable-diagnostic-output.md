---
id: REQ-005
title: Machine-readable diagnostic output
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs: [ADR-004]
related_stories: []
related_tests: []
---

# REQ-005 - Machine-readable diagnostic output

## Requirement

* The system shall provide a `--json` flag emitting the full session/sub-agent set as structured JSON to stdout; human-readable text remains the default.
* Each JSON node shall carry: full session id (untruncated UUID), parent session id (null for roots), agent id (null for non-sub-agents), project key, model, resolved context-window limit, window tokens (`input_tokens + cache_read_input_tokens + cache_creation_input_tokens`), fill percent bounded to [0, 100], and verdict (`ok | nearing | over_recycle`).
* The JSON shall preserve the parent -> child tree (nodes nested under their parent, or carrying an explicit parent id) so an orchestrator can act on individual sub-agents.
* The JSON schema shall be stable and documented; field names machine-stable (snake_case), verdict as an enumerated string.
* Security: JSON shall contain only ids, project name, model, counts, percentages, verdicts and timestamps — never transcript content or prompts (CODERULES r11). `--json` exposes FULL ids by design (needed to act on a node); the human-readable view keeps truncating ids.

## Rationale

brim today emits human plain-text only, with truncated ids. An orchestrator cannot reliably parse columns nor address a specific session/sub-agent. A stable `--json` contract with full ids lets the orchestrator programmatically read window state and name which nodes to recycle, while the default text view stays human-first.

## Acceptance Criteria

- [ ] `brim --json` emits valid JSON to stdout; without it the default output stays plain text.
- [ ] Each node exposes the full untruncated session id, parent id, agent id, project, model, limit, window tokens, fill percent, and verdict.
- [ ] The JSON preserves the parent -> child tree (nesting or explicit parent id).
- [ ] Field names are snake_case and the verdict is one of `ok | nearing | over_recycle`.
- [ ] The JSON never contains transcript content or prompts.
