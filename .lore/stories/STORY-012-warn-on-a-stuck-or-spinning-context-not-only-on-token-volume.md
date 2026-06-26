---
id: STORY-012
title: Warn on a stuck or spinning context, not only on token volume
status: Accepted
related_requirements:
  - FEATURE-001
  - REQ-016
related_adrs:
  - ADR-010
  - ADR-024
  - ADR-025
  - ADR-028
related_stories: []
related_tests:
  - TEST-010
---

# STORY-012 - Warn on a stuck or spinning context, not only on token volume

## User Story

As an agent operator,
I want brim to warn me when a context is STUCK or SPINNING — looping on the
same tool call, hitting a failure streak, ping-ponging between tools with no
progress, or growing unproductively — not only when raw token volume is high,
And I want the warning STAGES sharpened by draining transcript signals brim
already parses but discards today,
So that I recycle on the true onset of degradation (a session that has stopped
making progress) rather than waiting for an absolute-token backstop that a
healthy long session also trips — and so the top warning stages stop being
blind above that backstop.

## Context

- Today's engine verdict is keyed entirely off absolute tokens (ADR-010
  OR-gate, ADR-011 narrowed by ADR-020). Signal (a) "behavioral degradation"
  — ADR-010 §3's named "true onset detector" — is DEFERRED there as "needs
  eval probing." Research shows it does not: loop / repetition / failure-streak
  signals are derivable from `tool_use` / `tool_result` blocks already present
  in every provider transcript, so signal (a) is achievable WITHOUT breaking
  brim's deterministic / transcript-tokens-only invariant.
- The recycle-advisory recipe renders 5 presentation stages
  (lean / drift / bloated / stale / critical) as `max(occ_stage, verdict_stage)`.
  Stages 4–5 (stale / critical) are PURE OCCUPANCY: the engine enum saturates
  at `over_recycle`, so velocity and cache-thrash stop distinguishing the top
  tiers — only raw token count separates stale from critical. A spinning agent
  below the backstop reads healthy; a healthy long session above it reads
  critical. This is the blindness the story targets.
- "Sharpen by draining signals already parsed but discarded": brim already
  detects the compaction-reset index but throws away its drop magnitude, and
  already keeps timestamps and per-turn usage; these Tier-A signals can split
  stale-vs-critical by rate and learn each provider's real ceiling.

## Acceptance Criteria

- [ ] A context that is spinning (repeated identical tool call, consecutive
      same-tool failures, or A→B→A ping-pong with no output change) surfaces a
      recycle warning even when absolute tokens are below the backstop.
- [ ] The top warning stages (stale vs critical) are separated by a signal
      other than raw occupancy (e.g. velocity / behavioral state), closing the
      stage-4/5 pure-occupancy blindness.
- [ ] Every new warning input is derived from transcript `usage` /
      `tool_use` / `tool_result` blocks only — no eval probing, no content
      inspection — preserving the deterministic / transcript-tokens-only
      invariant.
- [ ] The advisory remains advisory-only and read-only (ADR-010 §5): brim
      recommends, never recycles or mutates a session.

<!-- Realizes ADR-010 signal (a) [[ADR-010]]; extends the operator intent of
STORY-001 (orchestrator self-diagnosis of context creep) and STORY-011 (warn
before the host hard-compacts). lore does not support story<->story links, so
those relations are recorded here in prose rather than front-matter. Candidate
signal catalog: REQ "Candidate verdict-signal variables". Decision direction:
ADR "Behavioral degradation gate from tool-call structure". -->
