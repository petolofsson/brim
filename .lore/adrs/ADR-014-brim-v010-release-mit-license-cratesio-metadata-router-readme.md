---
id: ADR-014
title: "brim v0.1.0 release: MIT license, crates.io metadata, router README"
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs: []
related_stories:
  - STORY-005
related_tests: []
---

# ADR-014 - brim v0.1.0 release: MIT license, crates.io metadata, router README

## Context

STORY-005 deferred the release hygiene for brim v0.1.0. The code is functionally
complete: ADR-006..011 are Accepted, 116 tests are green, and the STORY-004 slim
`--json` output has shipped. What remained was the packaging surface a public
release on GitHub / crates.io expects: a LICENSE, `Cargo.toml` `[package]`
publish metadata, and a README that accurately reflects the current CLI.

## Decision

- **LICENSE**: MIT, copyright `Copyright (c) 2026 Pol Olofsson`. MIT chosen
  (user-confirmed); it is compatible with all of brim's current dependencies
  (anyhow, chrono, clap, serde, serde_json, rusqlite).
- **`Cargo.toml [package]`**: filled with `description`, `license = "MIT"`,
  `repository = "https://github.com/petolofsson/brim"`, `keywords`, and
  `categories`. No `authors`, `homepage`, `documentation`, or `readme` fields
  are added yet (deferred — `cargo publish` does not require them).
- **README.md**: kept as the AGENTS.md-mandated router template (flat router,
  no changelog, no features section). Only the `## Run` block was updated to
  list the current CLI surface (`--tree`, `--session <id>`, `--json`, `--all`,
  `--active-mins <N>`, `--watch-tokens <N>`, `--recycle-backstop <N>`,
  `--once`; default = active-only flat list). The `## Map` targets were
  verified correct (`CLAUDE.md` and `CODERULES.md` both exist; lore for
  requirements/design/decisions).

## Consequences

- `cargo publish --dry-run` accepts the metadata (verified).
- brim is publishable to crates.io on demand once the user tags and pushes.
- The 200k context-window assumption for z-ai/glm-5.2 (ADR-005, Draft) and the
  deferred provider-verification STORY-003 remain open and do not block v0.1.0.
- The README stays a router; any future CLI flag changes require a matching
  `## Run` edit (no prose expansion per AGENTS.md).

## Non-goals

- No `git tag v0.1.0` in this delegation — the user tags after review.
- No `cargo publish` (dry-run only here).
- No README prose expansion beyond the router template.

## Alternatives Considered

- **Apache-2.0**: also dependency-compatible, but MIT was user-confirmed and is
  shorter / more conventional for small single-purpose CLIs.
- **Adding `authors` / `homepage` / `documentation` / `readme` fields now**:
  rejected; `cargo publish` does not require them and they can be added when
  the user fills in the GitHub repo surface.