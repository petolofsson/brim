---
id: REQ-004
title: Verdict thresholds
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-002
related_stories:
  - STORY-001
related_tests:
  - TEST-003
---

# REQ-004 - Verdict thresholds

## Requirement

* The system shall map fill percentage to a verdict: ok (low), nearing (approaching the limit), and over -> recycle (at or over the safe ceiling).
* The system shall make thresholds configurable, with documented defaults.
* The system shall surface the verdict per node so an orchestrator can act on individual sub-agents.
* The verdict shall be advisory only; the system shall never modify a session.
