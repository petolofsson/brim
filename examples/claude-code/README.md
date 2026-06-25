# brim × Claude Code — recycle advisory recipe

Out-of-conversation brim advisory for a Claude Code session: a **Stop hook**
(per turn) and a **statusline command** (per render) run `brim`, parse its JSON
**in-shell**, and surface the verdict. brim's JSON never enters the conversation
context, and the agent never runs brim itself. (FEATURE-003.)

## What it does — three surfaces

| Surface | When | Cost to context |
|---|---|---|
| **Statusline** — `◈ brim <5-block bar> <occ>% <label>`, a 5-stage severity bucket | every render | zero |
| **Desktop notification** (`notify-send`) | transition into `over` (stage ≥ 3) only, debounced per session | zero |
| **One-line agent nudge** (Stop hook `additionalContext`) | transition into `over` (stage ≥ 3) only, debounced per session | one line, only on `over` |

The nudge is the **only** path by which the advisory touches the loop, and only
on the first turn that crosses into `over` (sticky debounce, per session). It
fires a **single** alert on entry into `over` — there is **no** re-notify when
the session later escalates into `stale`/`critical` (deliberate).

## Statusline: the 5-stage bar

The statusline renders `◈ brim <bar> <occ>% <label>`, where the bar is 5 blocks
filled to the current stage (`■`) and padded (`□`). `occ` =
`floor(window_tokens × 100 / 128000)` — occupancy as a percentage of the 128k
backstop; it **can exceed 100%** once the window grows past the backstop.

The stage is `max(occupancy_stage, verdict_stage)` — **worst-of-both wins**. A
burst can trip the engine's `over_recycle` verdict (≥ stage 3) even at low
occupancy, and high occupancy wins even when the engine is calm:

| Stage | Bar | Label | Color | occupancy band | verdict |
|---|---|---|---|---|---|
| 1 | `■□□□□` | `lean` | green | `< 75%` | `ok` |
| 2 | `■■□□□` | `drift` | yellow | `75–99%` | `nearing` |
| 3 | `■■■□□` | `bloated` | orange | `100–199%` | `over_recycle` |
| 4 | `■■■■□` | `stale` | red | `200–399%` | — |
| 5 | `■■■■■` | `critical` | red-bold | `≥ 400%` | — |

Stages 4–5 are pure-occupancy: the engine verdict saturates at `over_recycle`
(stage 3), so only occupancy escalates above it. The 5 stages are a recipe-only
presentation layer over brim's 3-value engine verdict; promoting them into the
engine is tracked as roadmap item #4 in `docs/recycle-verdict-model.md`.

Stage 1 (`lean`, green) is the ambient `ok` render — the quiet steady state
(REQ-010). The nudge/notify text is stage-specific: `bloated` (~128k+),
`stale` (~256k+), `critical` (~512k+).

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

- **Thresholds**: the scripts pass `--watch-tokens 96000`; the
  `--recycle-backstop 128000` default is unchanged. brim's default watch is
  `32000`, but `nearing` at 32k cries wolf — real agentic sessions run far past
  it — so the recipe ends the green/`lean` band at 75% of the 128k backstop
  (96k). `occ%` is measured against the 128k backstop and **exceeds 100%** once
  the window grows past it. Lower `--watch-tokens` for an earlier `drift`
  warning, or raise it for a quieter statusline.
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
