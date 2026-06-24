---
id: STORY-005
title: Package and release brim v0.1.0
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-014
related_stories: []
related_tests: []
---

# STORY-005 - Package and release brim v0.1.0

> **Status: Accepted — shipped 2026-06-24 via ADR-014.**
> Release-hygiene deliverables complete (LICENSE MIT, Cargo.toml metadata, README, ADR-014). The v0.1.0 git tag is the user's post-review release act, not a brim deliverable.

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

- [x] A LICENSE file exists (MIT or Apache-2.0; decision recorded).
- [x] `Cargo.toml [package]` has `description`, `license`, `repository`, `keywords`,
      `categories`; `cargo publish --dry-run` accepts the metadata.
- [x] README.md documents all current CLI flags and matches the AGENTS.md router template.
- [ ] v0.1.0 is tagged with release notes.

> The v0.1.0 tag is the user's post-review release act, not a brim deliverable; hygiene work is complete (ADR-014).

## Related

- FEATURE-001 (brim context-window diagnostic) — the feature being released
  (recorded under `related_requirements` per this repo's story convention).
- STORY-004 (compact brim --json) — sibling story; story↔story links are rejected by the
  lore CLI, so noted here in the body. See "Sequencing consideration" above.
