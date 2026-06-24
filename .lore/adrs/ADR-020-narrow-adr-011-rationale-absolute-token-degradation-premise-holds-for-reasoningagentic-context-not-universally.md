---
id: ADR-020
title: "Narrow ADR-011 rationale: absolute-token degradation premise holds for reasoning/agentic context, not universally"
status: Draft
related_requirements: []
related_adrs: [ADR-011]
related_stories: []
related_tests: []
---

# ADR-020 - Narrow ADR-011 rationale: absolute-token degradation premise holds for reasoning/agentic context, not universally

## Context

ADR-011 ("brim reasons in absolute tokens only") rests its rationale on the
premise that degradation onset is governed by ABSOLUTE token count, not
advertised-window fraction (inherited from ADR-010). As stated, that premise
reads as a UNIVERSAL law of long-context behavior.

External long-context literature shows the premise is OVERCLAIMED at that
breadth:

- **NoLiMa** (Modarressi et al., ICML 2025, arxiv 2502.05167) and **RULER**
  (Hsieh et al., NVIDIA, COLM 2024, arxiv 2404.06654): for HARD, NON-LITERAL
  reasoning tasks, degradation onset is roughly WINDOW-AGNOSTIC and tracks
  absolute token count — e.g. at 32K most models drop well below their
  short-length baseline regardless of advertised window.
- **Gemini 1.5 technical report** (arxiv 2403.05530): for LITERAL
  retrieval/recall, onset is strongly WINDOW-DEPENDENT — Gemini 1.5 holds
  >99.7% needle recall to 1M tokens, while 200K-class models cannot operate
  past their advertised window at all. Here the advertised window, not an
  absolute token count, governs.

So the absolute-token premise is REFUTED as a universal law (literal recall is
window-bound) and is DEFENSIBLE only for hard non-literal reasoning — which is
precisely brim's domain: agentic coding context, where the occupied window is
dominated by reasoning/tool-use traffic, not a single literal needle. This was
recorded as the biggest weakness in docs/recycle-verdict-model.md (Validation,
external-literature CAVEAT), which recommended a superseding ADR.

## Decision

brim's absolute-tokens-only MECHANISM (ADR-011) is UNCHANGED and remains correct
for its workload. This ADR ONLY narrows the stated JUSTIFICATION:

- FROM: "degradation onset is governed by absolute token count, not
  advertised-window fraction" stated as a universal law.
- TO: "for reasoning/agentic context — brim's domain — effective degradation
  onset is roughly window-agnostic and absolute-token-driven; the
  absolute-tokens-only mechanism is therefore correct for this workload."

The narrowing explicitly does NOT claim the absolute premise for literal
retrieval/recall, where onset is window-dependent. No code or behavior change:
the watch band (~32k), recycle backstop (~128k), projection target, and JSON
surface are all untouched.

## Supersession

This ADR does NOT edit the Accepted ADR-011; it supersedes ONLY ADR-011's
absolute-vs-advertised-window-fraction RATIONALE CLAUSE (recorded here per the
never-edit-an-Accepted-ADR rule). ADR-011's decision, mechanism, and
consequences are otherwise intact.

## Consequences

- ADR-011's rationale is now scoped to reasoning/agentic context; the universal
  claim is withdrawn. Future readers know the absolute-token premise is not
  asserted for literal-recall workloads.
- No change to brim's behavior, fields, thresholds, or output.
- Aligns the documented rationale with docs/recycle-verdict-model.md Validation,
  closing the recorded CAVEAT.

## Alternatives Considered

- **Edit ADR-011's rationale in place.** Rejected — ADR-011 is Accepted;
  never-edit-an-Accepted-ADR rule requires a superseding ADR.
- **Drop the absolute-token premise entirely / re-introduce advertised-window
  reasoning.** Rejected — the premise is correct for brim's actual workload;
  only its universal phrasing was wrong. The mechanism stays.
