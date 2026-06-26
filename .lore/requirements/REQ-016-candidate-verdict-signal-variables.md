---
id: REQ-016
title: Candidate verdict-signal variables
status: Accepted
related_requirements: [FEATURE-001]
related_adrs:
  - ADR-010
related_stories: [STORY-012]
related_tests: []
---

# REQ-016 - Candidate verdict-signal variables

## Requirement

This requirement catalogs the CANDIDATE signal set for a future, sharper
recycle verdict (the realization of ADR-010 signal (a)). It is a planning
artifact: it names candidate variables and their intended use, it does NOT
mandate thresholds or an engine change. Anchors are candidates pending
calibration (recycle-verdict-model.md Roadmap #3); none is asserted as
accepted fact.

Each candidate is recorded as: **source field**, **what it predicts**,
**intended verdict use**, **cross-provider availability**.

### Tier A — transcript-derivable, cross-provider, cheap; KEEPS the invariant

Tier A signals preserve the deterministic / transcript-tokens-only invariant
(no tool-block parsing, no content inspection). They sharpen the existing
absolute-token verdict and split the saturated top stages.

| variable | source field | predicts | intended verdict use | cross-provider |
|---|---|---|---|---|
| output_tokens | `usage` block (all providers) | next-turn occupancy | +1-turn lead on `w` / velocity `v` | all providers |
| turn count `m` since reset | transcript line count | bloat independent of token volume | second `nearing` gate | all providers |
| wall-clock velocity (tokens/min) | turn timestamps (already kept) | burst vs slow drift | split `stale` vs `critical` by rate (closes the stage-4/5 pure-occupancy blindness) | all providers with timestamps |
| compaction-drop magnitude | reset index `s` (detected today, value discarded) | observed real ceiling + compression ratio | honest occupancy % (Roadmap #1); per-session/per-provider backstop | all providers (drop visible in timeline) |
| cache-read/create (`c`/`r`) trajectory | `usage` blocks | sustained `c` = growth; high `r` = reuse | directional cache-thrash gate (refines ADR-008/023 `rho`) | Claude/Codex (cache fields); n/a where absent |

Invariant note: Tier A is computable from `usage` blocks, line counts, and
timestamps brim already reads. It does NOT break the deterministic /
transcript-tokens-only design.

### Tier B — needs parsing tool blocks; STILL token-only/deterministic; realizes ADR-010 signal (a)

Tier B requires parsing `tool_use` / `tool_result` blocks (already present in
every provider transcript) but reads only their structure — tool name, an
argument hash, the `is_error` flag, and result size — NEVER their natural-
language content and NEVER an eval probe. This is the key research finding:
ADR-010 signal (a) "behavioral degradation" is realizable from tool-call
STRUCTURE alone, WITHOUT eval probing, so it keeps the deterministic invariant.

| variable | source field | predicts | intended verdict use | cross-provider |
|---|---|---|---|---|
| tool-call repetition (same tool+args, `input_hash` ≥3×) | `tool_use` blocks | agent stuck in a loop (12–29% of samples in the agent-ops literature) | behavioral `over_recycle` gate that fires regardless of token volume | any provider emitting tool_use |
| tool failure streak (same tool failing N consecutive) | `tool_result` `is_error` | thrash / dead-end | `nearing` → `over` signal | any provider with tool_result errors |
| ping-pong alternation (A→B→A, no output change) | `tool_use` sequence | doom-loop | same behavioral gate | any provider emitting tool_use |
| tool-output churn (large `tool_result` re-fetched) | `tool_result` sizes | redundant-read bloat | growth-quality refinement | any provider emitting tool_result |

ADR-010 realization note: Tier B is the deterministic, transcript-only path
to signal (a). It needs tool-block parsing but no eval probing and no content
inspection — the loop / failure-streak / ping-pong signals come from
repetition and error structure, not semantics.

### Tier C — needs content inspection / a new source; BREAKS the invariant (OUT OF SCOPE NOW)

Tier C is recorded for completeness only and is explicitly out of scope to
adopt now: each item requires content inspection or a new data source, which
breaks the deterministic / transcript-tokens-only invariant (Roadmap #5
territory — a different product, a deliberate fork).

- semantic divergence (drift from initial intent) — needs an LLM-judge or
  embeddings.
- positional / recency salience (degradation by distance-from-end once >50%
  full) — needs content-position modeling.
- recycle-outcome label (did recycling actually help) — supervised
  calibration source (Roadmap #3), not a transcript signal.

## Rationale

- Signal (a) "behavioral degradation" is named in ADR-010 §3 as the "true
  onset detector" but DEFERRED there as "needs eval probing." Research
  (agent-ops loop/repetition/failure-streak literature) shows it is derivable
  from tool-block STRUCTURE already in every provider transcript — so it is
  achievable WITHOUT eval probing and WITHOUT breaking the invariant. That is
  the reason to catalog the candidate set now.
- The Tier-A drop-magnitude signal lets brim LEARN each provider's real
  ceiling (Claude auto-compacts ~80% ≈ 160k/200k; Codex CLI ~90% ≈ 245k;
  opencode prune-then-summarize) instead of assuming the fixed 128k backstop —
  provider-agnostic by construction.

## Acceptance Criteria

- [ ] The Tier A and Tier B catalogs are recorded as the candidate signal set,
      each entry naming source field / what it predicts / intended verdict use /
      cross-provider availability.
- [ ] It is stated explicitly that Tier A keeps the deterministic /
      transcript-tokens-only invariant.
- [ ] It is stated explicitly that Tier B realizes ADR-010 signal (a) from
      `tool_use` / `tool_result` blocks WITHOUT eval probing.
- [ ] Tier C is recorded as out-of-scope-now (breaks the invariant; Roadmap #5).
- [ ] No threshold is asserted as accepted fact — all anchors are candidates
      pending calibration (Roadmap #3).

## Test Coverage

No behavioral test: this is a planning/catalog artifact (candidate signal set), not a shipped behavior. Nothing to assert at runtime.

<!-- Catalog for the decision in ADR "Behavioral degradation gate from
tool-call structure" [[ADR-010]]; serves STORY-012 [[STORY-012]]; refines the
verdict bands of REQ-004 (lore does not support req<->req links, so REQ-004 is
recorded in prose); parent feature FEATURE-001 [[FEATURE-001]]. -->
