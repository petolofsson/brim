---
id: STORY-001
title: Orchestrator self-diagnosis of context creep
status: Draft
related_requirements:
  - FEATURE-001
  - REQ-001
  - REQ-002
  - REQ-003
  - REQ-004
related_adrs: []
related_stories: []
related_tests:
  - TEST-001
  - TEST-002
  - TEST-003
---

# STORY-001 - Orchestrator self-diagnosis of context creep

As an orchestrator running multiple sub-agents, I want to see the context-window fill of myself and each sub-agent at a glance, so that I can recommend recycling a session into a fresh one before its context overbounds and quality degrades.
