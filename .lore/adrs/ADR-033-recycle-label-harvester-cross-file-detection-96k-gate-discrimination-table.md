---
id: ADR-033
title: "Recycle-label harvester: cross-file detection, 96k gate, discrimination table"
status: Accepted
related_requirements: [REQ-017, REQ-016]
related_adrs: []
related_stories: [STORY-013]
related_tests: [TEST-013]
---

# ADR-033 - Recycle-label harvester: cross-file detection, 96k gate, discrimination table

## Context

REQ-017 / STORY-013 add an append-only ground-truth recycle-label harvester
(`src/harvest.rs`) that feeds offline calibration of the ADR-025 vote-counter
thresholds. REQ-017 fixes WHAT is captured and the collection invariants; this
ADR records the DESIGN DECISIONS the implementation makes inside that boundary,
consistent with the per-feature ADR pattern (ADR-028 opencode, ADR-030 codex,
ADR-032 copilot). The harvester PRODUCES labels only; the live recycle verdict
and the ADR-025 candidate thresholds are unchanged.

The harvester runs per-provider over whatever local transcripts exist
(graceful degradation, REQ-017); a window reset is the same event REQ-007
already detects. Several point decisions (the recycle/compaction discriminator,
the cross-file detection mechanism and its gate, the persistence sink) were not
fully pinned by REQ-017 and are settled here. One of them — brim itself writing
the label file — sits in tension with ADR-004 and is reconciled below.

## Decision

**Dual detection paths.** A label is produced two ways, tagged by
`boundary_kind`:
- `in_session` — a REQ-007 timeline drop within one transcript (reset detection
  reused via `window::find_reset_indices`; no new mechanism, drop magnitude
  discarded as a live signal per REQ-016).
- `cross_file` — a same-`(provider, project_key)` session boundary: an earlier
  session that ended at high occupancy followed by a later session of the same
  project. This is the PRIMARY positive recycle label of STORY-013 (the
  operator started fresh at a break).

**Cross-file gate — `HARVEST_RECYCLE_GATE_TOKENS = 96_000` (N3).** The earlier
session's TERMINAL `window_tokens` must be `>= 96k` to qualify the boundary as
a recycle. The operator recycles at ~100k–300k tokens; 96k sits just below that
band, admitting genuine recycles at the 100k–127k low end while rejecting
new-task boundaries that merely start at moderate occupancy. Chosen over the
32k watch threshold (too loose) and the 128k backstop (would miss 100k–127k
recycles).

**N4 — later-session presence is sufficient; no B-side check.** Once the earlier
terminal occupancy clears the 96k gate, the mere EXISTENCE of a later
same-project session marks the recycle. The originally-proposed "later session
starts fresh" ratio test (`later.points[0] < earlier_terminal × 0.5`) is
DROPPED: a long later session's first loaded point is the `(n-K)th` turn (the
bounded tail K, not the true first turn), so the ratio test would spuriously
reject real recycles. The implementation omits it.

**Recycle / compaction / ambiguous discrimination table (in_session, Claude
primary).** Discriminate on `compact_boundary` presence,
`stop_reason=max_tokens` in the pre-reset window, and occupancy vs the absolute
backstop:

| `compact_boundary` | `stop_reason=max_tokens` | occupancy vs backstop | `event_type` |
|---|---|---|---|
| Yes | Yes | any | `compaction` |
| Yes | No | `< backstop` | `recycle` |
| Yes | No | `>= backstop` | `compaction` (conservative — host likely drove it) |
| No | Yes | any | `compaction` |
| No | No | any (incl. `>= backstop`) | `ambiguous` |

**Q5 — no markers ⇒ ambiguous, never compaction.** A drop with no
`compact_boundary` and no `stop_reason=max_tokens` is `ambiguous` even at high
occupancy — high occupancy alone is not evidence of a host wall-hit.
`ambiguous` rows are stored for provenance but EXCLUDED from calibration
consumers (filter `event_type in ("recycle", "compaction")`).

**Claude `compact_boundary` second bounded read.** `compact_boundary` events
are not stored on `SessionNode`, so each Claude session with at least one drop
gets a SECOND bounded `read_tail()` (256 KiB cap, `TAIL_CAP_BYTES`) that scans
for `{"type":"system","subtype":"compact_boundary"}`. The re-read is
STRUCTURAL-only — it inspects the event type/subtype, never natural-language
content (REQ-016 / ADR-025 no-content-inspection invariant preserved).

