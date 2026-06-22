# CODERULES.md
> NASA Power-of-Ten, intent kept. Rule isn't real unless lint/type/test enforces it.

## Rules
1. flow traceable; recursion only if bounded + clearer, else iterate
2. bound all work: loops/queries/retries/recursion have hard caps; paginate
3. bound all growth: no unbounded collection/cache/buffer; state limit policy
4. one unit, one job, one abstraction level (SRP+SLAP); >1 reason to change → split
5. validate inputs + assert invariants at boundaries; fail loud + early
6. smallest scope; immutable default; no shared mutable globals
7. no silent failures: handle/propagate every error; empty catch = bug
8. boring > clever: prefer the idiom a static analyzer/reviewer reads without pausing; no magic/metaprogramming; explicit data flow
9. lint+type+format zero-warning = merge gate; tool confused → rewrite, don't suppress
10. behavior change ships failing→passing test; no test = not done
11. never trust external input; no secrets in code/logs; least privilege at trust boundaries

## Priority (when rules tension)
* correctness > security(11) > bounds(2,3) > clarity(4,8). Document the trade inline.

## Deviations
* allowed only if documented inline: `// CODERULES-N exception: <why>`; prefer refactor

## Project specifics
* toolchain (build-clean r9, test r10): Rust/Cargo. clean = `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check`; test = `cargo test`
* bounds (r2-3): cap per-transcript reads — scan from file tail for the latest `assistant` turn, don't load whole JSONL into memory; cap sub-agents listed per session; window % is bounded [0,100]
* errors (r7): `anyhow::Result` at boundaries, typed errors internally; no `.unwrap()`/`.expect()` outside tests; a malformed/partial transcript line is skipped, never panics
* boring/banned (r8): no `unsafe`; no macro metaprogramming beyond derives; explicit data flow; provider trait + plain structs over clever generics
* security (r11): read-only on local transcript files; never log transcript contents or session prompts (only ids, counts, percentages); session ids displayed truncated; no network, no secrets
