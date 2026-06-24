---
id: STORY-009
title: Discover opencode sessions from a read-only SQLite DB
status: Accepted
related_requirements:
  - REQ-008
  - FEATURE-002
related_adrs: [ADR-005]
related_stories: []
related_tests: [TEST-004]
---

# STORY-009 - Discover opencode sessions from a read-only SQLite DB

## User Story

As a brim-driven orchestrator,
I want brim to discover opencode sessions from its SQLite DB (strictly
read-only) and report each session's point-in-time window with provenance,
So that I can self-diagnose opencode sub-agent contexts without brim
mutating the database.

Shipped behavior (satisfies REQ-008): `src/opencode.rs::OpencodeProvider`
discovers sessions from `$HOME/.local/share/opencode/opencode.db` opened
read-only (`SQLITE_OPEN_READ_ONLY`); `step_finish_oracle` reads the latest
`part` row of `type=step-finish`; `aggregate_window` falls back to the
`session` cumulative columns and tags `window_source = Aggregate`; `parent_id`
joins children under the parent uuid; project key resolves from
`project.name` else the `directory` basename; `is_available` is a cheap file
check. Wired in `src/main.rs:327` (`OpencodeProvider::new`),
`rusqlite` `bundled` in `Cargo.toml:19`. Design in ADR-005/015; TEST-004 covers
the 7 cases.

## Acceptance Criteria

- [x] Sessions discovered read-only from the opencode SQLite DB.
- [x] Unavailable (not error) when the DB file is absent.
- [x] Last-turn window from the latest step-finish part; aggregate fallback
      tagged `window_source = "aggregate"`.
- [x] Project key resolved from `project.name`, else directory basename.
- [x] parent→child tree via `session.parent_id`; no synthesized children.
- [x] `window_source` exposed in `--json` (REQ-005).
