---
id: ADR-011
title: brim reasons in absolute tokens only; drop advertised-window limit and capacity readout
status: Accepted
related_requirements:
  - REQ-001
  - REQ-004
  - REQ-007
related_adrs: []
related_stories: [STORY-010]
related_tests: [TEST-008]
---

# ADR-011 - brim reasons in absolute tokens only; drop advertised-window limit and capacity readout

## Context

Degradation onset is governed by ABSOLUTE token count, not advertised-window
fraction (ADR-010). The recycle verdict already keys entirely off absolute
tokens — the watch band (~32k) and recycle backstop (~128k) — plus the ADR-006
projection and the ADR-008 cache-thrash signal. None of these consult the
advertised window.

The advertised window only ever powered one thing: a `capacity_runway` readout
("distance to forced auto-compaction at ~95% of the window"), the demoted role
ADR-010 left it in. That readout is REDUNDANT: the absolute verdict fires far
earlier than any ~95%-window auto-compaction, so the runway never carries the
decision. Keeping it alive requires a per-provider limit table (or recorded-window
precedence) that is inherently approximate for transcript-only, stateless brim —
Claude Code strips the `[1m]` suffix before persisting, so the bare model id
cannot recover the true resolved window. The accuracy machinery buys a signal
nothing reads.

Whether to DISPLAY a fill percentage is the consumer's concern: brim reports
absolute occupancy and the verdict; a UI that wants "73% of 200k" can divide by
a window it chooses. brim should not own that window.

This ADR drops the entire advertised-window apparatus and has brim reason in
absolute tokens only.

## Decision

1. **Remove the advertised-window apparatus.** Delete the model→limit table,
   `WindowInfo.context_limit`, `WindowInfo.fill_percent`,
   `TimelinePoint.fill_percent`, the `capacity_runway` readout, and the
   `--nearing` / `--ceiling` advertised-% thresholds. Keep `window_tokens`,
   `model` (display), `window_source` (last_turn vs aggregate provenance) and
   `cache_hit_ratio`.

2. **Re-target the ADR-006 projection from the advertised window to the absolute
   recycle-backstop (default 128k).** `compute_trend` projected
   `(context_limit − current) / velocity`; it now projects to the backstop.
   The field/JSON key `projected_turns_to_overbound` is renamed
   `projected_turns_to_recycle` — "turns until crossing the absolute recycle
   backstop." The ADR-010 projection OR-gate is unchanged in behavior; it simply
   reads the re-targeted value, which makes the whole signal window-independent.

3. **SubtreeInfo worst-child metric switches from `worst_fill_percent` to
   worst-by-absolute-tokens** (`worst_tokens` + `worst_tokens_node`), and the
   worst-first sort key in the output follows. `total_subtree_tokens` is kept.

4. **JSON (REQ-005) drops the fill/limit/capacity fields** (`fill_percent`,
   `context_limit`, `capacity_runway`); everything retained stays
   additive-stable.

## Supersession

This ADR does NOT edit the Accepted ADR-006 or ADR-010; it supersedes specific
clauses of each (recorded here per the never-edit-an-Accepted-ADR rule):

- **Supersedes ADR-010 decision (2), the "advertised-window % demoted to a
  capacity-runway readout" clause.** The readout is now REMOVED, not demoted.
  ADR-010's absolute-token verdict (watch/backstop OR-gate) is otherwise intact.
- **Supersedes ADR-006's projection TARGET.** The projection is re-targeted from
  the advertised window (`limit − current`) to the absolute recycle-backstop.
  ADR-006's mechanism (median of positive deltas over a bounded last-K tail,
  negative-delta reset detection, `None` with <2 post-reset points) is unchanged.

## Consequences

- brim reports: absolute occupancy (`window_tokens`), the verdict + gate,
  velocity/projection (to the backstop), the cache-thrash signal, subtree health
  (by absolute tokens), and the recycle recommendation. No advertised-window
  concept remains anywhere in brim.
- The 1M-vs-200k mis-scaling problem disappears by construction — there is no
  window to scale against, so no per-provider limit table to maintain and no
  `[1m]`-suffix ambiguity to label.
- Displaying a fill percentage becomes purely the consumer's concern; a consumer
  that wants one divides `window_tokens` by a window of its choosing.
- The verdict path is behavior-unchanged except that its projection input now
  targets the backstop rather than the advertised window.

## Alternatives Considered

- **Keep `capacity_runway` accurate via a corrected limit table / provider
  recorded-window precedence (this was the rejected ADR-011 / REQ-010 pair).**
  Rejected — the runway signal is redundant given the absolute verdict fires far
  earlier, and is not worth the per-provider limit-resolution machinery
  (recorded-window precedence, a corrected static table, an `assumed (not
  recorded)` provenance label, a `--context-limit` override) that an approximate
  transcript-only tool would need to carry.
- **Leave the demoted readout in place (ADR-010 status quo).** Rejected — it
  keeps a mis-scalable, never-consulted number in the model and JSON surface,
  inviting consumers to treat it as authoritative.
