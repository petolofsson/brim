---
id: ADR-024
title: Behavioral degradation gate from tool-call structure
status: Accepted
related_requirements:
  - REQ-004
  - REQ-016
related_adrs: []
related_stories: [STORY-012]
related_tests: [TEST-012]
---

# ADR-024 - Behavioral degradation gate from tool-call structure

## Context

ADR-010 §3 defines recycle as an OR of four signals and names signal **(a)
behavioral degradation** the "true onset detector" — but DEFERS it as "needs
eval probing." The shipped engine therefore keys entirely off absolute tokens
(ADR-010 OR-gate; ADR-011 narrowed by ADR-020). Two consequences:

- **Stage-4/5 blindness.** The engine verdict enum saturates at
  `over_recycle`. The recycle-advisory recipe renders 5 presentation stages
  (lean / drift / bloated / stale / critical), but stages 4–5 (stale vs
  critical) are PURE OCCUPANCY — once the engine is at `over_recycle`,
  velocity and cache-thrash stop contributing and only raw token count
  separates the top two tiers (recycle-verdict-model.md scope-gap A/B).
- **No true-onset gate.** A spinning agent (looping tool calls, failure
  streak) below the absolute backstop reads healthy; a healthy long session
  above it reads critical. The token axis cannot tell "stuck" from "big."

Key research finding (REQ-016): signal (a) does NOT require eval probing. It
is derivable from `tool_use` / `tool_result` block STRUCTURE — tool name, an
argument hash, the `is_error` flag, result size — already present in every
provider transcript. The agent-ops literature gives concrete deterministic
signals: tool-call repetition (loop), tool failure streak, ping-pong
alternation. These keep brim's deterministic / transcript-tokens-only
invariant (ADR-011 mechanism, ADR-020 scoping both untouched).

ADR-020 already narrowed ADR-011's rationale to reasoning/agentic context;
this ADR adds a complementary BEHAVIORAL axis for the same domain, where the
occupied window is dominated by reasoning/tool-use traffic.

## Decision

Realize ADR-010 signal (a) as a **deterministic behavioral gate** fed by
tool-call-structure signals (repetition / failure-streak / ping-pong),
fixing the stage-4/5 blindness where the engine saturates at `over_recycle`
and only raw occupancy distinguishes the top tiers.

- The gate consumes the **Tier-B** candidate signals catalogued in REQ-016,
  reading tool-block STRUCTURE only — no eval probing, no content inspection.
  The deterministic / transcript-tokens-only invariant is preserved.
- This ADR **does NOT supersede ADR-010** — ADR-010 stays Accepted. This ADR
  EXTENDS it by UN-DEFERRING signal (a): the "deferred quality tier" named in
  ADR-010 §3(a) becomes a realizable, transcript-only gate.
- The token gates (watch ~32k, backstop ~128k), projection `tau`, and
  cache-thrash `rho` are unchanged. The behavioral gate is additive.
- **Advisory-only, read-only is preserved** (ADR-010 §5): brim recommends,
  never recycles or mutates a session.
- Status is **Draft** — direction, not a committed implementation. No
  threshold is asserted as accepted fact; behavioral anchors (e.g. "≥3×
  identical call", "N consecutive failures") are candidates pending
  calibration (recycle-verdict-model.md Roadmap #3).

### Open design question (deliberately unresolved at Draft)

1. **Which Tier-B signal first?** Tool-call repetition (clearest loop signal,
   12–29% of samples in the literature) vs failure-streak (cheapest, just the
   `is_error` flag) vs ping-pong. Likely failure-streak or repetition first.
2. **How does it compose with the verdict?** Two candidate shapes:
   - **(i) extend the OR-gate** — add a behavioral term that can raise the
     existing 3-value verdict to `over_recycle` regardless of tokens (minimal
     surface, but collapses into the same saturated enum that caused the
     stage-4/5 blindness); or
   - **(ii) a separate behavioral axis** — report behavioral state alongside
     the token verdict, so a consumer can render `stale` vs `critical` from
     (token-occupancy × behavioral-state) without the enum saturating. This
     pairs naturally with promoting the 5 presentation tiers into the engine
     (Roadmap #4) and is the path that actually closes the stage-4/5 blindness.

   The choice between (i) and (ii) is left to the implementing change.

## Consequences

- ADR-010 signal (a) becomes achievable without eval probing; the "true onset
  detector" stops being permanently deferred.
- A stuck/spinning context can warn below the absolute backstop, and the top
  presentation stages can be separated by behavioral state rather than raw
  occupancy alone — closing scope-gap A/B's pure-occupancy top tiers.
- Adds a tool-block parsing step to the provider feed (structure only). No
  change to the read-only / advisory trust boundary.
- If composed as a separate axis (option ii), the REQ-005 JSON contract gains
  a behavioral field — coordinate with Roadmap #4 (promote the 5 tiers into
  the engine). If composed into the OR-gate (option i), the contract is
  unchanged but the stage-4/5 split is not fully recovered.
- Anchors remain unvalidated until calibration (Roadmap #3); shipping must keep
  them tunable, consistent with ADR-010's tunable-default posture.

## Alternatives Considered

- **Keep signal (a) deferred (status quo).** Rejected — leaves the named "true
  onset detector" unrealized and the top presentation stages blind to anything
  but raw tokens; a spinning sub-backstop session stays invisible.
- **Realize signal (a) via eval probing / an LLM-judge (Tier C).** Rejected
  now — breaks the deterministic / transcript-tokens-only invariant
  (recycle-verdict-model.md Roadmap #5; REQ-016 Tier C). A different product;
  not a next step.
- **Supersede ADR-010 with a new verdict model.** Rejected — ADR-010's
  absolute-token mechanism is correct for brim's workload (ADR-011/ADR-020);
  the gap is the deferred signal (a), which this ADR extends rather than
  replaces.

<!-- Extends ADR-010 (un-defers signal (a); does NOT supersede it) and sits
beside ADR-020 (which scoped the absolute-token rationale to reasoning/agentic
context) — lore does not support adr<->adr links, so both are recorded in
prose. Realizes [[STORY-012]]; consumes the candidate catalog [[REQ-016]];
refines the verdict bands of [[REQ-004]]. -->

