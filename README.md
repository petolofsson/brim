# brim — context-window diagnostic for AI coding sessions

`brim` reports how full each session's context window is — for an orchestrator
and its sub-agents — so you can recycle a session before it overbounds.

## Run

```
brim [--tree] [--session <id>] [--json] [--all] [--active-mins <N>]
     [--watch-tokens <N>] [--recycle-backstop <N>] [--once]
# default: active-only flat list
```

## Map

- behavior → CLAUDE.md  - code rules → CODERULES.md
- requirements / design / decisions → lore (`lore show FEATURE-001 --recursive`, `lore search <text>`)
- Claude Code integration recipe → examples/claude-code/

<!-- router; edit on new entry point or moved target, not a changelog -->
