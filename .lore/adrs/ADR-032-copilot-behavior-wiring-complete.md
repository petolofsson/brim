---
id: ADR-032
title: copilot Behavior wiring complete
status: Accepted
related_requirements:
  - REQ-016
related_adrs:
  - ADR-024
  - ADR-025
  - ADR-028
  - ADR-031
related_stories:
  - STORY-012
related_tests:
  - TEST-012
---

# ADR-032 - copilot Behavior wiring complete

## Context

ADR-024 realized signal (a) "behavioral degradation" as a deterministic gate fed by
tool-call STRUCTURE, and ADR-025 made the Behavior family a weighted voter in the
recycle verdict. Of the four providers, claude/codex/opencode were wired
(ADR-024/ADR-028/ADR-030/ADR-031); copilot was the last holdout.

Two Accepted ADRs recorded copilot's status, and both are now false:

- **ADR-028** declared "**copilot remains structurally behavior-blind** … a
  **permanent FORMAT limitation, not a stub** … Its Behavior family can never fire
  regardless of brim changes." This was based on copilot's occupancy process-log,
  which indeed carries no tool structure.
- **ADR-031**'s forward note corrected that to "copilot is **PENDING WIRING**, not
  behavior-blind": a real-data inspection found copilot's
  `~/.copilot/session-state/<uuid>/events.jsonl` carries tool STRUCTURE — the format
  limitation claim was wrong, the gap was a brim parsing gap. ADR-031 deferred the
  actual wiring (and the corrections to ADR-024/ADR-028/STORY-012) to "a SEPARATE
  upcoming unit."

This ADR records that unit: `src/copilot.rs::extract_copilot_behavior` is now wired
and the events.jsonl event shape is LIVE-VERIFIED.

## Decision

The Behavior family is now wired for copilot, read from the per-session
`~/.copilot/session-state/<uuid>/events.jsonl` event stream — NOT the occupancy
process-log. **LIVE-VERIFIED** event shape:

- **Tool-call start:** `tool.execution_start` events carry `data.toolName` (the tool
  name) and `data.arguments` (the args object, structurally hashed for repetition
  detection — same role as opencode `data.state.input` and claude tool input).
- **Tool-call complete:** `tool.execution_complete` events carry `data.success`
  (bool) and `data.toolTelemetry.metrics.exit_code` (i64).
- **Error flag:** fires on `data.success == false` **OR**
  (`data.toolTelemetry.metrics.exit_code` present as an i64 **AND** != 0). This
  mirrors opencode's `status=='error' OR metadata.exit!=0` rule per **ADR-031** — the
  failure-streak keys on operation/exit-code failure, not on an invocation-status
  discriminator alone, so a command that fails while the tool call itself "succeeds"
  still raises the streak.
- **Structured-i64-only read:** the exit code is taken from the structured
  `toolTelemetry.metrics.exit_code` integer. No telemetry/output text is inspected —
  CODERULES r11 (no content inspection) and the deterministic /
  transcript-structure-only invariants hold.
- **Fail-closed:** missing or unparseable `events.jsonl`, events, `success`, or
  `exit_code` yield no error flag (and `None` rather than panic) — the REQ-015
  invariant, consistent with the rest of `copilot.rs`.

The repetition / ping-pong no-progress qualifier and the streak counting are inherited
from the shared `BehaviorSignals::from_signals` verdict logic, identical to the other
providers.

### Supersedes ADR-028's copilot blind claim (supersede-in-prose)

Per the never-edit-an-Accepted-ADR rule, ADR-028 is left unchanged and superseded here
in prose. ADR-028's consequence that "copilot remains structurally behavior-blind … a
permanent FORMAT limitation … can never fire" is **false and superseded**: copilot is
behavior-WIRED. The blindness conclusion was drawn from the occupancy process-log
alone; the events.jsonl stream — which ADR-028 did not consider — carries the tool
structure and exit-code signal the Behavior family needs.

### Resolves ADR-031's copilot PENDING note (supersede-in-prose)

ADR-031's "copilot is PENDING WIRING, not behavior-blind" forward note is now
**RESOLVED**: the wiring it deferred is complete and live-verified per the shape above.
ADR-031 is otherwise unchanged.

## Consequences

- **All four providers now contribute a real Behavior vote** in the family vote count
  (ADR-025). copilot joins claude/codex/opencode; the `behavior:None` copilot stub is
  gone.
- A real copilot agent stuck on a repeatedly-failing command now raises the
  failure-streak (the true stuck symptom), closing the gap ADR-028 wrongly described as
  permanent.
- The error-flag rule is uniform with opencode (success/status OR structured exit
  code), so cross-provider failure-streak behavior is consistent. claude keys on
  `is_error`; codex's failure-streak remains the documented dark spot (ADR-031) — no
  structural exit field exists in real codex `function_call_output` rows.
- Args-as-object (copilot/opencode/claude) vs args-as-string (codex) divergence stays
  isolated to the per-provider extractor; the shared `from_signals` logic is uniform.
- The read-only / advisory boundary (ADR-010) and the no-content-inspection invariant
  are untouched — only structured tool fields are read.

## Alternatives Considered

- **Leave copilot behavior-blind per ADR-028.** Rejected — the format claim was
  empirically false; events.jsonl carries the tool structure and exit code, so the
  Behavior family CAN and now does fire for copilot.
- **Read occupancy process-log for tool structure.** Rejected — the process-log truly
  carries no per-tool rows; the events.jsonl stream is the correct source.
- **Parse the copilot telemetry/output text for an exit code.** Rejected — violates
  CODERULES r11 (no content inspection); the structured
  `toolTelemetry.metrics.exit_code` i64 is read instead.
- **Key the error flag on `success` alone.** Rejected — a command can fail while the
  tool invocation reports `success == true`; per ADR-031 the streak must key on
  operation/exit-code failure, hence the `success==false OR exit_code!=0` rule.
