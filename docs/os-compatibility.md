# OS compatibility — macOS & Linux

brim is a read-only Rust CLI that inspects local transcripts from four coding-agent CLIs. Scope of this document: **macOS and Linux only** (POSIX, `$HOME`-rooted paths). brim resolves every provider path through `$HOME` (see `src/parser.rs:7`), so the cross-platform question reduces to: (a) do the four CLIs persist at the same `$HOME`-relative path on both OSes, (b) does brim's native dep (`rusqlite` `bundled`) build on both, and (c) what happens when `$HOME` is unset.

## Path parity per provider

All four providers are resolved by brim as `$HOME`-relative joins (brim source: `src/claude.rs:33`, `src/codex.rs:40`, `src/opencode.rs:42-46`, `src/copilot.rs:35,39`). macOS and Linux are both POSIX; neither uses a drive letter, so path strings are byte-identical given the same `$HOME`.

| Provider | macOS path | Linux path | Same? | Source |
|---|---|---|---|---|
| Claude Code | `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl` (+ `subagents/agent-*.jsonl`) | `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl` (+ `subagents/agent-*.jsonl`) | Yes | brim `src/claude.rs:33`; path layout corroborated by Claude Code sub-agents doc (`https://docs.claude.com/en/docs/claude-code/sub-agents`, which documents `~/.claude/agents/` and the `subagents/` directory convention). The `…/transcript` doc page 404s (see Pending). |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | Yes | brim `src/codex.rs:40`; Codex CLI repo `https://github.com/openai/codex` (session rollout format lives under `~/.codex/sessions/` per `codex-rs/` source — no field-level transcript docs page beyond the in-repo layout). |
| OpenCode | `~/.local/share/opencode/opencode.db` (SQLite/WAL) | `~/.local/share/opencode/opencode.db` (SQLite/WAL) | **Pending docs confirmation** (see note) | brim `src/opencode.rs:42-46`. opencode.ai/docs/config documents config paths (`~/.config/opencode/`, and managed settings at `/Library/Application Support/opencode/` on macOS vs `/etc/opencode/` on Linux) but does **not** explicitly document the session-DB path. The dev machine install uses the XDG data dir `~/.local/share/opencode`. Whether opencode persistently uses that same XDG path on macOS (vs `~/Library/Application Support/opencode`) is **not stated in the docs I could reach** — see Pending. |
| Copilot | `~/.copilot/session-state/<uuid>/` + `~/.copilot/logs/process-<epochMs>-<pid>.log` | `~/.copilot/session-state/<uuid>/` + `~/.copilot/logs/process-<epochMs>-<pid>.log` | Yes | brim `src/copilot.rs:35,39` (REQ-002 / REQ-009, VERIFIED-LIVE per `docs/provider-capability-matrix.md`). No official Copilot-CLI transcript-path doc page was located. |

### Path-encoding wrinkles

- **Claude `<encoded-cwd>`** — Claude Code encodes the session's cwd as the project directory name by replacing `/` with `-` (e.g. `/home/pol/code/gitcake` → `-home-pol-code-gitcake`). This is a POSIX-only transformation: macOS and Linux cwds contain only `/`-separated components and no drive letter, so the encoding is byte-identical on both. brim does not re-encode; it only enumerates whatever dirnames Claude wrote (`src/claude.rs` `discover_sessions`).
- **Codex `YYYY/MM/DD`** — pure Gregorian date tree, OS-agnostic.

## $HOME resolution

`home_dir()` at `src/parser.rs:7-11` reads **only** the `HOME` environment variable, falling back to `PathBuf::from(".")` (the current working directory) when `HOME` is unset. There is no `getpwuid`/`dirs`-crate fallback.

- On macOS and Linux interactive shells `HOME` is set by `login`/the shell and is the normal case — brim finds all four provider roots under `~`.
- **Edge case (the single real cross-platform hazard):** if a user runs brim with `HOME` unset (`env -i brim`, or a headless/systemd service unit that does not set `HOME`), brim silently degrades to `"."` and finds no provider directories — it returns an empty session list rather than erroring. This is a **known edge case, not a bug to fix in this delegation** (docs-only).
- Because there is no `getpwuid` fallback, headless services (cron, systemd units without `Environment="HOME=…"`, containers with an empty env) will also degrade to `"."`. Set `HOME` explicitly in the service definition to avoid this.

