# brim — context-window diagnostic for AI coding sessions

`brim` reports how full each session's context window is — for an orchestrator
and its sub-agents — so you can recycle a session before it overbounds.

## Run

```sh
cargo run -- --tree
```

## Map

- behavior → CLAUDE.md  - code rules → CODERULES.md
- requirements / design / decisions → lore (`lore show FEATURE-001 --recursive`, `lore search <text>`)

<!-- router; edit on new entry point or moved target, not a changelog -->
