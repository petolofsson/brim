---
id: ADR-028
title: Opencode Behavior family extraction from part tool rows
status: Accepted
related_requirements:
  - REQ-016
related_adrs:
  - ADR-024
  - ADR-025
related_stories:
  - STORY-012
related_tests: []
---

# ADR-028 - Opencode Behavior family extraction from part tool rows

## Context

ADR-024 realized signal (a) "behavioral degradation" as a deterministic gate
fed by tool-call STRUCTURE, and ADR-025 made the Behavior family a weighted
voter in the recycle verdict. Until now only Claude and codex were wired; the
opencode parser carried a `behavior:None` STUB — the SQLite transcript exposed
the tool structure (recycle-research-findings.md §3) but brim never consumed
it, so the Behavior family could never fire for opencode.

This ADR records the decision and the exact verified shape for extracting the
Behavior family from opencode transcripts, removing the stub.

## Decision

Extract the Behavior family for opencode from the `part` table tool rows.
**LIVE-VERIFIED on opencode 1.17.9.** Exact shape:

- **Source / discriminator:** tool rows live in the `part` table; type
  discriminator `json_extract(data, '$.type') = 'tool'`.
- **Tool name:** `data.tool`.
- **Args object:** `data.state.input` (a JSON **object**, hashed for
  repetition detection — unlike codex, whose args are a JSON string).
- **Error flag:** `data.state.status == 'error'`.
- **Ordering:** `ORDER BY time_created DESC, id DESC LIMIT K` (K = `TREND_TAIL_K`
  = 8), then **reversed back to chronological** so the analysis runs over the
  LAST K tool calls — the recent tail, matching the claude/codex windowing.
- Feeds `BehaviorSignals::from_signals`. The ping-pong no-progress qualifier
  (`name[i] == name[i+2] && argshash[i] == argshash[i+2]`) is **inherited from
  the shared verdict logic** — it is NOT a naive A-B alternation detector.
- **Fail-closed:** missing/malformed rows yield `None` rather than panic or
  error (REQ-015 invariant; consistent with the rest of `opencode.rs`).

Behavior now fires for opencode in the family vote count (ADR-025). The
deterministic / transcript-tokens-only / no-content-inspection invariants are
preserved — only tool-block STRUCTURE is read.

## Consequences

- The `behavior:None` opencode stub is **gone**; opencode joins Claude and
  codex as a behavior-wired provider. Three of four providers now contribute a
  real Behavior vote.
- **copilot remains structurally behavior-blind.** Its process-log source
  carries NO tool structure (no per-tool rows, no args, no error flag). This is
  a **permanent FORMAT limitation, not a stub** — there is nothing to wire. Its
  Behavior family can never fire regardless of brim changes (ADR-025's
  behavior-blind-provider handling applies).
- Args-as-object (opencode/Claude) vs args-as-string (codex) divergence is
  isolated to the per-provider extractor; the shared `from_signals` logic is
  uniform.

## Alternatives Considered

- **Keep the `behavior:None` stub.** Rejected — the format was verified present
  on 1.17.9; leaving it stubbed kept opencode falsely behavior-blind and
  understated the family vote count.
- **Read `session_message` instead of `part` for tool rows.** Rejected —
  step-finish/tool rows live in `part` on every observed version (ADR-027); the
  `session_message` preference is a forward-compat fallback for token windows,
  not the behavior source.
- **Naive A→B→A ping-pong detector.** Rejected — the shared `from_signals`
  qualifier already requires same-name AND same-args at stride 2, so opencode
  reuses it rather than reimplementing a weaker check.

<!-- Records the shipped opencode Behavior wiring. Realizes [[STORY-012]];
extends [[ADR-024]] (the behavioral gate) and feeds [[ADR-025]] (the
vote-counter family count); consumes the candidate catalog [[REQ-016]].
LIVE-VERIFIED opencode 1.17.9. copilot stays behavior-blind by FORMAT. -->
