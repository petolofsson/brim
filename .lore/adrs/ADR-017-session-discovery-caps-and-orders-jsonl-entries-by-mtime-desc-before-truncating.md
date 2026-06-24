---
id: ADR-017
title: Session discovery caps and orders .jsonl entries by mtime DESC before truncating
status: Draft
related_requirements:
  - FEATURE-001
related_adrs: []
related_stories:
  - STORY-003
related_tests: []
---

# ADR-017 - Session discovery caps and orders .jsonl entries by mtime DESC before truncating

## Context

Session discovery (`discover_project` in `src/claude.rs`) bounds the number of
transcript files read per project at `MAX_FILES_PER_PROJECT` (64) to keep the
diagnostic cheap on large `~/.claude/projects/<project>/` directories.

The prior implementation truncated the result of an **unordered** `read_dir`
walk. Directory iteration order is filesystem-dependent and non-deterministic, so
the cap silently dropped an arbitrary subset of `.jsonl` files — including, in the
worst case, the **live/active session** the orchestrator most needs. This was
found during STORY-003 real-session validation: a correctness defect (the active
session could be excluded) compounded by a determinism defect (different runs
could keep different sessions).

## Decision

Before applying `MAX_FILES_PER_PROJECT`, sort the project's `.jsonl` entries by
modification time **descending** (newest first), with a deterministic tiebreak:

1. mtime DESC;
2. on an unreadable mtime, fall back to `UNIX_EPOCH` (sorts oldest, i.e. dropped
   first);
3. final tiebreak on filename.

Then truncate to the cap. The cap therefore deterministically retains the
newest-N sessions and always includes the active session (highest mtime).

Code site: `discover_project` in `src/claude.rs`.

## Consequences

- Deterministic discovery: the same directory state always yields the same kept
  set, and the active session is never dropped by the cap.
- Trade-off: an O(N) metadata `stat` of every candidate `.jsonl` before
  truncation, versus the prior truncate-then-stat. Cost is bounded and small
  relative to reading the retained transcripts.

## Alternatives Considered

- **Raise/remove the cap** — does not fix determinism and unbounds cost on large
  histories.
- **Truncate unordered (status quo)** — non-deterministic; can drop the live
  session. Rejected as the defect being corrected.