**`--harvest` requires explicit `--all` (Q3).** `--harvest <PATH>` does not
imply `--all`; passed alone it scans only the active-session window. `--help`
documents that calibration wants `--all --harvest <path>` to reach stale
sessions where historical resets live. The harvester never runs by default.

**No `harvested_at` (Q4).** Records carry no wall-clock timestamp; provenance is
`(provider, session_id, project_key, reset_turn_index, boundary_kind)`. Same
transcript in ⇒ same record out (determinism, AGENTS.md goal 2).

**Grouping keyed on `(provider, project_key)` (N1).** `SessionNode` gained a
`provider: String` field (set in each provider's loader) so cross-file grouping
keys on `(provider, project_key)` and never pairs sessions across providers;
each `HarvestRecord` carries the correct `provider`. Groups are sorted by
`last_turn_at` ascending with `session_uuid` as a stable tiebreak (N2:
`last_turn_at` — terminal turn time — is sufficient for cross-file ordering;
no file-mtime field was added).

**Append-only consumer-owned sink — relationship to ADR-004 (SUPERSEDE in
prose).** `append_records` opens the operator-supplied `--harvest` path with
`append(true).create(true)` and never truncates, rewrites, or deletes prior
records. This is brim itself writing a file, which the literal text of ADR-004
("brim stays read-only and stateless ... never writes a history or summary
store"; "brim maintains its own ... file — REJECTED") does NOT permit — ADR-004
assigns persistence to the CONSUMER (the orchestrator saving snapshots), so
REQ-017's / STORY-013's claim that ADR-004 "explicitly permits" a brim-written
sink over-reads it. ADR-004 is Accepted and is not edited. This ADR EXTENDS it:
the guarantee ADR-004 actually protects — brim never mutates, recycles, or
writes to any session/transcript — is preserved in full. brim gains exactly ONE
narrow, operator-gated, off-by-default, append-only write surface (the
`--harvest` calibration label log at a user-specified path outside all
transcript/session state). ADR-004's "no new write surface" consequence is
narrowed to "no new write surface over sessions/transcripts; one explicit
opt-in calibration sink is allowed."

## Consequences

* The read-only-over-sessions invariant (ADR-002 / ADR-025 / REQ-017) is intact:
  no transcript is ever written; the only write is the opt-in label log.
* Calibration is additive and deterministic: re-running over the same
  transcripts re-derives identical records; enabling more providers only
  enriches the same store.
* `ambiguous` rows accrue but are filtered out by calibration consumers, so the
  conservative discrimination never poisons the training target.
* The 96k gate and N4 presence rule trade a small false-positive risk
  (new-task boundaries above 96k) for not missing operator recycles; consumers
  can re-filter on `occupancy_tokens` if a tighter band is wanted later.
* ADR-004 must be read together with this ADR: brim is read-only over sessions,
  not write-free at the process level.

## Alternatives Considered

- **Edit ADR-004 to permit the sink** — REJECTED: ADR-004 is Accepted; never
  edit an Accepted ADR. Superseded in prose here instead.
- **32k watch gate or 128k backstop for cross-file** — REJECTED: 32k admits
  low-pressure new-task boundaries; 128k misses operator recycles at 100k–127k.
  96k brackets the operator's ~100k–300k habit (N3).
- **Keep the B-side fresh-start ratio check** — REJECTED (N4): the later
  session's first LOADED point is the bounded-tail `(n-K)th` turn, not its true
  first turn, so the ratio test rejects real recycles.
- **Classify no-marker high-occupancy drops as `compaction`** — REJECTED (Q5):
  occupancy alone is not wall-hit evidence; such rows are `ambiguous` and
  excluded from calibration.
- **Add `harvested_at`** — REJECTED (Q4): breaks determinism for no provenance
  gain.

<!-- lore does not support adr<->adr links, so the ADR-004 (supersede-in-prose),
ADR-025 (vote-counter families consumed), ADR-011 (absolute tokens only),
ADR-002 (read-only sessions) and REQ-007 (reuse reset detection) relations are
recorded in the prose above. Linked via the CLI: REQ-017, REQ-016, STORY-013,
TEST-013. -->

