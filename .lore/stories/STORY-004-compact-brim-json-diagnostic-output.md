---
id: STORY-004
title: Compact brim --json diagnostic output
status: Draft
related_requirements:
  - REQ-005
  - FEATURE-001
related_adrs: []
related_stories: []
related_tests: []
---

# STORY-004 - Compact brim --json diagnostic output

> **Status: Draft — deferred future-work, scheduled 2026-06-24.**
> Out of scope for the completed 4-increment build; recorded now so the lever is not lost.

## User Story

As an orchestrator agent consuming brim to self-diagnose context health,
I want the `--json` output to be compact,
So that the diagnostic does not spend the very context budget it exists to protect.

## Motivation

Current `brim --json` output is ~2000 tokens per invocation — too large. An orchestrator
that polls brim to decide when to recycle pays this cost on every check, so the diagnostic
consumes the context it is meant to safeguard. This conflicts directly with the project's
low-tokens goal and undermines brim's core purpose.

## Scope / candidate levers

In the `src/main.rs` JSON path (`JsonNode`, `JsonSubtreeInfo`, `JsonRecycleRecommendation`):

- The per-turn `trend.points` array is the largest contributor — cap/drop it or make the
  timeline opt-in via a flag (e.g. `--json-timeline`).
- Shorten field names.
- Omit null/default fields.
- Collapse redundant self-vs-subtree duplication for flat single-node sessions.

## Constraint

Keep the REQ-005 machine-readable contract coherent. The slimmed schema likely needs a
follow-up Draft REQ or ADR to redefine the stable field set — **noted here as a dependency,
not created now.**

## Acceptance Criteria

- [ ] `brim --json` output is materially smaller than the current ~2000-token baseline.
- [ ] The largest contributor (`trend.points`) is capped, dropped, or gated behind a flag.
- [ ] The slimmed schema stays coherent with REQ-005 (or REQ-005 is superseded by a follow-up).
- [ ] Field-name / null-omission / duplication changes preserve a valid, parseable tree.

## Related

- REQ-005 (Machine-readable diagnostic output) — the contract this affects.
- FEATURE-001 (brim context-window diagnostic) — intended sibling feature relation
  (recorded under `related_requirements` per this repo's story convention).
