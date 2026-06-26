---
id: ADR-030
title: Codex provider parsers live-verified (payload-item tool calls, token_count occupancy)
status: Accepted
related_requirements:
  - REQ-016
related_adrs:
  - ADR-024
  - ADR-028
related_stories:
  - STORY-012
related_tests: [TEST-011]
---

# ADR-030 - Codex provider parsers live-verified (payload-item tool calls, token_count occupancy)

## Context

ADR-024 realized signal (a) "behavioral degradation" as a deterministic gate
fed by tool-call STRUCTURE; ADR-028 did the live-verification + stub removal for
opencode. brim's **codex** parsers (`extract_codex_behavior`, occupancy via
`extract_window`, `extract_project_key`) were **SPEC-DERIVED**: written from the
documented codex format and never run against a real codex transcript. The
Behavior tests were marked "schema-confirmed; not live-data validated."

A real codex **0.142.2** session was captured and the parsers run against it.
This surfaced a structural divergence: the spec-derived code expected tool calls
inside a `content[]` array, but real codex rows carry them as **direct payload
items**. Before this fix the Behavior family **NEVER fired** for any real codex
session — the `content[]` path matched nothing, so codex was silently
occupancy-only despite reading as "wired."

## Decision

Extract the Behavior family and occupancy for codex from the LIVE-VERIFIED
codex 0.142.2 row shapes. Exact shapes:

- **Discovery:** `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
- **Row wrapper:** each row wraps a `payload`. Tool calls are **DIRECT payload
  items** — `payload.type` ∈ {`function_call`, `function_call_output`,
  `custom_tool_call`} — NOT entries in a `content[]` array (that was the bug).
  The legacy `content[]` path is KEPT as a spec-derived fallback only.
  - `function_call`: tool name `payload.name`; args `payload.arguments` (a JSON
    **string**, hashed on the **raw string** for repetition detection).
  - `custom_tool_call` (e.g. `apply_patch`): name `payload.name`; args field is
    `payload.input` (not `arguments`), hashed raw.
  - `function_call_output`: error discriminator `payload.status == "failed"`
    feeds `failure_streak`.
- **Occupancy:** `payload.type == "token_count"` →
  `payload.info.total_token_usage.{input_tokens, cached_input_tokens}`.
  `window_tokens = input_tokens` (already INCLUDES cached); `cache_creation = 0`.
  Occupancy is **COMPLETE** (not Thrash-partial) — `cache_hit_ratio` stays
  well-defined. `payload.type` is the filter, so `event_msg` vs `response_item`
  at the top level is irrelevant.
- **project_key:** `payload.cwd`, with top-level `v["cwd"]` as legacy fallback
  (also tries `project_path`, `directory`).

## Consequences

- The Behavior family now actually fires for real codex sessions (verified:
  repeated `function_call` and `custom_tool_call` produce `repetition_run`).
  codex args are a STRING (hashed raw), unlike opencode/Claude objects (ADR-028).
- **Error path is SYNTHETIC-ONLY.** The `failed`-status → `failure_streak`
  mapping is validated synthetically; the verify session contained only
  `status == "completed"` outputs. Real-failure codex data is **PENDING** — the
  discriminator is unconfirmed against a real failed turn.
- **Pre-existing limitation (not introduced here):** `extract_project_key`
  scans a bounded line/tail window (`codex.rs` ~207, `.take(20)`). A very large
  session (>256KB) whose `session_meta` precedes the scanned tail window may
  yield an empty `project_key`. Out of scope for this live-verification.

## Alternatives Considered

- **Keep only the `content[]` path and treat codex as occupancy-only.**
  Rejected: the structure is present and live-verified; dropping it would leave
  codex permanently Behavior-blind for a divergence that is a one-row fix.
- **Drop the legacy `content[]` fallback.** Rejected: kept defensively in case a
  codex version emits the documented `content[]` shape; it costs nothing when
  the payload-item path matches first.
