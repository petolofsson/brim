---
id: TEST-003
title: Verdict thresholds and non-Claude degradation
status: Accepted
related_requirements:
  - FEATURE-001
  - REQ-004
related_adrs:
  - ADR-002
related_stories:
  - STORY-001
related_tests: []
---

# TEST-003 - Verdict thresholds and non-Claude degradation

## Test Case

Given sessions at varying fill percentages across Claude Code and Codex, and Copilot sessions that expose only a cumulative-spend counter,

When brim assigns verdicts and renders output,

Then fill below the nearing threshold shall read "ok", fill in the nearing band shall read "nearing", and fill at or over the ceiling shall read "over -> recycle"; Codex and Copilot sessions shall render as flat nodes with no parent and no error when sub-agent data is absent; and Copilot sessions shall list (session id, project, recency) with no fill % and no verdict, since their only counter is cumulative (window unavailable per ADR-002).
