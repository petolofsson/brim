#!/usr/bin/env bash
# brim Stop hook for Claude Code.
# Wired via settings.json hooks[].Stop.command.
# Prerequisite: `brim` must be on PATH (cargo install brim).
#
# REQ-015: never set -e; the hook MUST exit 0 on every path.
# REQ-010: emit nothing on ok/nearing; only fire on transition into Over.

# --- read session_id from stdin JSON ---
stdin_json=$(cat)
session_id=$(printf '%s' "$stdin_json" | jq -r '.session_id // empty' 2>/dev/null)
if [ -z "$session_id" ]; then
    exit 0  # REQ-015: parse failure → silent, exit 0
fi
# Reject session_id with chars outside [A-Za-z0-9-] to prevent path traversal on state file
case "$session_id" in
    *[!A-Za-z0-9-]*) exit 0 ;;
esac

# --- query brim for own-session verdict ---
# REQ-015: timeout guard — prevents a hung brim from stalling the turn
if command -v timeout >/dev/null 2>&1; then
    brim_out=$(timeout 5 brim --session "$session_id" --json 2>/dev/null)
else
    brim_out=$(brim --session "$session_id" --json 2>/dev/null)
fi
v=$(printf '%s' "$brim_out" | jq -r '.nodes[0].verdict // empty' 2>/dev/null)
if [ -z "$v" ]; then
    exit 0  # REQ-015: brim error / no session / parse failure → silent, exit 0
fi

# --- per-session debounce: only fire on transition INTO over ---
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/brim"
mkdir -p "$state_dir" 2>/dev/null || true
state_file="$state_dir/last-$session_id"
prior=""
if [ -f "$state_file" ]; then
    prior=$(cat "$state_file" 2>/dev/null)
fi
# If state_dir is unwritable, this silently fails; debounce degrades to 'may repeat'
printf '%s' "$v" > "$state_file" 2>/dev/null || true

# REQ-010 + REQ-015: anything other than a fresh transition into 'over' → silent
# Literal "over_recycle" comes from src/verdict.rs Verdict::Over as_json_str — keep in sync.
if [ "$v" != "over_recycle" ] || [ "$prior" = "over_recycle" ]; then
    exit 0
fi

# --- transition INTO over: compute occupancy for the message ---
tokens=$(printf '%s' "$brim_out" | jq '.nodes[0].window_tokens // 0' 2>/dev/null)
occupancy=$(printf '%s' "$tokens" | awk '{printf "%d", ($1 * 100 / 128000)}')

# --- emit one-line agent nudge via additionalContext (jq -n is injection-safe) ---
jq -n \
    --arg msg "brim: context window ${occupancy}% full (over budget) — wrap up this thread and start a new session to avoid hard compaction." \
    '{hookSpecificOutput:{hookEventName:"Stop",additionalContext:$msg}}'

# --- optional desktop notification (guarded; absence of notify-send is harmless) ---
# Set BRIM_NO_NOTIFY=1 to suppress notifications (e.g. dry-runs or automated tests)
if [ -z "${BRIM_NO_NOTIFY:-}" ] && command -v notify-send >/dev/null 2>&1; then
    notify-send 'brim' "context over budget (${occupancy}%) — wrap up and recycle" 2>/dev/null || true
fi

exit 0
