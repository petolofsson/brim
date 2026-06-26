---
id: TEST-009
title: "Ok-verdict node stays silent (verdict=ok, tier=lean, recycle_recommendation omitted)"
status: Accepted
related_requirements:
  - REQ-010
related_adrs: []
related_stories: []
related_tests: []
---

# TEST-009 - Ok-verdict node stays silent

## Test Case

Render an Ok-verdict node to `--json`. Implementing test: `src/output.rs::test_ok_verdict_silent_contract`.

## Expected Result

The node emits `verdict=ok` and `tier=lean`, and OMITS `recycle_recommendation` from the `--json` output, so the engine / Stop-hook stays silent (no warning surfaced).
