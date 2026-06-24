---
id: ADR-015
title: opencode z-ai/glm-5.2 context limit is 1,000,000; supersede ADR-005 200k assumption
status: Accepted
related_requirements:
  - REQ-002
related_adrs:
  - ADR-005
related_stories:
  - STORY-003
related_tests: []
---

# ADR-015 - opencode z-ai/glm-5.2 context limit is 1,000,000; supersede ADR-005 200k assumption

## Context

ADR-005 (Draft) assumed `z-ai/glm-5.2` context limit = **200,000** because the
limit is not stored in the opencode db or config and brim's default is 200k.
STORY-003's verified matrix (`docs/provider-capability-matrix.md`) corrected this
from `https://models.dev/?q=glm` (the AI-SDK model registry opencode imports):
`zhipuai/glm-5.2` Context = **1,000,000**, Output = 131,072. The real opencode DB
queried read-only during STORY-003 found 0 of N sessions with a non-200k default —
the limit is not stored in the `session` / `part` / `project` tables (`.schema`
confirmed), agreeing with ADR-005's secondary claim. brim's 200k default therefore
underestimates the real window by 5× for opencode sessions.

## Decision

(a) The opencode `z-ai/glm-5.2` context-window limit used by brim is **1,000,000**
(sourced to models.dev, the registry opencode imports).
(b) ADR-005's 200k assumption is **superseded**.
(c) ADR-005's step-finish oracle, `parent_id` join, and aggregate-fallback design
are **retained unchanged** (confirmed against the real DB by STORY-003).
(d) ADR-005's secondary claim "limit not stored in opencode db or config" is
**confirmed**.
(e) No model→limit registry is introduced in this ADR; the single observed model
gets its verified limit and the registry question remains revisit-when-needed.

## Consequences

brim's opencode occupancy verdicts now reason against 1M, not 200k; recycle
projections (ADR-006) for opencode sessions land later in the true window,
matching real behavior. The 200k default remains for any model brim cannot resolve
(Claude-without-`[1m]`, etc.), per the existing fallthrough; this ADR only fixes
the opencode/glm-5.2 case. If a future opencode model has a different limit,
supersede this ADR with a new one.

## Supersession

Supersedes the 200k limit assumption of ADR-005 only. ADR-005's oracle / fallback /
join design is preserved. ADR-005 transitions Draft → Superseded.

## Alternatives Considered

- **Edit ADR-005 in place.** Rejected — ADR-005 is Draft (editable in principle), but
  the matrix file recommends the supersession route and the limit correction is a
  substantive decision change best recorded as a distinct ADR so the audit trail is
  explicit.
- **Build a model→limit registry now.** Rejected — only one model is observed and
  its verified limit is now hardcoded; a registry adds surface area without behavior.
  Revisit when a second non-default opencode model appears.
