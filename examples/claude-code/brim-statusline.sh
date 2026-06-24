#!/usr/bin/env bash
# brim statusLine command for Claude Code.
# Wired via settings.json statusLine.command.
# Prerequisite: `brim` must be on PATH (cargo install brim).
#
# REQ-015: always exits 0 and degrades to a neutral value on any failure.
# REQ-010: always renders ambient occupancy/verdict (the only surface on 'ok').

# ANSI colour codes
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
RESET='\033[0m'

# --- read session_id from stdin JSON ---
stdin_json=$(cat)
session_id=$(printf '%s' "$stdin_json" | jq -r '.session_id // empty' 2>/dev/null)
if [ -z "$session_id" ]; then
    printf 'brim --\n'  # REQ-015: neutral fallback
    exit 0
fi

# --- query brim ---
# REQ-015: timeout guard — prevents a hung brim from stalling the turn
if command -v timeout >/dev/null 2>&1; then
    brim_out=$(timeout 5 brim --session "$session_id" --json 2>/dev/null)
else
    brim_out=$(brim --session "$session_id" --json 2>/dev/null)
fi
v=$(printf '%s' "$brim_out" | jq -r '.nodes[0].verdict // empty' 2>/dev/null)
tokens=$(printf '%s' "$brim_out" | jq '.nodes[0].window_tokens // 0' 2>/dev/null)

if [ -z "$v" ]; then
    printf 'brim --\n'  # REQ-015: parse failure → neutral fallback
    exit 0
fi

# occupancy % = floor(tokens * 100 / 128000)
occupancy=$(printf '%s' "$tokens" | awk '{printf "%d", ($1 * 100 / 128000)}')

# --- render with colour (SC2059: ANSI vars passed as args, not in format string) ---
case "$v" in
    ok)      printf '%sbrim %s%% ok%s\n'      "$GREEN"  "$occupancy" "$RESET" ;;
    nearing) printf '%sbrim %s%% nearing%s\n' "$YELLOW" "$occupancy" "$RESET" ;;
    over)    printf '%sbrim %s%% over%s\n'    "$RED"    "$occupancy" "$RESET" ;;
    *)       printf 'brim %s%% %s\n'          "$occupancy" "$v" ;;
esac

exit 0
