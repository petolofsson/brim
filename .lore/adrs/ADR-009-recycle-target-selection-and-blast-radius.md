---
id: ADR-009
title: Recycle-target selection and blast radius
status: Accepted
related_requirements:
  - REQ-003
  - REQ-006
related_adrs:
  - ADR-001
related_stories:
  - STORY-001
related_tests: []
---

# ADR-009 - Recycle-target selection and blast radius

## Context

Hierarchy-aware diagnosis must name *which* node to recycle and what breaks if
it is. STORY-001 wants surgical, preemptive recycling — not "this branch is
unhealthy somewhere," but a specific node and the consequences of recycling it.

## Decision

When a subtree is unhealthy, recommend the **deepest, smallest node whose own
self-metrics explain the problem** — default to the degraded **leaf** (cheapest,
leaves healthy context intact). Prefer an intermediate node **only when its
self-metrics are the cause**. **Flag the root case distinctly**: recycling the
root restarts the whole operation.

The recommendation states **blast radius** = the node's descendant set, marking
which descendants are **active** (REQ-006) — i.e. the costly orphans a recycle
would discard. A degraded leaf is recyclable **independently** of its parent.
This is pure tree analysis over existing data (ADR-001 tree + REQ-006 recency);
no message content is read.

## Consequences

* Closes STORY-001 acceptance: names a specific node plus its blast radius; a
  leaf is recyclable independently.
* No effect for flat providers (single node) — the only target is itself, with
  an empty descendant set.
* Surfacing active descendants (REQ-006) makes the cost of recycling an
  intermediate node explicit, biasing toward the cheaper leaf target.

## Alternatives Considered

- **Always recommend recycling the root.** Rejected — maximal blast radius;
  throws away healthy sibling context to fix one degraded leaf.
- **Recycle the highest unhealthy subtree node.** Rejected — recycles healthy
  descendants with it; the deepest explanatory node is the surgical choice.
- **Omit blast radius / active-descendant marking.** Rejected — without it the
  orchestrator cannot weigh the cost of recycling an intermediate node.
