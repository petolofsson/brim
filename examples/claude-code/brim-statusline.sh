#!/usr/bin/env bash
# brim statusLine command for Claude Code.
# Wired via settings.json statusLine.command.
# Prerequisite: `brim` must be on PATH (cargo install brim).
#
# REQ-015: always exits 0 and degrades to a neutral value on any failure.
# REQ-010: always renders ambient occupancy/verdict (the only surface on 'ok').

# ANSI colour codes — use printf to get real ESC byte (0x1b), not literal backslash
GREEN=$(printf '\033[0;32m')
YELLOW=$(printf '\033[1;33m')
ORANGE=$(printf '\033[38;5;208m')
RED=$(printf '\033[0;31m')
RED_BOLD=$(printf '\033[1;31m')
RESET=$(printf '\033[0m')

# --- read session_id from stdin JSON ---
stdin_json=$(cat)
session_id=$(printf '%s' "$stdin_json" | jq -r '.session_id // empty' 2>/dev/null)
if [ -z "$session_id" ]; then
    printf '◈ brim --\n'  # REQ-015: neutral fallback
    exit 0
fi

# --- query brim ---
# REQ-015: timeout guard — prevents a hung brim from stalling the turn
if command -v timeout >/dev/null 2>&1; then
    brim_out=$(timeout 5 brim --session="$session_id" --watch-tokens 96000 --json 2>/dev/null)
else
    brim_out=$(brim --session="$session_id" --watch-tokens 96000 --json 2>/dev/null)
fi
v=$(printf '%s' "$brim_out" | jq -r '.nodes[0].verdict // empty' 2>/dev/null)
tokens=$(printf '%s' "$brim_out" | jq '.nodes[0].window_tokens // 0' 2>/dev/null)

if [ -z "$v" ]; then
    printf '◈ brim --\n'  # REQ-015: parse failure → neutral fallback
    exit 0
fi

# occupancy % = floor(tokens * 100 / 128000)
occupancy=$(printf '%s' "$tokens" | awk '{printf "%d", ($1 * 100 / 128000)}')

# --- determine stage (1..5) = max(occ_stage, verdict_stage) ---
if   [ "$occupancy" -ge 400 ]; then occ_stage=5
elif [ "$occupancy" -ge 200 ]; then occ_stage=4
elif [ "$occupancy" -ge 100 ]; then occ_stage=3
elif [ "$occupancy" -ge 75  ]; then occ_stage=2
else                                 occ_stage=1
fi
case "$v" in
    ok)           verdict_stage=1 ;;
    nearing)      verdict_stage=2 ;;
    over_recycle) verdict_stage=3 ;;
    *)            verdict_stage=1 ;;
esac
stage=$occ_stage
[ "$verdict_stage" -gt "$stage" ] && stage=$verdict_stage

# --- stage → label, color ---
case "$stage" in
    1) label='lean';     color="$GREEN"    ;;
    2) label='drift';    color="$YELLOW"   ;;
    3) label='bloated';  color="$ORANGE"   ;;
    4) label='stale';    color="$RED"      ;;
    5) label='critical'; color="$RED_BOLD" ;;
    *) label='?';        color="$RESET"    ;;
esac

# --- build 5-block BAR: STAGE filled (■) then padded (□) ---
bar=""
for i in 1 2 3 4 5; do
    if [ "$i" -le "$stage" ]; then bar="${bar}■"; else bar="${bar}□"; fi
done

# --- render (SC2059: ANSI vars passed as args, not in format string) ---
printf '◈ brim %s%s%s %s%% %s\n' "$color" "$bar" "$RESET" "$occupancy" "$label"

exit 0
