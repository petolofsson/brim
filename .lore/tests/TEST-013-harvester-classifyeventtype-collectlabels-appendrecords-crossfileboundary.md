---
id: TEST-013
title: "Harvester: classify_event_type, collect_labels, append_records, cross_file_boundary"
status: Draft
related_requirements:
  - REQ-017
related_adrs: [ADR-033]
related_stories:
  - STORY-013
related_tests: []
---

# TEST-013 - Harvester: classify_event_type, collect_labels, append_records, cross_file_boundary

## Test Case

Unit coverage for the REQ-017 / STORY-013 append-only recycle-label harvester
(`src/harvest.rs`). Deterministic, transcript-only, no content inspection, no
network. Four groups:

1. **`classify_event_type` table** — pure discriminator over
   (`compact_boundary_present`, `stop_reason_max_tokens`, `occupancy_tokens` vs
   `ABSOLUTE_RECYCLE_BACKSTOP`).
2. **`collect_labels` over a synthetic in-session drop** — one `SessionNode`
   with a single `window_tokens` drop in `trend.points`.
3. **`append_records` append-only / idempotent** — two writes to a tempfile.
4. **`cross_file_boundary`** — two same-`(provider, project_key)` synthetic
   sessions: earlier terminal occupancy `>= 96k` gate, a later same-project
   session present (N4: presence sufficient; no B-side fresh-start check).

## Expected Result

(a) **`classify_event_type` table** — exact mapping:

| compact_boundary | stop_reason=max_tokens | occupancy vs backstop | event_type |
|---|---|---|---|
| Yes | Yes | any | `compaction` |
| Yes | No | `< backstop` | `recycle` |
| Yes | No | `>= backstop` | `compaction` (conservative) |
| No | Yes | any | `compaction` |
| No | No | any (incl. `>= backstop`) | `ambiguous` (Q5: high occupancy + no markers = ambiguous, NOT compaction) |

(b) **`collect_labels` synthetic in-session drop** — yields exactly one
well-formed `HarvestRecord`: `schema_version = 1`, `boundary_kind =
"in_session"`, `occupancy_tokens` = the last pre-drop `window_tokens`,
`drop_magnitude` = pre − post, `family_fires` a length-5 `[bool; 5]`,
`event_type` per the table above, and provenance (`provider`, `session_id`,
`project_key`, `reset_turn_index`, `n_turns_in_window`) populated.

(c) **`append_records` append-only / idempotent** — calling twice on the same
tempfile appends without truncation: file line count = sum of records across
both calls (prior lines preserved), and every line parses back as a valid
`HarvestRecord`. Never rewrites or deletes prior records.

(d) **`cross_file` boundary** — two synthetic `SessionNode`s sharing one
`(provider, project_key)`, ordered by `last_turn_at`: a cross-file recycle is
detected when the earlier session's terminal `window_tokens >= 96k` (the
harvest gate, `HARVEST_RECYCLE_GATE_TOKENS`) AND a later same-project session
EXISTS. Presence of the later session is sufficient (N4) — there is NO B-side
fresh-start ratio check on the later session's first point. Result: **exactly
one** `HarvestRecord` with `event_type = "recycle"`, `boundary_kind =
"cross_file"`, `session_id` = the EARLIER session's id (anchored to where the
operator judged it time to recycle), `compact_boundary_marker = false`, and
`stop_reason_max_tokens_in_window = false`. A below-gate earlier terminal
(`< 96k`) yields no cross-file record.

`ambiguous` rows are stored for provenance but excluded from calibration
consumers (filter `event_type in ("recycle", "compaction")`).

Validation: `cargo test harvest -- --nocapture` and
`cargo clippy --all-targets -- -D warnings` clean.
