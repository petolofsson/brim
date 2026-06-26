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
    brim_out=$(timeout 5 brim --session="$session_id" --watch-tokens 96000 --json 2>/dev/null)
else
    brim_out=$(brim --session="$session_id" --watch-tokens 96000 --json 2>/dev/null)
fi
tier=$(printf '%s' "$brim_out" | jq -r '.nodes[0].tier // empty' 2>/dev/null)
if [ -z "$tier" ]; then
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
printf '%s' "$tier" > "$state_file" 2>/dev/null || true

# REQ-010 + REQ-015: only fire on fresh transition into bloated/stale/critical (ADR-025 tier).
# Tier strings come from src/verdict.rs Tier::as_json_str — keep in sync.
case "$tier" in
    bloated|stale|critical) ;;
    *) exit 0 ;;
esac
case "$prior" in
    bloated|stale|critical) exit 0 ;;  # already notified at this level or higher
esac

# --- stage from tier (3=bloated, 4=stale, 5=critical) ---
case "$tier" in
    bloated)  stage=3 ;;
    stale)    stage=4 ;;
    critical) stage=5 ;;
esac

case "$stage" in
    5) nudge_msg='brim: context critical (~512k+) — recycle now.'
       notify_msg='context critical (~512k+) — recycle now' ;;
    4) nudge_msg='brim: context stale (~256k+) — start a fresh session.'
       notify_msg='context stale (~256k+) — start a fresh session' ;;
    *) nudge_msg='brim: context bloated (~128k+) — recycle recommended.'
       notify_msg='context bloated (~128k+) — recycle recommended' ;;
esac

# --- emit one-line agent nudge via additionalContext (jq -n is injection-safe) ---
jq -n \
    --arg msg "$nudge_msg" \
    '{hookSpecificOutput:{hookEventName:"Stop",additionalContext:$msg}}'

# --- optional desktop notification (guarded; absence of notify-send is harmless) ---
# Set BRIM_NO_NOTIFY=1 to suppress notifications (e.g. dry-runs or automated tests)
if [ -z "${BRIM_NO_NOTIFY:-}" ] && command -v notify-send >/dev/null 2>&1; then
    notify-send 'brim' "$notify_msg" 2>/dev/null || true
fi

exit 0
