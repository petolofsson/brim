# AGENTS.md

**/caveman** — terse: no preamble, code over prose, smallest viable change

## Project rules
* on code tasks (not docs/chat/planning): read ./CODERULES.md if present and follow it
* binding; on conflict CODERULES wins for code rules
* absent + task triggers a Plan → offer once to create one; else silent

## Goals (higher wins on conflict)
1. correctness
2. deterministic execution
3. minimal diffs
4. low tokens
5. avoid loops
* diff-vs-completeness conflict → completeness wins, within target files only
* minimal diff = only required lines; never reformat untouched lines
* orphan cleanup: remove imports/vars/funcs YOUR change made unused; never remove pre-existing dead code
* unrelated dead code → mention, don't delete

## Stack / test (Rust)
* Rust edition 2024, Cargo. binary crate `brim` (src/main.rs). intended deps: clap (derive, CLI), serde + serde_json (JSONL transcripts), chrono (clock, dates), anyhow (errors)
* provider layer lifted/adapted from ctop (`ctop-rs/src/provider/`, `parser.rs`, `model.rs`); reads point-in-time last-turn window, not aggregate spend
* format: `cargo fmt`; lint: `cargo clippy --all-targets -- -D warnings` — run on changed code before done; never add another formatter/linter
* test: `cargo test`, scoped to the change (`cargo test <name>`); smallest subset, not full suite
* no test for change → say so, don't invent one; done = validation clean OR blocker reported

## Plan
* when requested OR required (3+ files, cross-domain, or any Stop trigger)
* must name: target files, validation command, non-goals; execution boundary, not architecture doc

**Reads** — targeted only; prefer `lore show <ID> --recursive` / `lore search <text>` over repo scans; no repo-wide scans; don't read `.lore/` files directly unless the lore CLI is unavailable; never re-read same context

## Docs (lore — Git-native project memory)
* source of truth = lore artifacts in `.lore/`: FEATURE (execution boundary), REQ (behavior), ADR (decisions), STORY (intent), TEST (expected outcome). follow linked artifacts; inspect only those relevant to the task
* "update docs" → update only the lore artifacts the change invalidates; none → say so, don't invent. changelog = git, never a file. artifacts ride the code commit
* create via `lore feature|req|adr|story|test new "<title>"`; link with `--related <ID>` or `lore link <id1> <id2>`; run `lore validate` before done
* never invent requirements, tests or ADRs; update lore when requirements or design change
* ADRs: status Draft→Accepted; never edit an Accepted ADR — supersede it with a new one
* README = router only; absent is valid — create flat default ONLY when an entry point is added/present
* README template:
  ```
  # <project> — <one-line>
  ## Run
  <cmd or → docs/SETUP.md>
  ## Map
  - behavior → AGENTS.md  - code rules → CODERULES.md
  - requirements/design/decisions → lore (`lore show <ID> --recursive`, `lore search <text>`)
  <!-- router; edit on new entry point or moved target, not a changelog -->
  ```

## Workflow
1. inspect targeted files
2. implement complete scoped change
3. self-review vs Goals — "would a senior eng call this overcomplicated? if yes, simplify"
4. run smallest relevant test
5. fix failures local to the change
6. update plan output if plan exists
7. concise summary

## Debugging
* no fix without root cause; never patch the symptom
* trace backward: symptom → immediate cause → caller → original trigger
* 3+ failed fixes → Stop (question the design, don't keep patching)

## Output
* changed files
* commands run
* test pass/fail
* blockers

## Commits / PR
* one logical change per commit (code + its docs together); imperative subject ≤50 chars
* PR body = what + why + test evidence
* never commit/log secrets, keys, credentials

**Dependencies** — don't add a dep for what stdlib/existing deps do; new dep → flag, don't add silently
**Avoid** — verbose explanations, large diffs, unrelated refactors, whole-file reformat, exploratory rewrites

## Stop if (halt + escalate)
* 3+ files or multiple domains without approved plan
* security/RBAC/tenant isolation affected
* broad architecture context required
* two failed attempts
* unclear requirements OR multiple valid approaches
* no FEATURE/lore artifact exists for the requested work (planning required)
* lore CLI unavailable when an artifact update is required
> emit: trigger | state | options | need. Do not proceed without approval.
