---
id: ADR-008
title: Cache-hit ratio as a context-thrash signal
status: Accepted
related_requirements:
  - REQ-007
  - REQ-001
related_adrs:
  - ADR-002
related_stories: []
related_tests: []
---

# ADR-008 - Cache-hit ratio as a context-thrash signal

## Context

brim already parses the cache split (`cache_read_input_tokens` /
`cache_creation_input_tokens` / `input_tokens`) to compute window tokens, then
discards the composition. A **falling cache-read fraction** across turns
indicates the cached prefix is churning — context thrash. It is an early, cheap
symptom obtainable with zero new parsing and no message-content read.

## Decision

Expose `cache_hit_ratio = cache_read_input_tokens / window_tokens` (bounded
`[0,1]`) per turn, trended over the REQ-007 timeline. Providers that do not
report a cache split yield `None`. This is a **secondary** signal: it informs
diagnosis but does **not** by itself drive the recycle verdict.

## Consequences

* Thrash visibility for free — reuses the already-parsed cache split, faithful
  to ADR-002's usage-only framing.
* `None` for non-cache providers, documented per provider.
* As a secondary signal it cannot produce a false recycle verdict on its own;
  it only corroborates the primary fill/growth signals.

## Alternatives Considered

- **Drive the verdict from cache ratio alone.** Rejected — a low hit ratio can
  be benign (cold start, legitimate large new input); it must corroborate, not
  decide.
- **Parse message content to attribute the churn.** Rejected — violates the
  usage-metadata-only constraint; the ratio already localizes thrash in time
  without reading content.
