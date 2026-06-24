---
id: ADR-005
title: opencode point-in-time window from step-finish with aggregate fallback
status: Superseded
related_requirements:
  - REQ-008
related_adrs:
  - ADR-002
  - ADR-015
related_stories: [STORY-009]
related_tests:
  - TEST-004
---

# ADR-005 - opencode point-in-time window from step-finish with aggregate fallback

> **Superseded by ADR-015** — the 200k limit assumption is corrected to 1,000,000 for `z-ai/glm-5.2`. The step-finish oracle, `parent_id` join, and aggregate-fallback design below remain in force.

## Context

opencode (v1.17.5 observed) stores session transcripts in a SQLite database at
`$HOME/.local/share/opencode/opencode.db`, not JSONL. The orchestrator inspected
this database before writing the spec; the facts below are observed, not
assumed.

* The `session` table has aggregate token columns
  (`tokens_input`, `tokens_cache_read`, `tokens_cache_write`, `tokens_output`,
  `tokens_reasoning`) — per-session cumulative totals.
* The `part` table carries the per-turn detail. Rows whose `data` JSON has
  `type == "step-finish"` include a `tokens` object:
  `{ total, input, output, reasoning, cache: { write, read } }`, plus a `time`
  field and the row's own `time_created`.
* `message` / `part` `data` is populated only after opencode checkpoints a
  session; pre-checkpoint rows carry empty `data` / empty aggregates.
* The `session_context_epoch` compaction ledger is currently empty in this
  install; its modeling is out of scope.
* No `parent_id` sessions exist yet (sub-agent support is structural but
  unpopulated).

ADR-002 established the principle: brim reports the point-in-time window, not
cumulative spend. ADR-002 also explicitly allows the "approximate or unavailable"
case when a provider does not emit per-turn cumulative prompt tokens.

## Decision

For opencode, brim's point-in-time oracle is the latest `part` row for a session
with `json_extract(data,'$.type')='step-finish'`, ordered by `time_created DESC
LIMIT 1`. Field mapping to claude's last-turn usage (and to ADR-002's formula):

* opencode `input`           ↔ claude `input_tokens`
* opencode `cache.read`       ↔ claude `cache_read_input_tokens`
* opencode `cache.write`      ↔ claude `cache_creation_input_tokens`
* `window_tokens = input + cache.read + cache.write` (saturating add, same as
  claude).

When (and only when) no `step-finish` part exists for a session, brim falls back
to the `session` aggregate columns (`tokens_input + tokens_cache_read +
tokens_cache_write`) and tags the resulting `WindowInfo` with
`window_source = Aggregate` (vs `LastTurn` for the step-finish oracle). The tag
is exposed in the `--json` output so a consumer can distinguish point-in-time
from cumulative occupancy — ADR-002 explicitly warns cumulative ≠ window, so
brim must not silently substitute one for the other.

Model context-window limit: `z-ai/glm-5.2` (the only model observed, via the
`llmbase` provider) is **not stored in the opencode db or config**. Decision
(user-confirmed): treat it as the existing **200_000** default — the same value
claude's `window_limit` returns for any model without the `[1m]` marker. No
model→limit registry is introduced; opencode model ids (`z-ai/glm-5.2`,
`deepseek/deepseek-v4-pro`, …) carry no `[1m]` marker and fall through to the
default automatically. A registry is revisit-when-needed, not now.

`parent_id` on `session` is the sub-agent join key (analog of claude's
`<uuid>/subagents/`). brim joins on it exactly as claude's `child_map` does;
until opencode spawns sub-agents this produces flat trees. No children are
synthesized.

## Consequences

* Reported fill reflects current window pressure for any session that has had at
  least one step-finish checkpoint; for pre-checkpoint sessions the fill is
  explicitly surfaced as `window_source = Aggregate` (an ADR-002 permitted
  approximation).
* `cache.read` re-counts the cached prefix on every turn in the aggregate case
  (ADR-002 caveat) — the `Aggregate` tag lets a consumer downgrade such rows.
* Reading SQLite read-only via `rusqlite`+`bundled` keeps the build deterministic
  and respects CODERULES r11 security (no read-write, no network).
* The 200k assumption for `z-ai/glm-5.2` is documented; if a future model has a
  different limit, ADR-005 is superseded (this ADR stays Draft until confirmed in
  production, then transitions to Accepted).
* opencode compaction (`session_context_epoch`) is not modeled; revisit only if
  compaction starts non-trivially affecting the post-compact aggregate baseline.

## Alternatives Considered

- **Always use the aggregate columns.** Rejected — conflates cumulative spend
  with window occupancy (ADR-002's whole point). The step-finish part is the
  correct point-in-time oracle and is reliably present post-checkpoint.
- **Build a model→limit registry.** Rejected for v1 — only one model is observed
  and its limit (200k) is already the default. A registry adds surface area
  without behavior; revisit when a non-200k model appears.
- **Copy/checkpoint the SQLite db before reading.** Rejected — read-only rusqlite
  handles WAL natively without touching the user's opencode state.
- **Edit ADR-002.** Rejected — ADR-002 is Accepted and states the principle;
  opencode is a per-provider implementation detail that fits under it. A new ADR
  references rather than rewrites it.