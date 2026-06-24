---
id: STORY-004
title: Compact brim --json diagnostic output
status: Accepted
related_requirements:
  - REQ-005
  - FEATURE-001
related_adrs: [ADR-013]
related_stories: []
related_tests: []
---

# STORY-004 - Compact brim --json diagnostic output

> **Status: Accepted — shipped 2026-06-24 via ADR-013 (55.3% output reduction).**
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

In the `src/output.rs` JSON path (`JsonNode`, `JsonSubtreeInfo`, `JsonRecycleRecommendation`):

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

- [x] `brim --json` output is materially smaller than the current ~2000-token baseline.
- [x] The largest contributor (`trend.points`) is capped, dropped, or gated behind a flag.
- [x] The slimmed schema stays coherent with REQ-005 (or REQ-005 is superseded by a follow-up).
- [x] Field-name / null-omission / duplication changes preserve a valid, parseable tree.

## Shipped

STORY-004 shipped via ADR-013: dropped `trend.points` + `generated_at`,
shortened nested keys (`velocity`, `proj_turns`, `subtree_tokens`,
`worst_node`, `worst_proj`, `worst_proj_node`, `target`, `node`), and applied
`#[serde(skip_serializing_if = "Option::is_none")]` to nullable fields.
Measured on the live tree (opencode.db, 179 nested nodes): 155,304 bytes ->
69,497 bytes (55.3% reduction); ADR-013's 76-session spec baseline measured
109,929 -> ~54,638 bytes (50.3%). REQ-005 AC and TEST-005 updated to match.
Tests: 116/116 green; `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings` clean.

## Related

- REQ-005 (Machine-readable diagnostic output) — the contract this affects.
- FEATURE-001 (brim context-window diagnostic) — intended sibling feature relation
  (recorded under `related_requirements` per this repo's story convention).
