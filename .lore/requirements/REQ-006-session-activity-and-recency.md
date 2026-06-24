---
id: REQ-006
title: Session activity and recency
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs: [ADR-016]
related_stories: [STORY-007]
related_tests: [TEST-006]
---

# REQ-006 - Session activity and recency

## Requirement

* The system shall record each node's last-turn timestamp (the timestamp of the latest `assistant` turn used for the window computation) and expose it in both text and JSON output.
* The system shall classify a session as active when its last-turn timestamp is within a configurable recency threshold, with a documented default.
* The default listing shall show active sessions only (satisfying FEATURE-001's "active sessions" wording); a `--all` flag shall include stale/historical sessions.
* The system shall derive recency only from real transcript timestamps; an absent or unparseable timestamp -> treat the session as inactive (shown only under `--all`), never panic.
* The activity signal is advisory; it shall never modify or delete any transcript.

## Rationale

brim lists ALL historical transcripts, so an orchestrator cannot distinguish live sub-agents of the current task from stale ones. Surfacing a real last-turn timestamp and filtering to active sessions by default lets the orchestrator focus on what is live, while `--all` preserves access to historical sessions.

## Acceptance Criteria

- [ ] Each node carries a last-turn timestamp in both text and JSON output.
- [ ] A session within the configurable recency threshold is classified active; the default threshold is documented.
- [ ] Default output lists only active sessions; `--all` includes stale/historical sessions.
- [ ] Absent or unparseable timestamps mark a session inactive (visible only under `--all`) and never cause a panic.
- [ ] The activity signal never modifies or deletes a transcript.
