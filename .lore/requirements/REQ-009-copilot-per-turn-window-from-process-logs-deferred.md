---
id: REQ-009
title: Copilot per-turn window from process logs
status: Accepted
related_requirements: [REQ-002]
related_adrs: [ADR-002]
related_stories: []
related_tests: []
---

# REQ-009 - Copilot per-turn window from process logs

> Implemented (increment B). brim derives Copilot point-in-time window
> occupancy from process-log `CompactionProcessor` entries. Format below is
> VERIFIED-LIVE against a real session.

## Requirement

- The system derives Copilot **point-in-time** window fill by reading
  `~/.copilot/logs/process-<epochMs>-<pid>.log` `CompactionProcessor` entries,
  which record the running token count of the conversation context **before
  each model request** — i.e. per-turn occupancy.
- Per-turn line format (VERIFIED-LIVE):
  `CompactionProcessor: Utilization <pct>% (<used>/<limit> tokens) below threshold <thresh>%`.
  brim takes **only `<used>`** as the point-in-time occupancy; `<limit>`,
  `<pct>`, and `<thresh>` are ignored (absolute-token-only reasoning per
  ADR-011). The latest occupancy is the last `CompactionProcessor` line in the
  newest process log for the session.
- Session ↔ process-log linkage: the log filename embeds `<pid>`; the live
  session dir holds `inuse.<pid>.lock` with the same pid, tying a session to its
  process log.
- This per-turn occupancy is **absent from the source brim formerly relied on**:
  Copilot's `session-state/<id>/events.jsonl` persists only cumulative
  `session.shutdown` metrics. The token-bearing per-turn events
  (`assistant.usage`, `session.shutdown`) are **ephemeral — held in memory for
  `/usage`, never written to `events.jsonl`** (github/copilot-cli #1394). The
  process log is the only on-disk point-in-time source.

## Rationale

ADR-002's Copilot rationale ("Copilot is cumulative-only, so its fill is
approximate or unavailable") is **incomplete**: per-turn occupancy exists, but
in the process logs, not the persisted transcript. ADR-002's *decision* still
stands; only the Copilot-specific justification is narrowed by this finding.
This REQ is the record of that narrowing — ADR-002 (Accepted) is not edited.

### Sources

- github/copilot-cli #1394 — usage stats ephemeral, only shown on exit.
- tokentopapp/agent-copilot-cli — parses `~/.copilot/logs/process-*.log`
  `CompactionProcessor` pre-request token counts (point-in-time occupancy).

## Acceptance Criteria

- [x] Read the newest `process-<epochMs>-<pid>.log` for the session and take
      `<used>` from its last `CompactionProcessor: Utilization ... (<used>/<limit>
      tokens) ...` line as point-in-time window occupancy.
- [x] Ignore `<limit>`, `<pct>`, `<thresh>` (absolute-only per ADR-011).
- [x] Link session to process log via `<pid>` (`inuse.<pid>.lock`).
