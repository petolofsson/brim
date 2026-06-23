---
id: REQ-004
title: Verdict thresholds
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-002
  - ADR-010
related_stories:
  - STORY-001
related_tests:
  - TEST-003
---

# REQ-004 - Verdict thresholds

## Requirement

* The system shall key the verdict off an absolute, model-agnostic effective-budget of active tokens (per ADR-010), not a percentage of the advertised window: ok (below the watch band), nearing -> watch (in measured-degradation territory), and over -> recycle (at or past the recycle backstop).
* The verdict shall combine the absolute watch/backstop bands with the ADR-006 projection (re-targeted to the recycle backstop) and the ADR-008 cache-thrash signal; it shall NOT consult an advertised-window fill percentage or capacity-runway readout (superseded by ADR-011 — the advertised-% nearing/ceiling thresholds and the capacity-runway readout are removed).
* The system shall make the effective-budget bands configurable (e.g. --watch-tokens / --recycle-backstop), with documented defaults.
* The system shall surface the verdict per node so an orchestrator can act on individual sub-agents.
* The verdict shall be advisory only; the system shall never modify a session.
