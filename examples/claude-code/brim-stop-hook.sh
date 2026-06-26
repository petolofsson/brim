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

# REQ-010 + REQ-015: only fire on bloated/stale/critical (ADR-025 tier).
# Tier strings come from src/verdict.rs Tier::as_json_str — keep in sync.
case "$tier" in
    bloated|stale|critical) ;;
    *)
        # Reset stored stage so a later re-escalation notifies (MEDIUM fix)
        _sd="${XDG_STATE_HOME:-$HOME/.local/state}/brim"
        printf '0\n' > "$_sd/last-$session_id" 2>/dev/null || true
        exit 0 ;;
esac

# --- stage from tier (3=bloated, 4=stale, 5=critical) ---
case "$tier" in
    bloated)  stage=3 ;;
    stale)    stage=4 ;;
    critical) stage=5 ;;
esac

# --- per-session debounce: fire only on escalation (current stage > last-notified stage) ---
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/brim"
mkdir -p "$state_dir" 2>/dev/null || true
state_file="$state_dir/last-$session_id"
prior_stage=0
if [ -f "$state_file" ]; then
    read -r prior_stage < "$state_file" 2>/dev/null || prior_stage=0
    # Validate: must be a non-negative integer; anything else resets to 0
    case "$prior_stage" in
        ''|*[!0-9]*) prior_stage=0 ;;
    esac
fi
# If stage is same or lower than last-notified, suppress (already warned at this level or higher)
if [ "$stage" -le "$prior_stage" ] 2>/dev/null; then
    exit 0
fi
# Update stored stage; if unwritable debounce degrades to 'may repeat'
printf '%s\n' "$stage" > "$state_file" 2>/dev/null || true

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
