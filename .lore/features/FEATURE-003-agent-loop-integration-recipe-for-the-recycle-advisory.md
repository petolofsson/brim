---
id: FEATURE-003
title: Agent-loop integration recipe for the recycle advisory
status: Draft
related_requirements:
  - REQ-010
  - REQ-015
related_adrs:
  - ADR-010
  - ADR-011
related_stories:
  - STORY-011
related_tests: []
---

# FEATURE-003 - Agent-loop integration recipe for the recycle advisory

## Feature

A host-specific example integration that delivers brim's recycle advisory to an AI coding agent's loop WITHOUT brim's JSON ever entering the conversation context. The core premise: the host runs brim and parses its JSON entirely out-of-conversation — the agent never runs brim itself, and the JSON never enters the loop. brim already reasons in absolute tokens and advises recycle BEFORE the host's hard auto-compaction net (ADR-011), but that advice is useless if reading it costs the very context it is trying to protect. This recipe ships the thin, out-of-conversation wrapper that closes that gap.

This example targets Claude Code's Stop-hook + statusline JSON contract specifically. It is an example integration, not brim behavior: brim itself stays provider-neutral and unchanged (read-only, advisory CLI).

The recipe drives three consumer surfaces from the host (never the agent):

- **Statusline (ambient)** — a statusline command runs `brim --session <session_id> --json`, parses it in-shell, and renders the parent session's occupancy/verdict. Always-on, zero context cost.
- **Desktop notification** — a host Stop hook fires an OS notification on escalation into Over.
- **One-line agent nudge** — the same Stop hook returns a single advisory line via `additionalContext`, the only path by which the advisory touches the loop, and only on transition into Over.

## Recipe behavior

- **Out of conversation** — brim is invoked only by the host (Stop hook + statusline command); its JSON is parsed in-shell and never enters the conversation. The agent never runs brim itself.
- **Over** — emit a one-line agent nudge (Stop hook `additionalContext`) plus a desktop notification, debounced per session to fire only on transition into Over (sticky-Over, ADR-022).
- **Nearing** — surfaces on the statusline only; no context injection.
- **Verdict scope** — the parent session's OWN window via `brim --session <session_id>`, not the subtree `worst_verdict`.
- **Thresholds** — brim's defaults `--watch-tokens 32000` / `--recycle-backstop 128000`, no override.

## Included Artifacts

- REQ-010 SILENT-ON-OK — Ok emits nothing beyond the ambient statusline value.
- REQ-015 FAIL-CLOSED — any brim error / parse failure / missing session emits nothing and never blocks or delays the turn.
- STORY-011 — operator intent.

## Scope

- Example scripts (statusline command, Stop hook) and documentation shipped IN the brim repo as a host-specific integration example.
- Consumes brim's existing `--session` / `--json` contract (REQ-005) and brim's default thresholds.

## Out of Scope (non-goals)

- brim does NOT change its own behavior — it stays read-only and advisory.
- brim does NOT auto-recycle, auto-`/clear`, or modify any session.
- NO new brim CLI flags — the recipe uses the existing surface only.
- brim does NOT own or track the host hook schema; this example may need updating if the host changes its contract.
- The concrete shell scripts and `docs/` markdown are authored later (coder/documenter); this feature is the planning boundary, not the implementation.

## Realization

Implemented in `examples/claude-code/`:

- `brim-stop-hook.sh` — Stop hook: one-line agent nudge (`additionalContext`) + desktop notification on transition into Over (REQ-010, REQ-015).
- `brim-statusline.sh` — statusline command: ambient occupancy% + verdict.
- `settings.snippet.json` — paste-safe Claude Code `settings.json` wiring.
- `README.md` — the recipe (install, three surfaces, cadence, caveats).
