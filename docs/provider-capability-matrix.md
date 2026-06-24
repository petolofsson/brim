# Provider Capability Matrix — brim

Verified matrix for STORY-003; reconciles ADR-002, ADR-005, ADR-006 against official docs and real captured sessions. No inference-only cells: each is sourced to docs, a real session on this machine, or marked **Pending** when neither exists yet.

## Matrix

| Provider | Transcript location / format | Per-turn occupancy source | Cache split | Sub-agent linkage | Data-quality wrinkles | Model context-window limit | Verification status |
|---|---|---|---|---|---|---|---|
| **Claude Code** | `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`; JSONL one event per line; sub-agents at `<uuid>/subagents/agent-*.jsonl`. **No real session on this machine.** | Last `type:"assistant"` turn's `message.usage.{input_tokens, cache_read_input_tokens, cache_creation_input_tokens}`; point-in-time `window_tokens = input + cache_read + cache_create` (ADR-002). | Read (`cache_read_input_tokens`) and creation (`cache_creation_input_tokens`) both present. | Parent session UUID dir contains `subagents/agent-<agent-id>.jsonl`; brim joins by parent UUID → `agent_id` stem (code-confirmed). | Skipping zero-usage trailing turns; tail-read truncation risk on >256KB transcripts. | Docs-listed Claude models (Opus/Sonnet 200k base, `[1m]` variants 1M) via models.dev brim default 200k. Real model = **Pending**. | Docs — Pending real-session capture. |
| **Codex** | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`; one event per line; **flat** (single rollout per file). Real sessions present (2026/04–06). | `payload.type=="token_count"` events carry `payload.info.total_token_usage` (cumulative); brim takes the delta of the last two deduped events = last-turn occupancy — `input_tokens` (includes cached) + `cached_input_tokens`. | `cached_input_tokens` present (read-side only). **No** cache-creation field (verified real session). | **None in real JSONL.** `parent_thread_id` / `forked_from_id` / `agent_role` are ABSENT. Only `payload.source.subagent` (free-text role, e.g. `"review"`) appears, on session_meta. brim correctly renders flat (matches REQ-002 scope cut, but the REQ-002 rationale "linkable via SessionMeta…" is itself unsupported). | Duplicate consecutive `token_count` events with identical totals (rate-limit #14489) — brim dedupes. **Confirmed in real session** (dup observed). `% (used/limit)` style not present. | Real model `gpt-5.4` / `gpt-5.5` / `gpt-5.4-mini` (openai). Advertised limit per models.dev: gpt-5.4 = 1,050,000; gpt-5.5 = 1,050,000. brim clamps to ABSOLUTE backstop. | Real-session (Docs pending per-cell below). |
| **OpenCode** | `~/.local/share/opencode/opencode.db` (SQLite, WAL). `session` + `part` + `project` tables. Real DB present (~33 MB, 30 sessions, 674 step-finish rows). | Latest `part` row with `json_extract(data,'$.type')='step-finish'` ordered `time_created DESC`; `data.tokens.{total?, input, output, cache.{read, write}}`. Prefer `tokens.total`; else sum components. | `cache.read` and `cache.write` both present (`data.tokens.cache`). | `session.parent_id` is the join key (schema confirmed; docs confirm subagents create child sessions). **0 rows with `parent_id` non-null in real DB** — structural, unpopulated. brim joins on it. | No `type` column on `part` (uses `json_extract`); no pre-checkpoint empty-data rows in real DB; `time` inside `data` overrides row `time_created` for timestep. | **200k assumption is WRONG.** models.dev (opencode AI-SDK registry) lists `z-ai/glm-5.2` context = **1,000,000**. brim's 200k default underestimates real window by 5×. | Real-session + Docs. |
| **Copilot** | `~/.copilot/session-state/<uuid>/` + `~/.copilot/logs/process-<epochMs>-<pid>.log`. Per-turn occupancy from `CompactionProcessor: Utilization <PCT> (<USED>/<LIMIT> tokens)` log lines; `events.jsonl` carries NO occupancy. | `CompactionProcessor` log lines; `<USED>` parsed (PCT/LIMIT ignored, ADR-011). | None (no cache fields in process log). | None (flat). | ephemeral usage in process log; session↔log linkage via `inuse.<pid>.lock`. | Per Copilot advertised limit in log `<LIMIT>` token — not stored by brim. | **VERIFIED-LIVE** (per REQ-002/REQ-009; not re-verified here). |

## Claude Code

- **Docs consulted:** `https://docs.claude.com/en/docs/claude-code/sub-agents` (sub-agents / `subagents/` directory). Transcript doc URL `…/transcript` returned 404 at fetch time — layout inferred from `src/claude.rs` path constants and the sub-agents doc.
- **Real session:** `~/.claude/projects/` does NOT exist on this machine — no real Claude Code session. Backing the matrix's real-session half is **Pending capture** (STORY-003 stated dependency).
- **Discrepancy:** docs / brim code agree on the `subagents/agent-*.jsonl` layout; the last-turn `message.usage` field shape (`input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`) is only code-confirmed here, not real-session-confirmed.
- **Redacted sample line:** *no real session available — pending capture.*

