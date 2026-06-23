---
id: ADR-007
title: Subtree health aggregation over the agent tree
status: Accepted
related_requirements:
  - REQ-003
  - REQ-007
related_adrs:
  - ADR-001
related_stories:
  - STORY-001
related_tests: []
---

# ADR-007 - Subtree health aggregation over the agent tree

## Context

REQ-003 and ADR-001 build a parent→child agent tree, but brim surfaces only the
per-node window. STORY-001's orchestrator needs one glanceable number for the
whole branch: a parent can be self-healthy while its subtree degrades, and
scanning every node by hand defeats the point of self-diagnosis.

## Decision

Each node carries two readings — **self** (its own window) and **subtree**
(the node plus all descendants). Aggregation rules differ per metric because the
metrics do not share semantics:

* **Token fill** does **not** sum to a single ratio (each window is independent)
  — report **total subtree tokens + worst-child fill**.
* **Growth** → the **worst (fastest) descendant** sets the branch deadline.
* **Cost** → **sum** across the subtree.
* **Degradation / loop flags** (where such signals exist) **propagate upward**,
  naming the offending node.

The root's subtree score is the **top-line glanceable value**; all nodes are
rankable worst-first. The aggregation is derived, deterministic, bounded by the
existing tree caps (64/256 per ADR-001/REQ-003), introduces no new data source,
and reads no message content.

## Consequences

* Closes STORY-001 acceptance: root subtree score as the top-line value, nodes
  rankable worst-first.
* Flat-provider sessions (Codex, Copilot) have a single-node subtree, so
  subtree == self with no special-casing.
* The per-metric rules keep the aggregate faithful — independent windows are not
  averaged into a meaningless single ratio.
* **Cost-sum rule is N/A / not implemented** for brim's current data model: brim
  reads point-in-time window occupancy, not aggregate spend (ADR-002), so no
  cost/spend source exists to sum. The Decision's cost rule is retained as intent
  but is inert until a spend source is ever added; revisit then.

## Alternatives Considered

- **Average fill across the subtree.** Rejected — windows are independent and
  differently sized; an average hides the one node that is actually full.
- **Sum growth across descendants.** Rejected — growth is a rate per window;
  the branch deadline is set by the fastest single descendant, not the sum.
- **One blended health scalar per node.** Rejected — collapses distinct signals
  (fill, growth, cost) that the orchestrator needs to act on separately.
