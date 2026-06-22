---
id: TEST-003
title: Verdict thresholds and non-Claude degradation
status: Draft
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

Given sessions at varying fill percentages across Claude Code, Codex, and Copilot,

When brim assigns verdicts and renders output,

Then fill below the nearing threshold shall read "ok", fill in the nearing band shall read "nearing", fill at or over the ceiling shall read "over -> recycle", and Codex/Copilot sessions shall render as flat nodes with no parent and no error when sub-agent data is absent.
