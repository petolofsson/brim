---
id: ADR-010
title: "Recycle threshold: absolute effective-budget over advertised-window ratio"
status: Accepted
related_requirements:
  - REQ-004
  - REQ-007
related_adrs:
  - ADR-002
  - ADR-006
  - ADR-008
related_stories:
  - STORY-001
related_tests: []
---

# ADR-010 - Recycle threshold: absolute effective-budget over advertised-window ratio

## Context

STORY-001 needs a preemptive recycle *recommendation*. The naive basis — a
percentage of the advertised window (REQ-004: nearing 70% / ceiling 90%) — is
wrong per a deep-research pass over 18 primary sources (RULER, Lost-in-the-Middle
TACL 2024, NoLiMa, BABILong, Chroma "Context Rot", "Context Length Alone Hurts",
"Llama-See-Llama-Do", GSM-DC; 23/25 verified claims). Key findings:

- **Onset is governed by ABSOLUTE token count, not % of window.** A 1M-window
  model gets no more usable *reasoning* context than a 200k model — onset sits in
  the low thousands for both. Isolated causally: length degrades performance even
  with perfect retrieval and distractors masked (≥7.9% drop at 30k masked).
- **Effective context ≪ advertised** — typically 10–50% of the marketing number
  (NoLiMa: GPT-4o effective ~8k, Claude 3.5 Sonnet ~4k vs 128k+ claimed).
- **Task-dependent**: literal retrieval survives to ~32k; multi-hop/latent
  reasoning collapses at 2k–8k. Agentic coding (mixed reasoning + long tool
  chains) follows the reasoning curve. Most models drop to ≤half baseline by ~32k.
- **Composition degrades independently of fill** (validates ADR-008): distractors
  hurt at fixed length; any prior-context token's logit is boosted 10–100×.
- **No clean breakpoint** — non-uniform, sometimes cliff-like → behavioral signals
  beat any fixed number.

Refuted (NOT cited as support): "LLMs utilize only 10–20% as a hard rule"
(refuted 0-3); "instance count predicts degradation more than length"
(refuted 0-3).

## Decision

1. **The recycle recommendation keys off an ABSOLUTE, model-agnostic
   effective-budget (configurable), NOT a percentage of the advertised window.**
   The same absolute band applies to 200k and 1M models — this is what makes the
   heuristic generalize across models.
2. **Advertised-window % is demoted to a capacity-runway readout only** — distance
   to forced auto-compaction (~95% of the advertised window), a separate question
   from quality.
3. **Recycle = OR of four signals:** (a) behavioral degradation signals [deferred
   quality tier — the true onset detector]; (b) projection-to-capacity (ADR-006);
   (c) absolute effective-budget fill crossing a watch/recycle band; (d)
   cache-thrash watch (ADR-008).
4. **Default bands:** `watch` ≈ **32k absolute active tokens** (research-anchored:
   most models ≤half baseline by ~32k; model-agnostic), surfaced as "in
   measured-degradation territory." `recycle` does **not** fire mechanically at
   32k (real agentic sessions run far past it — that would cry wolf); the
   act-level trigger combines projection + behavioral + a higher pragmatic
   backstop, with 32k as the quality-runway marker beneath it. The absolute budget
   is the single tunable knob, defaulted conservatively.
5. **brim only RECOMMENDS — it never recycles, mutates, or acts on a session.**
   The recycle decision is always the user's. brim emits an advisory
   recommendation (which node, blast radius per ADR-009, why); the user decides.
   Consistent with brim's read-only design (CODERULES r11) and statelessness
   (ADR-004).

## Consequences

- Fixes the 1M mis-scaling (advertised-% overstates usable context up to ~5×). The
  absolute basis "fits all models" by construction.
- Supersedes REQ-004's threshold *defaults* on acceptance; REQ-004 untouched while
  this is Draft.
- Behavioral signals remain the real onset detector (deferred tier); the absolute
  threshold is a fallback/early-warning, not the primary gate.
- Advisory-only keeps brim within its read-only trust boundary — no session
  mutation, ever.

### Open questions (caveats)

1. **Frontier transfer:** the 7k–32k onset band is measured on 7B–70B and 2024-era
   models; whether it holds quantitatively on current frontier 1M-window models
   (Opus 4.x, Gemini 2.5, GPT-4.1+) is unverified. The absolute default must
   therefore be tunable, not a hard claim.
2. **Compaction-loss magnitude:** no measured figure was found for the discrete
   quality drop caused by harness auto-compaction/summarization. The qualitative
   argument for deliberate early recycling (avoid letting the harness make the
   lossy discard) stands, but is unquantified.

## Alternatives Considered

- **Recycle on % of advertised window (REQ-004 defaults: 70% / 90%).** Rejected —
  onset is governed by absolute token count, not window fraction; this overstates
  usable context up to ~5× on 1M-window models.
- **Single fixed absolute threshold as the mechanical recycle gate.** Rejected —
  no clean breakpoint exists (non-uniform, sometimes cliff-like); a fixed number
  fired alone would cry wolf on long real sessions. Use the OR of behavioral +
  projection + absolute-budget + cache-thrash instead.
- **Let the harness auto-compact rather than recommend recycling.** Rejected —
  defers the lossy discard to the harness at an uncontrolled point; deliberate
  early recycling is preferable (though compaction-loss magnitude is unquantified).

## Rationale — primary sources

- RULER — arxiv.org/html/2404.06654v2 (effective ≪ advertised)
- Lost in the Middle (Liu et al., TACL 2024) — aclanthology.org/2024.tacl-1.9
  (positional U-shape persists)
- NoLiMa — arxiv.org/html/2502.05167v1 (latent reasoning drops 2k–8k;
  85%-of-baseline effective-length definition)
- BABILong — arxiv.org/pdf/2406.10149 (struggles past ~10k tokens)
- Context Length Alone Hurts — arxiv.org/html/2510.05381v1 (absolute length
  causal; ~7k problem-solving onset)
- Chroma "Context Rot" — research.trychroma.com/context-rot (18 frontier models
  incl. Claude 4; composition + non-uniform/cliff)
- Llama-See-Llama-Do (ACL 2025) — arxiv.org/pdf/2505.09338 (distractor logit boost
  10–100×, mechanistic)
- GSM-DC — arxiv.org/pdf/2505.18761 (error power-law with reasoning depth +
  distractor count)
