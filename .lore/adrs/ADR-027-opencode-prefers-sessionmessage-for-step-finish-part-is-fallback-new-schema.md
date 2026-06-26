---
id: ADR-027
title: opencode prefers session_message for step-finish; part is fallback (new schema)
status: Accepted
related_requirements:
  - REQ-008
related_adrs: []
related_stories: []
related_tests:
  - TEST-004
---

# ADR-027 - opencode prefers session_message for step-finish; part is fallback (new schema)

> Supersedes the **part-only** step-finish assumption of **ADR-005** (already
> Superseded by ADR-015 on the limit axis; this ADR corrects its source-table
> axis). The `parent_id` join, the `data.tokens` field mapping, and the
> aggregate-fallback design from ADR-005 remain in force. ADR-002 (point-in-time
> over cumulative) is the governing principle and is unchanged.

## Context

ADR-005 fixed opencode's point-in-time oracle to the latest `part` row with
`json_extract(data,'$.type')='step-finish'`, ordered `time_created DESC`. That
was correct for opencode v1.17.5.

This decision is **DEFENSIVE forward-compat**, not a fix for an observed
current-version bug. The opencode 1.17.9 schema adds a `session_message` table
with native `type` and `seq` columns (`UNIQUE(session_id,seq)`, index
`(session_id,type,seq)`), so a future opencode *could* relocate step-finish rows
there. brim prefers that table **if step-finish is present in it**, and falls
back to `part` otherwise.

**VERIFIED against opencode 1.17.9** (real verification run): step-finish rows
are **observed in `part`**, NOT in `session_message`. A real `part` step-finish
row was read and its token JSON shape
(`data.tokens.{total, input, output, cache.{read, write}}`) **matches brim's
`step_finish_oracle` exactly**. The `session_message`-empty → `part` fallback
fires correctly, so the original part-only parser would have worked on 1.17.9 —
**the "silent aggregate fallback" was NOT reproduced on any observed version.**

The dossier's earlier "relocated step-finish out of `part` into
`session_message`" claim is **UNCONFIRMED**: no opencode version has been
observed actually moving step-finish there. The `session_message` preference is
therefore speculative/forward-looking — harmless if the move never happens
(the `part` fallback covers today's schema), and correct ahead-of-time if it
does. Aggregate spend (cumulative, which ADR-002 establishes is NOT window
occupancy) remains the last-resort fallback tagged `window_source = aggregate`.

This matters for **ADR-026 M1**: opencode is behavior-blind (`behavior:None`), so
the vote-counter leans on accurate occupancy for the `recycle_backstop` decisive
override that supplies opencode's STORY-011 hard-compaction floor. A silently
wrong occupancy weakens that backstop.

Verified schema (read-only local DB):

```
session_message: id, session_id, type, seq, time_created, time_updated, data
  UNIQUE(session_id,seq); INDEX(session_id,type,seq), (session_id,time_created,id)
  -- table EXISTS in 1.17.9 but currently carries NO step-finish rows
part: id, message_id, session_id, time_created, time_updated, data
  -- step-finish OBSERVED here on 1.17.9 via json_extract(data,'$.type')
```

## Decision

brim's opencode step-finish oracle uses this source precedence:

1. **Preferred — `session_message`.** Latest row for the session where the native
   `type` column = `'step-finish'`, ordered `seq DESC LIMIT TREND_TAIL_K`.
2. **Fallback — `part`.** When `session_message` is absent (table missing →
   `prepare()` fails → treated as "not available") or has no step-finish row for
   the session, the old `part` query (`json_extract(data,'$.type')='step-finish'`,
   `time_created DESC`).
3. **Aggregate.** When neither table yields a step-finish row, the `session`
   aggregate columns, tagged `window_source = Aggregate` (ADR-002 permitted
   approximation) — unchanged from ADR-005.

Helpers: `fetch_step_finish_rows` → `try_session_message` / `try_part`. The
`data.tokens` field mapping (`{ total?, input, output?, cache: { read, write } }`)
and `window_tokens` formula are unchanged from ADR-005 — only the source table
and its ordering column (`seq` vs `time_created`) change.

## Consequences

* If a future opencode relocates step-finish to `session_message`, brim already
  reports point-in-time occupancy from it instead of silently degrading to the
  aggregate — preserving the occupancy accuracy ADR-026 M1's backstop relies on
  for behavior-blind opencode, with no flag-day cutover.
* Today's opencode (1.17.9, step-finish in `part`) and older DBs keep working via
  the `part` fallback — the observed-correct current path.
* Two extra prepared queries per session in the worst case (session_message miss
  → part). Bounded by `TREND_TAIL_K`; no full scan.

## Verified — step-finish `data` JSON shape (opencode 1.17.9)

**VERIFIED against opencode 1.17.9.** A real step-finish row was read from the
`part` table and its token JSON shape —
`data.tokens.{total, input, output, cache.{read, write}}` — **matches brim's
`step_finish_oracle` exactly**. The `window_tokens` mapping
(`input + cache.read + cache.write`) reads correctly off it.

step-finish is currently observed in `part`, NOT `session_message`, so the
`session_message`-empty → `part` fallback supplies the point-in-time window on
1.17.9. The forward-looking risk is only theoretical: *if* a future opencode both
relocates step-finish to `session_message` AND changes the token shape there
(renamed/nested fields, different units), the `session_message` parse could yield
zero/garbage tokens and the oracle would revert to the aggregate fallback. That
combination is unobserved on any version. If a future schema move lands, re-verify
the `session_message` token shape and supersede this ADR if the mapping differs.

## Alternatives Considered

- **Keep part-only (ADR-005), add nothing.** Defensible today — part-only is
  observed-correct on 1.17.9. Rejected only as forward-compat: if a future
  opencode moves step-finish to `session_message`, part-only would silently fall
  back to the aggregate (cumulative ≠ window, ADR-002), degrading opencode's
  behavior-blind backstop. The `session_message` preference forecloses that
  failure cheaply.
- **Query only `session_message` (drop `part`).** Rejected — breaks older
  opencode DBs that still carry step-finish in `part`; the fallback is cheap and
  schema-version-agnostic.
- **Detect schema version up front.** Rejected — `prepare()` failure already
  distinguishes table presence per query; an explicit version probe adds a round
  trip and a version registry for no behavior gain.
