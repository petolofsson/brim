---
id: STORY-002
title: How brim reasons about context health
status: Accepted
related_requirements:
  - FEATURE-001
  - REQ-007
related_adrs:
  - ADR-006
  - ADR-007
  - ADR-008
  - ADR-009
  - ADR-010
  - ADR-011
related_stories: []
related_tests: []
---

# STORY-002 - How brim reasons about context health

> Reasoning narrative for new contributors: *why* brim diagnoses context the way
> it does. The behavior boundary lives in FEATURE-001; the decisions live in
> ADR-006..010. This story does not introduce new behavior — it explains the
> model that ties those decisions together. (Sibling intent: STORY-001,
> orchestrator self-diagnosis — not linked here only because lore rejects
> story↔story relations.)

## Vantage point: a read-only transcript scanner

brim is **not** a live proxy and holds no session state. It reads agent
transcripts off disk and computes a **point-in-time, per-turn window reading** —
the size of the *last* turn's context window, not aggregate spend across the
session (ADR-002). It is **stateless** between runs: every diagnosis is recomputed
from the transcript, nothing is persisted (ADR-004). This vantage point is the
root constraint behind everything below — brim observes, it never intercepts, and
it never mutates a session.

## Two denominators, two different questions

> Note (post-ADR-011): the advertised-window % denominator and capacity-runway readout described below were REMOVED by ADR-011. brim now reasons in absolute tokens only; see ADR-011.

A token count is meaningless without a denominator. brim deliberately keeps
**two**, because they answer two unrelated questions (ADR-010):

1. **Effective budget** — an *absolute*, model-agnostic token count. This is the
   **quality basis**: how far into measured-degradation territory the active
   context has crept. The same absolute band applies to a 200k model and a 1M
   model.
2. **Advertised-window %** — fill as a fraction of the model's marketed window.
   This is *only* a **capacity-runway readout**: distance to forced harness
   auto-compaction (~95% of the advertised window). It says nothing about quality.

Conflating the two is the classic mistake. "73% of the window" is a runway number,
not a health number.

## Why absolute, not ratio

The quality basis is absolute because degradation onset is governed by **absolute
token count, not percentage of window**. The research is consistent on this:

- **Effective context ≪ advertised** — usable context is typically a small
  fraction of the marketed window (RULER — arxiv.org/html/2404.06654v2; NoLiMa —
  arxiv.org/html/2502.05167v1, which defines effective length at 85% of baseline).
- **A 1M model buys no more usable *reasoning* context than a 200k one** — onset
  sits in the low thousands for both, so scaling a threshold by the advertised
  window over-credits the big-window model (up to ~5×).
- **Length is causal on its own** — performance drops with length even with
  perfect retrieval and distractors masked ("Context Length Alone Hurts" —
  arxiv.org/html/2510.05381v1, ~7k problem-solving onset).
- **Positional effects persist** — the U-shaped lost-in-the-middle penalty does
  not go away at scale (Lost in the Middle, TACL 2024 —
  aclanthology.org/2024.tacl-1.9).
- **Reasoning collapses early** — latent/multi-hop reasoning degrades at 2k–8k
  (NoLiMa; BABILong struggles past ~10k — arxiv.org/pdf/2406.10149); agentic
  coding follows the reasoning curve, not the retrieval curve.
- **Task-dependent** — literal retrieval can survive to ~32k while reasoning has
  already collapsed; there is no single number for "full."
- **Composition hurts independently of fill** — distractors degrade output at a
  *fixed* length; any prior-context token's logit can be boosted 10–100×
  (Llama-See-Llama-Do, ACL 2025 — arxiv.org/pdf/2505.09338), and error rises as a
  power law in reasoning depth × distractor count (GSM-DC —
  arxiv.org/pdf/2505.18761). This is the empirical basis for treating thrash as a
  signal in its own right (ADR-008).
- **No clean breakpoint** — degradation is non-uniform and sometimes cliff-like
  across frontier models including Claude 4 (Chroma "Context Rot" —
  research.trychroma.com/context-rot), so behavioral signals beat any fixed
  number.

(Two claims were **refuted** by the research pass and are deliberately *not* used
as support: the "10–20% utilization is a hard rule" claim, and "instance count
predicts degradation more than length.")

## The four recommendation gates (OR)

brim recommends a recycle when **any one** of four signals fires (ADR-010 §3) —
an OR, because each catches a failure the others miss:

1. **Behavioral degradation** — the true onset detector. **Deferred**: this
   quality tier is not yet implemented (see open questions).
2. **Projection-to-capacity** — velocity/overbound projection from a bounded
   last-k tail read (ADR-006): will this session run out of runway soon?
3. **Absolute effective-budget fill** — crossing the watch/recycle band of the
   absolute, model-agnostic budget (ADR-010). Default `watch` ≈ **32k active
   tokens** ("in measured-degradation territory"); `recycle` does **not** fire
   mechanically at 32k — real agentic sessions run far past it, so that would cry
   wolf. The absolute budget is the single tunable knob.
4. **Cache-thrash** — cache-hit-ratio as a context-thrash signal (ADR-008):
   churn degrades output independent of raw fill.

## Hierarchy: self vs subtree, and what to recycle

brim diagnoses a **tree** of agents, not a single session (ADR-007). Each node
carries **self** health (its own window) and **subtree** health (rolled up from
descendants), so an orchestrator can be healthy itself while a child is
overbounding, or vice versa.

When a recycle is warranted, the **target is the deepest, smallest node** that
resolves the problem, to minimize **blast radius** (ADR-009). A leaf can be
recycled independently of its parent; recycling higher up costs more, so brim
prefers the lowest node that fixes it and reports the blast radius of the choice.

## Advisory-only invariant

brim **only recommends** — which node, why, and the blast radius. It **never
recycles, mutates, or acts on a session**; the decision is always the user's
(ADR-010 §5). This is not a missing feature — it is the trust boundary that
follows directly from the read-only, stateless vantage point above.

## Honest limits / open questions

- **Behavioral onset detection is deferred.** The behavioral tier (gate 1) is the
  *real* degradation detector; until it ships, the absolute threshold and
  projection are early-warning fallbacks, not the true signal.
- **Frontier transfer is unverified.** The 7k–32k onset band is measured on
  7B–70B and 2024-era models. Whether it holds quantitatively on current frontier
  1M-window models (Opus 4.x, Gemini 2.5, GPT-4.1+) is unverified — which is
  *why* the absolute budget is tunable, not a hard claim.
- **Compaction-loss magnitude is unquantified.** No measured figure exists for the
  discrete quality drop caused by harness auto-compaction/summarization. The
  qualitative case for deliberate early recycling (don't let the harness make the
  lossy discard for you) stands, but the size of the loss is unmeasured.

## Acceptance Criteria

- [x] The narrative accurately describes brim's reasoning model (point-in-time window, absolute effective budget, four-gate OR recycle, self-vs-subtree, advisory-only) as implemented and accepted in ADR-002/004/006/007/008/009/010/011.
