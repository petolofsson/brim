---
id: STORY-003
title: Verify provider data semantics against official docs and real sessions
status: Draft
related_requirements:
  - REQ-002
related_adrs:
  - ADR-002
  - ADR-005
  - ADR-006
related_stories: []
related_tests: []
---

# STORY-003 - Verify provider data semantics against official docs and real sessions

> **Status: Draft — deferred / future work.** Out of scope for the current build
> increments; sequenced to run after they land.

## User Story

As a brim maintainer,
I want each provider's transcript/usage semantics verified against **official CLI
docs and real captured sessions** (not synthetic fixtures + inference),
So that brim's per-provider window and trend rest on ground truth.

## Motivation

brim's provider model currently relies partly on synthetic ctop fixtures and
reasoning. This proved fragile:

- A blocking review claim that **Codex lacks point-in-time occupancy was wrong** —
  the delta of cumulative `total_token_usage` *is* per-turn occupancy.
- **Copilot's per-turn data was found to exist in process logs**, but is absent
  from the persisted transcript brim reads.

A docs-grounded pass converts assumptions into verified facts and will likely
surface more oracles and bugs.

## Scope — for EACH of the four providers (Claude Code, OpenCode, Codex, Copilot)

- Transcript location + format; which source is authoritative.
- Per-turn fields: does it expose **point-in-time occupancy**, or only cumulative
  spend?
- Cache split availability (read / creation).
- Sub-agent / tree linkage mechanism.
- Known data-quality wrinkles (e.g. Codex rate-limit duplicate `token_count`
  events #14489; Copilot ephemeral usage / process-log `CompactionProcessor`
  oracle).
- Per-model context-window limits.

## Deliverable + dependency

- **Deliverable:** a verified provider capability matrix that hardens REQ-002 and
  confirms/corrects ADR-002 / ADR-005 / ADR-006; may spawn per-provider TEST
  artifacts (parity against real transcripts).
- **Dependency:** the ground-truth half needs **real captured sessions** from each
  CLI (only synthetic fixtures exist today). Docs are checkable immediately;
  real-session parity follows when sessions can be captured.

## Why deferred

Out of scope for the current build increments; sequenced after they land.

## Acceptance Criteria

- [ ] Provider capability matrix produced, each cell sourced to official docs or a
      real captured session (no inference-only cells).
- [ ] ADR-002 / ADR-005 / ADR-006 each confirmed or flagged for correction against
      the verified matrix.
- [ ] REQ-002 reconciled with verified provider data sources.
- [ ] Data-quality wrinkles per provider documented (incl. Codex #14489, Copilot
      process-log oracle).