## Native deps

`Cargo.toml` deps: `anyhow`, `chrono` (`clock` feature), `clap` (`derive`), `serde` (`derive`), `serde_json`, `rusqlite` `0.32` with feature `bundled`.

- **`rusqlite` `bundled`** — the `bundled` feature routes through `libsqlite3-sys`, which compiles the SQLite amalgamation (portable C source) via the `cc` crate. This removes any system `libsqlite3` dependency: no Homebrew/apt `sqlite3` needed at build or run time. docs.rs publishes `rusqlite 0.32.0` for both `x86_64-apple-darwin` and `x86_64-unknown-linux-gnu` (see the crate platform list at `https://docs.rs/rusqlite/0.32.0/rusqlite/`), confirming it builds on both targets. `aarch64-apple-darwin` (Apple Silicon) and `aarch64-unknown-linux-gnu` are standard Tier-2 Rust targets with a working C toolchain; the bundled C source builds identically there. No dep in `Cargo.toml` adds a platform constraint.
- **`chrono` `clock`** — uses `time`/system clock; portable across macOS/Linux.
- **`clap` (`derive`), `anyhow`, `serde` (`derive`), `serde_json`** — pure-Rust, platform-independent; no native lib, no per-OS behavior.

## Permissions

brim is read-only on every local transcript path (CODERULES r11: read-only on local transcript files; no network; no secrets; only ids/counts/percentages surfaced).

- **File transcripts (Claude/Codex/Copilot)** — opened via `std::fs::File::open` (`src/parser.rs:37` in `read_tail`). `File::open` is read-only; brim never opens with write/create flags. Tail reads use `SeekFrom::Start` plus a 256 KB `take()` cap (`src/parser.rs:33-43`) — both OS-agnostic stdlib calls.
- **OpenCode SQLite** — opened with `OpenFlags::SQLITE_OPEN_READ_ONLY` (`src/opencode.rs:53`) and further locked down with `pragma query_only = true` (`src/opencode.rs:56`). No read-write path exists.
- macOS and Linux are both POSIX; file ownership/permissions behave identically. brim needs only read access to the transcript files/DB the owning CLI already wrote.

## Verdict

brim runs on both macOS and Linux: all four provider paths are `$HOME`-relative and byte-identical across the two POSIX OSes (no drive letters), `rusqlite` `bundled` removes the only native dep by compiling SQLite from source and builds on both `x86_64-apple-darwin`/`aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu`, and every file/DB open is read-only (CODERULES r11). The single known cross-platform edge case is `HOME` being unset: `home_dir()` (`src/parser.rs:7-11`) has no `getpwuid` fallback and silently degrades to `.`, so under `env -i` or a mis-configured headless/systemd service brim finds no provider dirs and returns an empty list rather than erroring — set `HOME` explicitly in such environments.

## Pending

- **OpenCode session-DB path on macOS vs Linux.** opencode.ai/docs/config documents config locations (`~/.config/opencode/`, managed `/Library/Application Support/opencode/` on macOS, `/etc/opencode/` on Linux) but the page I reached does **not** state where the `opencode.db` session DB lives per OS. The dev machine uses `~/.local/share/opencode/opencode.db` (XDG data home). Whether opencode uses that same XDG path on macOS, or `~/Library/Application Support/opencode/opencode.db`, is **unconfirmed from docs** — the "Same?" cell is marked Pending for the OpenCode row above.
- **Claude Code `…/transcript` doc page** returns 404 (noted in `docs/provider-capability-matrix.md`); the `<encoded-cwd>` project-dir layout and `subagents/` convention are sourced from the sub-agents doc + brim source, not a dedicated transcript doc.
- **No field-level Codex transcript docs page** beyond the in-repo layout at `github.com/openai/codex` (`codex-rs/`); the `~/.codex/sessions/YYYY/MM/DD/` shape is sourced from brim source + the repo, not a Codex-CLI docs page.
- **No official Copilot-CLI transcript-path docs page** located; the `~/.copilot/` paths are sourced from brim source (VERIFIED-LIVE per `docs/provider-capability-matrix.md`).
- **rusqlite `aarch64-*` build confirmation** is by inference from the bundled-C-source build model + standard Tier-2 toolchains, not from a docs.rs platform listing (docs.rs lists `x86_64-apple-darwin` and `x86_64-unknown-linux-gnu` explicitly).