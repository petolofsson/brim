---
id: ADR-031
title: Failure-streak keys on operation/exit-code failure, not a status discriminator
status: Accepted
related_requirements:
  - REQ-016
related_adrs:
  - ADR-024
  - ADR-028
  - ADR-030
related_stories:
  - STORY-012
related_tests: [TEST-012]
---

# ADR-031 - Failure-streak keys on operation/exit-code failure, not a status discriminator

## Context

ADR-024 realized signal (a) "behavioral degradation" as a deterministic gate fed
by tool-call STRUCTURE; one of its Tier-B signals (REQ-016) is the **failure
streak** — N consecutive failed tool calls. The shared `BehaviorSignals::from_signals`
(`verdict.rs:117`) was designed for the real stuck symptom: its contract doc
(`verdict.rs:104`) reads "is_error / status=failed / exit-code != 0". The per-provider
extractors, however, each picked a single discriminator and several diverged from
that contract:

- **opencode** keyed only on `data.state.status == 'error'` (ADR-028). Real-data
  inspection shows this is the WRONG event for a stuck agent: a bash/exec tool whose
  COMMAND fails keeps `state.status == 'completed'` and records the non-zero code in
  `state.metadata.exit`. The `status=='error'` path fires only on tool-INVOCATION
  failure, not on operation/exit-code failure — so the most common "stuck" symptom
  (a command failing over and over) never raised the streak.
- **codex** keyed on `function_call_output.status == 'failed'` (ADR-030). ADR-030
  recorded this as "synthetic-validated; real-failure pending." Real codex data now
  shows the discriminator **does not exist**: real `function_call_output` rows carry
  only `call_id`, `output`, `type` — there is NO `status` field at all. The exit code
  appears only inside the free-text `output` string ("Process exited with code N"),
  which CODERULES r11 (no content inspection) forbids parsing.
- **claude** already keys on the `tool_result.is_error` boolean, which real
  transcripts confirm reflects exit-code failure.

The token gates, projection, cache-thrash, and the read-only/advisory boundary
(ADR-010) are untouched; this is purely a correction to which structural field the
failure-streak reads, aligning each provider to the `from_signals` contract.

## Decision

The failure-streak signal keys on **operation / exit-code failure** wherever it is
structurally available — not on a tool-invocation `status` discriminator alone. This
**supersedes ADR-024's** implicit single-discriminator framing and **supersedes
ADR-030's** codex error-path claim. Exact per-provider rules, all LIVE-VERIFIED
against real transcripts except where noted:

- **opencode** (`opencode.rs::extract_opencode_behavior`): error flag fires on
  `data.state.status == 'error'` **OR** (`data.state.metadata.exit` present as an i64
  **AND** != 0). LIVE-VERIFIED: a failed bash command keeps `status == 'completed'`
  but sets `metadata.exit` to the non-zero code. Fail-closed: missing/malformed
  `metadata.exit` => not-error.
- **claude** (`claude.rs`): unchanged — `tool_result.is_error` already reflects
  exit-code failure. LIVE-VERIFIED from real `~/.claude` transcripts: a non-zero bash
  exit sets `is_error == true` and the content begins "Exit code N". No code change.
- **codex** (`codex.rs::extract_codex_behavior`): **DOCUMENTED LIMITATION.** Real
  `function_call_output` rows have NO `status` field (keys: `call_id`, `output`,
  `type`), and the exit code lives only in the free-text `output` string, which r11
  forbids inspecting. Therefore `error_flags` are always false and **failure_streak
  CANNOT fire for real codex sessions.** The legacy `status == 'failed'` path is
  harmless spec-legacy. No code change. codex Behavior still fires on
  repetition / ping-pong (ADR-030) — only the failure-streak sub-signal is dark.

### Correction to ADR-030 (supersede-in-prose; ADR-030 not edited)

ADR-030 (Accepted, committed) states the codex error path is "SYNTHETIC-ONLY … the
discriminator is unconfirmed against a real failed turn … Real-failure codex data is
PENDING." That framing implied the `status=='failed'` discriminator would validate
once real failure data arrived. The real finding is stronger and different: the
`status` field **does not exist** in real `function_call_output` rows at all, so the
discriminator is not "pending confirmation" — it is **structurally absent**. This ADR
records that correction; per the never-edit-an-Accepted-ADR rule, ADR-030 itself is
left unchanged and superseded in prose here.

## Consequences

- A real opencode agent stuck on a repeatedly-failing command now raises the
  failure-streak (the true stuck symptom), closing a gap where ADR-028's
  `status=='error'`-only rule read such a session as healthy.
- codex failure-streak is a known dark spot: repetition/ping-pong still cover codex
  loops, but consecutive command failures with no repetition will not trip Behavior
  for codex until a structural exit field appears in the format.
- The deterministic / transcript-tokens-only / no-content-inspection invariants hold:
  opencode reads an integer `metadata.exit`, claude a boolean, neither inspects
  free-text output.

### Forward-looking: copilot is PENDING WIRING, not behavior-blind

A just-completed real-data inspection found copilot is **NOT** permanently
behavior-blind. Its `~/.copilot/session-state/<uuid>/events.jsonl` carries
`tool.execution_start` (with `toolName`, `arguments`) and `tool.execution_complete`
(with a `success` boolean) events — exactly the tool STRUCTURE and operation-failure
signal the Behavior family needs. brim does not yet parse this file; it currently
reads only the copilot occupancy process-log. So copilot's failure/Behavior signal is
a **brim parsing gap (PENDING WIRING)**, not a format limitation. The full copilot
wiring and the corresponding corrections to other artifacts (ADR-024/ADR-028/STORY-012
copilot claims) are a SEPARATE upcoming unit — this ADR only records the forward note.

## Alternatives Considered

- **Parse the codex/copilot free-text output for "exited with code N".** Rejected:
  violates CODERULES r11 (no content inspection) and the transcript-structure-only
  invariant.
- **Keep opencode on `status=='error'` only.** Rejected: misses the dominant
  command-failure symptom, which keeps `status=='completed'`.