## Codex

- **Docs consulted:** `https://github.com/openai/codex` (CLI session rollout format; issue #14489你已经 dedupe for). Codex CLI session format is defined in-repo; no field-level transcript page was located beyond the rollout file layout.
- **Real session:** `~/.codex/sessions/2026/{04,05,06}/*.jsonl` opened directly. Sample (redacted) `session_meta`:
  ```json
  {"timestamp":"2026-04-02T11:46:30.193Z","type":"session_meta","payload":{"id":"019d4e04-…","timestamp":"2026-04-02T11:46:28.615Z","cwd":"<redacted-cwd>","originator":"codex-tui","cli_version":"0.118.0","source":{"subagent":"review"},"model_provider":"openai","git":"<redacted>"}}
  ```
  Sample (redacted) populated `token_count` event:
  ```json
  {"timestamp":"2026-06-01T10:02:43.374Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10176,"cached_input_tokens":8576,"output_tokens":260,"reasoning_output_tokens":65,"total_tokens":10436},"last_token_usage":{"input_tokens":10176,…}},"rate_limits":{"credits":{"balance":"<redacted>"}}}}
  ```
- **`SessionMeta` linkage fields — explicit finding:** `parent_thread_id`, `forked_from_id`, `agent_role` DO NOT appear in any real rollout file under `~/.codex/sessions/` (grepped all 2026 files; zero matches). The only subagent-related field present is `payload.source.subagent` (a free-text role string carried on `session_meta`). brim's flat rendering is therefore correct, but **the REQ-002 rationale ("linkable via `SessionMeta` `parent_thread_id`/`forked_from_id`/`agent_role`") is unsupported by real sessions** and should be revised when lore is realigned. brim's flat rendering is a data-availability reality, not merely a scope cut.
- **Dedupe wrinkle confirmed:** a real duplicate `token_count` event with identical `total_token_usage` was observed (timestamps `2026-06-01T11:29:19.177Z` and `11:29:43.518Z`) — brim's codex #14489 dedupe is necessary, not defensive-only.
- **Model context limit:** real-session models `gpt-5.4` (394 events), `gpt-5.5` (35), `gpt-5.4-mini` (32) — all `openai` provider. Per models.dev advertised context: gpt-5.4 = 1,050,000; gpt-5.5 = 1,050,000. No Codex-specific docs page found that asserts a hard CLI window limit; real-session token_count events expose cumulative usage, not a limit field.

## OpenCode

- **Docs consulted:** `https://opencode.ai/docs/agents/` (subagents / `parent_id` child sessions via `session_child_first`); `https://opencode.ai/docs/models/` (model selection, no model→context registry in user config); `https://opencode.ai/docs/providers/` (credentials stored in `~/.local/share/opencode/auth.json` — NOT consulted, contains secrets).
- **Model context limit — explicit verdict on ADR-005:** **corrected-from-200_000 to 1,000,000.** `https://models.dev/?q=glm` (the AI-SDK model registry opencode imports) lists `zhipuai/glm-5.2` (the only model in the real DB: `{"id":"z-ai/glm-5.2","providerID":"llmbase"}`) with Context = **1,000,000**, Output = 131,072. ADR-005's stated 200k default is wrong by 5×; a new ADR should supersede it. ADR-005's secondary claim — "limit not stored in opencode db or config" — is **confirmed**: `.schema session|part|project` has no `limit`/`context` column, and `/docs/models` documents no limit field.
- **Real DB evidence:** 30 sessions, 674 `step-finish` parts, 0 sessions with `parent_id` non-null (sub-agent structural, unpopulated — matches ADR-005). Counts are point-in-time against the live WAL; expect drift between captures. Redacted step-finish row (latest):
  ```json
  {"reason":"tool-calls","snapshot":"cdeef…","type":"step-finish","tokens":{"total":69520,"input":15917,"output":99,"reasoning":0,"cache":{"write":0,"read":53504}},"cost":0}
  ```
  Redacted session rows (truncated ids):
  ```
  ses_1066 | parent_id=NULL | model={"id":"z-ai/glm-5.2","providerID":"llmba…"} | directory=<redacted-cwd>
  ```
- **Schema wrinkles:** the real `part` table has NO `type` column (only `data` JSON with `$.type`), consistent with brim's `json_extract(data,'$.type')='step-finish'` query. No pre-checkpoint empty-`data` step-finish rows were found (0 of 674). ADR-005's pre-checkpoint fallback path is therefore untested against a real session here, but the code path stands.
- **Discrepancy:** opencode docs do not document the step-finish `tokens` schema explicitly; the schema is real-session-confirmed here. docs ↔ real session agreement on `parent_id` linkage; disagreement only with ADR-005's 200k limit assumption.

## Copilot

- **Docs consulted:** not re-verified (per task scope). The VERIFIED-LIVE status of REQ-002/REQ-009 stands: per-turn occupancy from `CompactionProcessor: Utilization <USED>/<LIMIT> tokens` lines in `~/.copilot/logs/process-<epochMs>-<pid>.log`; session↔log linkage via `inuse.<pid>.lock`; `events.jsonl` carries NO occupancy. brim reports absolute `<USED>` only (ADR-011).
- **Redacted sample line:** per the existing VERIFIED-LIVE record, a representative log line is
  ```
  2026-01-01T10:00:00.000Z [INFO] CompactionProcessor: Utilization 10% (10000/200000 tokens) below threshold 90%
  ```
  (`/Users/pol/code/brim/src/copilot.rs:307`, test fixture — matches the real-session shape captured in REQ-009).
- **Discrepancy:** none reported by the prior VERIFIED-LIVE pass; not re-verified here. Copilot is the reference row.

## ADR reconciliation

- **ADR-002 (point-in-time window over cumulative aggregate) — confirmed.** Both OpenCode (step-finish `tokens`) and Codex (delta of cumulative `total_token_usage`) realize a point-in-time oracle; brim's design fits observed data. The Claude parity half remains Pending real-session capture but is code-consistent. Copilot is the lone exception (process-log oracle, already VERIFIED-LIVE).
- **ADR-005 (opencode point-in-time window from step-finish with aggregate fallback) — confirmed in principle, CORRECTED on the 200k limit.** The step-finish oracle, `parent_id` join, and aggregate fallback are all confirmed against the real DB. The specific claim "treat `z-ai/glm-5.2` as 200_000" must be corrected to **1,000,000** (models.dev). Because ADR-005 is still **Draft** and its own text says "this ADR stays Draft until confirmed in production", the limit correction should ship as a new superseding ADR (per CLAUDE.md / lore rules) rather than an in-place edit.
- **ADR-006 (velocity and overbound projection from bounded last-K tail) — confirmed.** Per-turn history is present for Claude (per assistant turn), Codex (per cumulative `token_count` delta), and OpenCode (per step-finish); Copilot's process-log oracle yields trend too (VERIFIED-LIVE). The bounded-tail design is supportable on all four providers; the median + reset-detection robustness claims hold against real codex duplicate-event data (#14489 dedupe feeds clean deltas).

## Oracle + trend parity

- **Claude Code:** per-turn oracle = last `assistant` turn `message.usage`; ADR-006 trend supportable from the last K assistant turns. Real-session parity pending.
- **Codex:** per-turn oracle = delta of the last two deduped `token_count` cumulative events (= last-turn usage); ADR-006 trend supportable from the last K+1 deduped cumulative-event deltas. Confirmed on real session.
- **OpenCode:** per-turn oracle = latest `step-finish` part `data.tokens`; ADR-006 trend supportable from the last K `step-finish` parts. Confirmed on real DB. Aggregate fallback (`session.tokens_*`) is cumulative-only and yields `window_source=Aggregate` (ADR-002-permitted approximation) — no per-turn oracle for pre-checkpoint sessions.
- **Copilot:** per-turn oracle = `CompactionProcessor: Utilization` log line; ADR-006 trend supportable from the last K utilization lines. VERIFIED-LIVE.

## Pending / blockers

- **Claude Code real-session capture** — no `~/.claude/projects/` on this machine; the entire Claude row's real-session half (transcript layout, last-turn-field shape, model id, and limit) is **Pending**. Docs half is grounded. Do NOT fabricate.
- **Codex per-model CLI window limit** — real Codex JSONL exposes cumulative usage but no explicit `context_window` / `limit` field; the per-model advertised limits here come from models.dev, not from a Codex-CLI docs page. Cell is docs-sourced (models.dev), not Codex-CLI-docs-sourced. Flag if strict Codex-docs sourcing is required.
- **OpenCode pre-checkpoint aggregate-fallback path** — 0 empty-`data` step-finish rows in the real DB, so the `WindowSource::Aggregate` path is real-DB-untested here; code path stands under ADR-002's permitted approximation.
- **ADR-005 200k assumption** — corrected to 1,000,000 per models.dev. A superseding ADR is the realignment step (out of scope here per task — lore not edited in this delegation).
- **REQ-002 Codex `SessionMeta` linkage rationale** — the requirement states Codex sub-agents are linkable via `parent_thread_id` / `forked_from_id` / `agent_role`; real sessions contain none of these. REQ-002 lore realignment is a separate delegation.