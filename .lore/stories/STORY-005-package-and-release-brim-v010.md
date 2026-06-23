---
id: STORY-005
title: Package and release brim v0.1.0
status: Draft
related_requirements:
  - FEATURE-001
related_adrs: []
related_stories: []
related_tests: []
---

# STORY-005 - Package and release brim v0.1.0

> **Status: Draft — deferred future-work, scheduled 2026-06-24.**
> Code is functionally complete; recorded now so release hygiene is not lost.

## User Story

As a maintainer,
I want brim packaged for public release,
So that it can ship to GitHub / crates.io.

## Motivation

The code is functionally complete — 4 build increments, ADR-006/007/008/009/010 Accepted,
113 tests passing, `clippy`/`fmt` clean, and pushed. What is missing is release hygiene:
licensing, publish metadata, README accuracy, and a tagged release.

## Scope — release checklist

1. **LICENSE** — add a LICENSE file. Choose MIT or Apache-2.0 — **decision pending.**
2. **Cargo.toml `[package]` metadata** — fill the fields `cargo publish` requires/expects
   (`description`, `license`, `repository`, `keywords`, `categories`). Currently only
   `name` / `version` / `edition` are present.
3. **README.md accuracy pass** — verify it documents the current CLI flags
   (`--json`, `--all`, `--active-mins`, `--nearing`, `--ceiling`, `--watch-tokens`,
   `--recycle-backstop`) and matches the router template in AGENTS.md.
4. **Tag v0.1.0** with release notes.

## Sequencing consideration (non-goal here)

Consider landing **STORY-004 (compact `--json` output)** before promoting brim widely.
brim's primary consumer is an orchestrator agent, and the current ~2000-token output dents
the value prop. This is a **sequencing consideration, not a hard blocker** for the release
mechanics above.

## Acceptance Criteria

- [ ] A LICENSE file exists (MIT or Apache-2.0; decision recorded).
- [ ] `Cargo.toml [package]` has `description`, `license`, `repository`, `keywords`,
      `categories`; `cargo publish --dry-run` accepts the metadata.
- [ ] README.md documents all current CLI flags and matches the AGENTS.md router template.
- [ ] v0.1.0 is tagged with release notes.

## Related

- FEATURE-001 (brim context-window diagnostic) — the feature being released
  (recorded under `related_requirements` per this repo's story convention).
- STORY-004 (compact brim --json) — sibling story; story↔story links are rejected by the
  lore CLI, so noted here in the body. See "Sequencing consideration" above.
