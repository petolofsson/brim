---
id: ADR-029
title: brim carries no per-model context-window limit; recycle uses absolute token bands (supersedes ADR-015)
status: Accepted
related_requirements:
  - REQ-008
related_adrs:
  - ADR-011
  - ADR-020
  - ADR-015
  - ADR-005
related_stories: []
related_tests: []
---

# ADR-029 - brim carries no per-model context-window limit; recycle uses absolute token bands (supersedes ADR-015)

> Supersedes the per-model context-window limit decisions of **ADR-015**
> (`z-ai/glm-5.2` = 1,000,000; 200k default for any other model). ADR-015 in turn
> superseded the 200k limit assumption of **ADR-005** (lineage: ADR-005 → ADR-015 →
> ADR-029). ADR-015 stays **Accepted**; this ADR records the supersession in prose
> (matching how ADR-026/ADR-027 recorded theirs). The absolute-token architecture of
> **ADR-011** (drop advertised-window apparatus) and its narrowed rationale in
> **ADR-020** (absolute-token premise holds for agentic/reasoning context) are the
> governing principles and are unchanged.

## Context

ADR-015 fixed a per-model context-window limit for opencode (`z-ai/glm-5.2` =
1,000,000, 200k default otherwise) so occupancy verdicts could reason against the
true advertised window. ADR-011 had already removed the advertised-window apparatus
from the recycle decision; ADR-015's limit therefore described a value the verdict
path no longer consumes.

A read-only trace of the current source confirms the limit is **vestigial** — no
model-to-context-window-limit value reaches brim's verdict or output:

- The recycle verdict bands are **absolute, model-independent constants** in
  `verdict.rs`: `ABSOLUTE_WATCH_TOKENS = 32_000` (watch) and
  `ABSOLUTE_RECYCLE_BACKSTOP = 128_000` (recycle backstop). They never consult an
  advertised window.
- Model id is **display-only**; the `[1m]` marker is **absent from all of `src/`**,
  so the bare id cannot recover a resolved window even if something wanted to.
- There is **no model→limit registry or function** anywhere in `src/`.
- The copilot provider **parses a limit field but explicitly ignores it** — it does
  not feed the verdict.

So ADR-015's decision (and REQ-008's "model context-window limit" clause) describe
behavior the code no longer has.

## Decision

1. brim carries **NO per-model context-window limit**. There is no model→limit
   registry, and no advertised window is consulted by the recycle verdict.
2. The recycle verdict reasons in **absolute token bands** — `32_000` watch /
   `128_000` backstop (`verdict.rs`), independent of any model's advertised window
   (per ADR-011 / ADR-020).
3. **Model id is display-only.** The `[1m]` marker plays no role; it is absent from
   `src/`. The copilot-parsed limit field is read but not consumed.
4. ADR-015 (and the ADR-005 → ADR-015 limit lineage) is **retired as vestigial**.
   REQ-008's "model context-window limit" bullet is amended to match.

## Consequences

The audit trail now states plainly that brim consumes no per-model window. No
per-provider limit table needs maintaining: opencode alone exposes ~40 providers /
250+ models and grows fast with **no automatic limit source**, so a per-model window
registry is unmaintainable — absolute-token reasoning (ADR-011 / ADR-020) is the
deliberate architecture that makes the registry unnecessary. A consumer that wants a
fill percentage divides absolute occupancy by a window it chooses (ADR-011); brim
does not own that window.

## Alternatives Considered

- **Edit ADR-015 in place / mark it Superseded.** Rejected — ADR-015 is Accepted, and
  project policy never edits an Accepted ADR; supersession is recorded in this ADR's
  prose, matching ADR-026/ADR-027.
- **Reintroduce a model→limit registry.** Rejected — the trace shows nothing reads a
  limit, and the opencode model surface (~250+ models, no automatic limit source)
  makes a registry pure unmaintained surface area. ADR-011/ADR-020 absolute-token
  reasoning is the standing decision.
