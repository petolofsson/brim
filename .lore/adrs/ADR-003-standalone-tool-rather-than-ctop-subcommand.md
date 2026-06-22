---
id: ADR-003
title: Standalone tool rather than ctop subcommand
status: Draft
related_requirements:
  - FEATURE-001
  - REQ-002
related_adrs:
  - ADR-001
  - ADR-002
related_stories: []
related_tests: []
---

# ADR-003 - Standalone tool rather than ctop subcommand

## Context

brim could have been a `ctop` subcommand, reusing its binary and provider layer. But ctop's core abstraction aggregates across turns into daily/weekly/session rollups — the opposite of the point-in-time window brim needs (see ADR-002). Threading a window-fill feature through ctop's parser -> aggregate -> pricing -> UI pipeline means fighting that abstraction.

## Decision

Ship brim as a standalone Rust binary in its own repository. Reuse only the valuable, hard-won part of ctop — multi-provider log discovery and JSONL parsing (`provider/`, `parser.rs`, `model.rs`) — adapted to emit last-turn window records. Drop ctop's pricing, aggregation, and TUI entirely.

## Consequences

* brim stays small and single-purpose; its data model is point-in-time, not cumulative.
* The lifted provider code may diverge from ctop over time; that is acceptable — it is a fork of a stable parsing layer, not a shared dependency.
* No coupling to ctop's release cadence or its credits/pricing concerns.
