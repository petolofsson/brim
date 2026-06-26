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

New opencode **relocated step-finish rows out of `part` into a new
`session_message` table** with native `type` and `seq` columns
(`UNIQUE(session_id,seq)`, index `(session_id,type,seq)`). Under the new schema
the `part` table no longer carries step-finish rows. The old part-only oracle
therefore found nothing and **silently fell back to the `session` aggregate
columns** — cumulative spend, which ADR-002 establishes is NOT window occupancy.
Reported occupancy was wrong (cumulative ≠ point-in-time) with no signal to the
consumer beyond the `window_source = aggregate` tag.

This matters for **ADR-026 M1**: opencode is behavior-blind (`behavior:None`), so
the vote-counter leans on accurate occupancy for the `recycle_backstop` decisive
override that supplies opencode's STORY-011 hard-compaction floor. A silently
wrong occupancy weakens that backstop.

Verified schema (read-only local DB):

```
session_message: id, session_id, type, seq, time_created, time_updated, data
  UNIQUE(session_id,seq); INDEX(session_id,type,seq), (session_id,time_created,id)
part (new): id, message_id, session_id, time_created, time_updated, data
  -- no type column; step-finish data NOT here under the new schema
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

* Sessions on the new opencode schema again report point-in-time occupancy
  instead of silently degrading to the aggregate, restoring the occupancy
  accuracy ADR-026 M1's backstop relies on for behavior-blind opencode.
* Older opencode DBs keep working via the `part` fallback — no flag-day cutover.
* Two extra prepared queries per session in the worst case (session_message miss
  → part). Bounded by `TREND_TAIL_K`; no full scan.

## Open Risk — UNVERIFIED step-finish `data` JSON shape (verification PENDING)

The fix **assumes** new opencode kept the step-finish `data` JSON shape
identical — `data.tokens.{total, input, output, cache.{read, write}}` — and only
relocated the row from `part` to `session_message`. This is **UNVERIFIED**: the
local opencode DB is **empty (0 rows)**, so the assumed shape could not be checked
against populated new-schema data.

If the shape actually changed (renamed/nested token fields, different units), the
`session_message` parse yields zero/garbage tokens and the oracle **silently
reverts to the aggregate-fallback bug** — the exact failure this ADR fixes — which
again weakens ADR-026 M1's backstop reliance for behavior-blind opencode.

**Verification against a populated new-schema opencode DB is PENDING.** Until a
real DB with step-finish `session_message` rows confirms the token shape, treat
the new-schema point-in-time path as assumed-correct-but-unconfirmed. If
verification fails, supersede this ADR with the corrected field mapping.

## Alternatives Considered

- **Keep part-only (ADR-005) and accept aggregate fallback on new schema.**
  Rejected — that IS the bug: cumulative ≠ window (ADR-002), and it silently
  degrades opencode's behavior-blind backstop.
- **Query only `session_message` (drop `part`).** Rejected — breaks older
  opencode DBs that still carry step-finish in `part`; the fallback is cheap and
  schema-version-agnostic.
- **Detect schema version up front.** Rejected — `prepare()` failure already
  distinguishes table presence per query; an explicit version probe adds a round
  trip and a version registry for no behavior gain.
