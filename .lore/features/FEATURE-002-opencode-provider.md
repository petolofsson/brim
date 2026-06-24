---
id: FEATURE-002
title: opencode provider
status: Accepted
related_requirements:
  - REQ-002
  - REQ-008
related_adrs:
  - ADR-002
  - ADR-005
related_stories: [STORY-009]
related_tests:
  - TEST-004
---

# FEATURE-002 - opencode provider

## Feature

Add an **opencode** provider to `brim` alongside the existing claude provider,
so `brim` reports context-window occupancy for opencode sessions (current model
observed: `z-ai/glm-5.2` via the `llmbase` provider). Claude stays untouched;
both providers are enumerated in `main.rs`, merging their sessions into one
output list.

## Scope

- New `Cargo.toml` dependency: `rusqlite` with the `bundled` feature
  (stdlib has no SQLite; bundling avoids a system `libsqlite3` dependency, per
  CODERULES r2 deterministic builds).
- New `src/opencode.rs`: `OpencodeProvider` + discovery + last-turn parse,
  mirroring `src/claude.rs` structure. Reads
  `$HOME/.local/share/opencode/opencode.db` read-only (CODERULES r11).
- `src/window.rs` (NEW): extracted `window_limit` + `compute_window_info`
  helpers, parameterised by `WindowSource`. `claude` reuses them with
  `LastTurn`; opencode uses `LastTurn` for the step-finish oracle and
  `Aggregate` for the session-column fallback.
- `src/model.rs`: add `WindowSource` enum (`LastTurn` | `Aggregate`) and a
  `window_source` field on `WindowInfo` so JSON exposes provenance (REQ-005
  machine-readable).
- `src/main.rs`: wire `OpencodeProvider` next to `ClaudeProvider`; merge
  sessions; add `window_source` to the JSON output.

## Non-goals

- No watch mode, no TUI, no compaction modeling.
- No codex / copilot providers (FEATURE-001 lists those as future work).
- No model→context-limit registry. `z-ai/glm-5.2` uses the existing 200_000
  default (user-confirmed; documented in ADR-005). Revisit only when a model
  with a non-200k limit is added.
- No new formatter/linter; `cargo fmt` + `cargo clippy --all-targets -- -D
  warnings` on changed code only.
- Do NOT edit any Accepted ADR (ADR-002 stays Accepted; opencode behavior is a
  NEW ADR-005 referencing ADR-002's principle).

## Included Artifacts

- REQ-008 — behavior: opencode SQLite transcript source, step-finish oracle,
  aggregate fallback, `window_source` exposure.
- ADR-005 — Draft: opencode point-in-time window from `step-finish` with
  aggregate fallback; references ADR-002.
- TEST-004 — expected outcome: step-finish oracle window math, aggregate
  fallback provenance tag, and `parent_id` sub-agent tree join.