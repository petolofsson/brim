# brim × Claude Code — recycle advisory recipe

Out-of-conversation brim advisory for a Claude Code session: a **Stop hook**
(per turn) and a **statusline command** (per render) run `brim`, parse its JSON
**in-shell**, and surface the verdict. brim's JSON never enters the conversation
context, and the agent never runs brim itself. (FEATURE-003.)

## What it does — three surfaces

| Surface | When | Cost to context |
|---|---|---|
| **Statusline** — ambient occupancy% + verdict (`ok` / `nearing` / `over`) | every render | zero |
| **Desktop notification** (`notify-send`) | transition into `over` only, debounced per session | zero |
| **One-line agent nudge** (Stop hook `additionalContext`) | transition into `over` only, debounced per session | one line, only on `over` |

The nudge is the **only** path by which the advisory touches the loop, and only
on the first turn that crosses into `over` (sticky debounce, per session).

## Install

**Prerequisites**

- `brim` on `PATH` (`cargo install brim`)
- `jq`
- optional: `notify-send` (desktop notifications), `timeout` (hard cap on a hung brim)

**Wire it into `~/.claude/settings.json`** (or project `.claude/settings.json`):

```json
{
  "statusLine": {
    "type": "command",
    "command": "$CLAUDE_PROJECT_DIR/examples/claude-code/brim-statusline.sh"
  },
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "$CLAUDE_PROJECT_DIR/examples/claude-code/brim-stop-hook.sh",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

Critical wiring notes:

- **`Stop` is an array — APPEND, don't replace.** If you already have a Stop
  hook (e.g. a dot-agent-deck orchestration hook), add brim's entry to the
  existing array. Clobbering it breaks the other hooks.
- **`statusLine` is a single command.** If you already have a statusLine, you
  must merge brim into your existing command, not blindly overwrite it.
- **Adapt the paths.** `$CLAUDE_PROJECT_DIR` is the project root; point the
  `command` paths at wherever these scripts live. Bare script names work if they
  are on `PATH`.

## Cadence

- **Stop hook** — once per turn, on completion.
- **Statusline** — every render (~300ms host throttle).

## Config & caveats

- **Thresholds are brim's defaults**: `--watch-tokens 32000` /
  `--recycle-backstop 128000`. `nearing` fires at 32k, so on a large host window
  the statusline reads `nearing` for most of a session — **`over` (128k) is the
  actionable state.** Raise `--watch-tokens` in the scripts if you want a
  quieter statusline.
- **`BRIM_NO_NOTIFY=1`** suppresses desktop notifications. Useful for testing:
  running the Stop hook fires a real `notify-send`.
- **Host-schema drift.** This targets Claude Code's *current* hook/statusline
  JSON contract. brim does not own that schema, so the example may need updating
  if the host changes it.
- **Statusline latency** is ~57ms locally and scales with your **total** project
  count (brim scans all projects), not just this session.

## Fail-closed (REQ-015)

Any brim error, `jq` failure, or missing/invalid session → **silent, exit 0,
never blocks or delays the turn.** The statusline degrades to a neutral
`brim --`; the Stop hook emits nothing. On `ok`, nothing surfaces beyond the
ambient statusline value (REQ-010).
