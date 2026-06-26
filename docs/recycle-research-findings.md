# brim Recycle-Advisory — Research & Decision Findings

**Status:** living research dossier. Consolidates everything established while designing the
vote-counter recycle model. In-repo, version-controlled source of truth (not the auto-memory).
**Last updated:** 2026-06-26.

Lore artifacts remain the formal source of truth for decisions (STORY/REQ/ADR); this file is
the *narrative + evidence* behind them, and the parts that don't fit a single artifact
(empirical findings, dataset research, math roadmap, confidence).

---

## 1. The goal

Replace brim's absolute-threshold OR-gate verdict (ADR-010: `w>=128k -> Over`, etc.) with a
**vote-counter**: measure many independent "worrisome signals", group into **FAMILIES**, and
recommend recycle when enough independent *families* fire. No absolute gate as the decider;
occupancy becomes one weak vote.

Hard invariants (carried from ADR-010/011/020/024, never broken):
- **Deterministic** and explainable (no black-box ML at runtime).
- **Transcript-only**: token counts, tool-call structure, timestamps. No content inspection,
  no logprobs/entropy of model output, no eval probing.
- **Advisory + read-only**: brim recommends, never recycles or mutates a session.
- **No ceiling-learning / no scaling thresholds to the advertised window** (ADR-011/020).

Subtlety that drove the whole design: count independent **families**, not raw detectors —
else Volume/Speed (the same "tank filling" story told three ways) stuffs the ballot.

**The five families:** Volume (occupancy %) · Speed (token-growth rate / projection) ·
Thrash (cache churn, compaction events, `stop_reason=max_tokens`) · Behavior (tool-call
repetition / failure-streak / ping-pong — the "true onset" detector) · Drift (slow
session-long rot a short tail misses).

---

## 2. Empirical findings (real data)

From 3 Explore passes over 442 real Claude logs + raw provider data:

- **Occupancy is NOT discriminating.** 62% of real messages >200k tokens are normal steady
  state. Strong evidence FOR killing absolute gates — the 128k gate cries wolf on healthy
  long sessions. This is the single most important finding; it justifies demoting occupancy
  to a weak vote.
- **Tool-repetition:** RARE but high-precision. Keep as a near-decisive vote. (Completed logs
  have survivorship bias vs abandoned stuck sessions, so rarity ≠ uselessness.)
- **Ping-pong A↔B:** fires in 100% of sessions *including successful ones* as first measured
  → NOT discriminating without a **no-progress qualifier** (same args / output unchanged).
- **Compaction-drops are NOT a live signal** (the wall already hit) — they are a potential
  ground-truth label for calibration, not a predictor.
- **New high-value signals found:** `stop_reason=="max_tokens"` (decisive, rare, unambiguous),
  `is_error` streaks, `apiErrorStatus:429`, `<synthetic>` model fallback.

---

## 3. Cross-provider signal availability (verified)

Behavioral signals were initially assumed Claude-only. That was a **brim-parsing artifact**,
not a format absence — brim's codex/copilot/opencode parsers only ever extracted occupancy.
Verified availability of tool-call STRUCTURE (name, args, error flag, ordering):

**FORMAT-available ≠ WIRED-in-brim.** The "Tool structure (format)" column states whether the
provider's transcript *exposes* the tool-block structure the Behavior family needs; the "Wired in
brim?" column states whether brim's parser actually *extracts* it. A provider can have the format
yet stub `behavior:None`, in which case its Behavior family can never fire regardless of format.

| Provider | Tool structure (format) | Wired in brim? | Error discriminator | Args | Notes |
|---|---|---|---|---|---|
| **Claude** | Yes (live data) | **Yes** (`claude.rs`) | `is_error` boolean | object | Fully implemented |
| **codex** | Yes (spec-derived, no local data) | **Yes** (`codex.rs:98`) | `function_call_output.status='failed'` | JSON **string** (parse before hash) | `~/.codex` absent on machine |
| **opencode** | Yes (verified, 1.17.9) | **Yes — WIRED** (live-verified 1.17.9, `opencode.rs::extract_opencode_behavior`; ADR-028) | `state.status='error'` | JSON **object** | tool rows in `part` (`data.type='tool'`); name `data.tool`, args `data.state.input`, last K=8 tail; stub removed, Behavior fires |
| **copilot** | **No** — process-log source carries no tool structure | **No** (`copilot.rs:113` `behavior:None`) | — | — | neither format nor wiring; Behavior family can never fire |

Key gotchas baked into the design: **no provider uses a uniform boolean `is_error`** (status
discriminators); **codex args are a string, opencode/Claude args are objects** (string needs
parse before hashing). Runtime error detection stays **structural** per provider — the
no-content-inspection invariant holds at runtime.

**Forward-compat note (not a current bug):** on opencode 1.17.9 (verified) step-finish is still
written to the `part` table; the `session_message` table exists in the schema but carries no
step-finish rows. A real `part` step-finish row was read and its token JSON shape matches brim's
oracle exactly. brim's `opencode.rs` prefers `session_message` *if present* and falls back to
`part` — defensive forward-compat for a possible future schema move, NOT a reproduced bug. No
observed opencode version actually relocates step-finish, so the aggregate fallback does not fire
on current versions.

---

## 4. The vote-counter design (ADR-025, Accepted) + amendments

**The decided vote rule:**
1. **Threshold:** recommend recycle when **≥2 of 5 families** fire.
2. **Weights:** Behavior + Thrash high; Volume/occupancy lowest.
3. **Decisive override:** a single unambiguous signal recommends recycle alone, bypassing the
   count — `stop_reason=max_tokens` OR a confirmed tool-loop.
4. **Tiers:** family **count** drives the 5 presentation tiers (lean/drift/bloated/stale/
   critical); Behavior/Drift split stale-vs-critical (closes the STORY-012 stage-4/5
   pure-occupancy blindness).

**Two amendments (user-approved 2026-06-26; require superseding ADR-026):**
- **M1 — backstop as decisive override.** Occupancy at/above `recycle_backstop` fires Over
  alone. *Why:* behavior-blind providers (copilot/opencode = `behavior:None`) could otherwise
  never reach Over; restores the STORY-011 "warn before hard-compact" floor. Occupancy stays a
  weak vote everywhere else.
- **M3 — Drift dropped from the family count.** Drift is computed from the same K=8 tail as
  Speed, so it correlated with Speed/Volume → ballot-stuffing. The count now uses **4
  genuinely-independent families** (Volume/Speed/Thrash/Behavior). `drift_score` is still
  computed + emitted in `--json`, and may only split Stale-vs-Bloated *within* the ≥2 band —
  never escalate. Real long-horizon Drift (EWMA across resets) is deferred to roadmap #2.

**Implementation status (2026-06-26):** vote-counter implemented, 165 tests pass, clippy
clean. Audit: PASS (read-only, fail-closed, deterministic, no secret leak all hold). Review:
no blockers, faithful to ADR-025. The M1/M3 + minor fixes are in flight at time of writing.

---

## 5. External literature — grounds the design rationale

Long-context degradation research (does NOT supply calibration thresholds, but corroborates
the "occupancy weak / behavior strong" thesis):

- **arXiv 2505.06120 "LLMs Get Lost in Multi-Turn"** — strongest support. 200K+ sim convos;
  Concat≈Full (95.1%) ⇒ degradation is driven by multi-turn **dynamics/unreliability, NOT raw
  token count**. "Once a model takes a wrong turn it doesn't recover" = our Behavior family,
  externally validated.
- **arXiv 2505.10570 "LongFuncEval"** — best agentic-specific source. Tool-use accuracy drops
  7–91% as tool catalog / response length / turns grow. Usable to quantify the Volume↔tool-use
  interaction.
- Occupancy *does* degrade quality (gradual, no cliff, model-specific): RULER (2404.06654),
  NoLiMa (2502.05167 — at 32K, 11/13 models <50% of short baseline), Chroma "Context Rot",
  LongBench v2, HELMET. Lost-in-the-Middle (TACL 2024) — degradation is *positional*, U-shaped.
- **arXiv 2511.00197** — failed code-agent traces run 12–82% longer, but correlational (task
  difficulty confounds occupancy). Rationale-support only.

---

## 6. Dataset research — four passes, one firm conclusion

Goal: find public data to calibrate the model. **The families bifurcate by data availability.**

- **Behavior family** = the ONE family public datasets feed. Sources: nebius/SWE-rebench
  (Unresolved split = failures retained), nvidia/Open-SWE-Traces, the error-analysis corpus
  (failure-dense), τ²-bench (deterministic forced errors across providers).
- **Volume / Speed / Thrash / Drift** = NOT public-calibratable. No public dataset has
  per-turn token usage AND compaction boundaries together. Must capture locally.

**Verified against real raw rows (not cards — all HF viewers were broken):**

| Dataset | Per-step tokens | Compaction event | Error flag | Outcome label |
|---|---|---|---|---|
| **OpenHands/openhands-evaluation-outputs** | ✅ `usage` incl. cache tokens | ❌ (condenser was OFF) | ✅ `exit_code` | partial (`resolved`) |
| **hkust-nlp/Toolathlon-Trajectories** | ❌ session-total only | ❌ (`truncations`=0) | ❌ (free text) | ✅ done/fail/timeout/max_turn |
| **CAT-Instruct (2512.22087)** | — | built for it, but **NOT released** | — | — |

**The orphan field — compaction/recycle-EVENT marker — is absent from ALL public data.**
Confirmed across four research passes:
1. No curated trajectory dataset records compaction boundaries.
2. OpenHands per-step tokens exist, but that dump ran with the condenser OFF.
3. Framework runtime logs (MemGPT/Letta eviction, OpenHands `CondensationEvent`, LangGraph
   trim, Aider, ReSum/ACON) DO carry compaction events — but **every downloadable one is
   HEURISTIC-triggered** (a hardcoded token threshold = the "~70% budget" folklore, which has
   no benchmark behind it). The only *outcome-driven* trigger, **ERGO (2510.14077)** — resets
   on a Shannon-entropy spike — ships **no labeled trajectories** (code only).
4. CAT-Instruct, the one dataset built around the field, is paper-only.

**Conclusion (firm):** outcome-validated recycle timing is **structurally unsourceable
online** — it's a runtime artifact nobody publishes and a counterfactual nobody runs. It must
be **self-generated.**

Useful crumbs kept: `github.com/swe-bench/experiments` (OpenHands Verified submissions run
`LLMSummarizingCondenser` ON → real `CondensationEvent`s, *heuristic*, good for **detector
sanity-checks** + real compaction-point token curves); ERGO entropy-spike as a *signal idea*
(but likely needs logprobs transcripts lack → may not fit the invariant).

---

## 7. The calibration problem — and why local data is the only path

What "calibrating" means: replace the guessed thresholds with values learned from real
examples of "all good" (long but healthy) and "went bad" (stuck/looping) sessions.

**The operator's actual behavior (confirmed):** recycles at **100–300k tokens on a 1M-context
model** (10–30% full), by habit / at task breaks — NOT on brim's recommendation, NOT on
stuck-detection. Consequences:

1. **Circularity.** Because recycle is occupancy-triggered, calibrating to these recycle-event
   labels would re-learn *occupancy* as the trigger — re-inflating Volume, starving Behavior,
   contradicting the whole design. Operator recycle-events are **not** a valid calibration
   target for the non-occupancy families.
2. **Data drought.** Preventive early recycling means sessions rarely run to behavioral
   failure → few/no local stuck examples. Same structural gap as "never compacts."
3. **What local data IS good for:** healthy-occupancy baseline (direct evidence occupancy is
   non-discriminating — healthy at 10–30%) and per-step token curves.
4. **The non-circular path:** OUTCOME labels — occasionally let brim's *behavioral*
   recommendation drive a recycle (A/B against the occupancy habit) and log whether it helped
   / was premature. Roadmap #3; a nice-to-have, not a blocker.

**Calibration source split:**
| Source | Good for | Effort |
|---|---|---|
| Public failure datasets (nebius Unresolved, Toolathlon timeout/max_turn) | the stuck/loop trigger (Behavior) | zero — already exist |
| Local recycle events (the harvester) | healthy-occupancy baseline + token curves | zero — logs normal workflow |
| Optional small A/B (~5–10, opportunistic, never to the 1M wall) | non-circular local outcome labels | small |

**What the user actually needs to do:** let the harvester log normal sessions; keep
recycling as usual; *optionally* let a handful of sessions run slightly past the usual point
and report whether they degraded. The minimum is "switch on logging and keep working."

**The harvester** (STORY-013 / REQ-017, Draft): per-event append of one label record =
occupancy + the family fire-vector over N turns before the event + provenance, with
`event_type ∈ {recycle, compaction}` (primary = recycle event, secondary = rare compaction).
Per-provider graceful degradation (absence yields zero labels, never an error), append-only
local log, deterministic/transcript-only, ADR-004 consumer-side persistence.

---

## 8. Window-size & cross-model transferability

Does calibration depend on total context window, and does it extrapolate across models?

- **Behavior + Thrash (the high-weight signals) are window- AND model-independent.** A loop is
  a loop; "same tool called 8× in a row" means the same on a 200k or 1M model, Claude or
  codex. They measure an *observed symptom*, not a fullness level. **Calibrate once →
  transfers.**
- **Volume / Speed / Drift (occupancy-flavored) are window-DEPENDENT.** "100k tokens" is 50%
  of a 200k model but 10% of 1M. Absolute occupancy thresholds do **not** transfer across
  window sizes — which is exactly why they're demoted to weak votes.
- **Across model strengths:** degradation *onset* is model-specific (weaker models break
  earlier; cf. ERGO's per-model entropy thresholds). But because we trigger on the *symptom
  that already happened* (the loop occurred) rather than predicting propensity, behavior
  thresholds should be robust. *Hypothesis, not yet proven* — testable on the multi-model
  public datasets (Toolathlon spans 17 models).

**1M-host corollary:** brim's 128k backstop fires at **12.8%** of a 1M window — far too eager
for the window, and the operator is healthy recycling at 10–30%. This reinforces the
occupancy-demotion. The vote-counter is the structural answer (replaces the fixed gate rather
than retuning it), which is why the open 1M-anchor question resolves into ADR-025 rather than
a threshold tweak.

---

## 9. Math roadmap (v2 direction)

The two hard problems are **separable** and want **different math**; sequential change
detection is plumbing on top.

- **P1 = correlated families (ballot-stuffing).** Best answer: **inverse-covariance
  decorrelation** — Mahalanobis / Hotelling T², online as **MEWMA** (Lowry et al. 1992). Σ⁻¹
  collapses Drift/Speed/Volume redundancy into ~one effective dimension *by construction* —
  the rigorous version of the M3 "drop Drift" patch (decorrelate, don't drop). Cheap,
  deterministic, **label-free**. Caveats: needs a mostly-healthy corpus for Σ; must be made
  one-sided so it doesn't fire on benign novelty. Folds in the existing EWMA-Drift.
- **P2 = no labels.** Best answer: **weak-supervision label model** — Dawid-Skene (1979) →
  Snorkel/correlation-aware (Ratner 2019) → closed-form triplet (Fu 2020). Estimates
  per-signal accuracy + correlation **without ground truth**, fit offline and **frozen** into
  static log-odds weights (`weight_i = log(acc_i/(1−acc_i))`, Nitzan-Paroush 1982) → runtime
  stays deterministic. **Blunt caveat: 5 families is near the identifiability floor.** Must
  split into ~8–10 atomic signals (Behavior = 3) and validate stability with the triplet
  estimator on real transcripts before trusting it; if unstable, fall back to expert-set
  weights and P2 stays unsolved-by-math.
- **Runtime fusion:** CUSUM per-family (Page 1954) + SPRT/log-odds across families (Wald 1945)
  — the correct generalization of the hard "≥2 count". Inherits P1 double-counting unless its
  inputs are decorrelated first.
- **Rejected:** survival/hazard (Cox/KM — needs degradation-event times we lack); Dempster-
  Shafer (pathological under correlated evidence); BOCPD (overkill for a CLI).

Proposed v2 architecture: **MEWMA on a decorrelated family vector (P1) → log-odds weights
(P2) → CUSUM/SPRT gate (runtime).** This raises the roadmap ceiling (a path to lift Production
confidence past ~60); it is NOT today's confidence, and is a substantial re-architecture — not
to be chased while v1 fixes are in flight.

---

## 10. Confidence assessment + the ≥80 goal

**Stated goal:** all three categories **over 80**.

**Is 80 the right bar?** Yes — for *this* tool. brim is **advisory, deterministic, and
read-only**: it recommends, never acts, and the cost of a wrong recommendation is low (the
operator ignores it). 80 is a defensible "trustworthy enough to rely on" threshold. Chasing
much past ~85 would be **false precision** — the timing signal is inherently fuzzy (degradation
is gradual, model-specific, no cliff), and a deterministic advisory does not need 95%+ certainty
to beat the occupancy-gate it replaces (which it already does). So: **80 across all three is
sufficient and is the goal; >85 is not worth the cost.**

| Category | Current | Target | Path to ≥80 |
|---|---|---|---|
| **Design soundness** | ~87 ✅ | ≥80 | **Already met.** Vote-counter shape corroborated (2505.06120, LongFuncEval). Hold; a parameter sweep once data exists would only firm it up. |
| **Evidence backing** | ~79 | ≥80 | **Essentially there.** Occupancy-weak (442 logs) + behavior-strong are well-grounded; per-step token data now exists (OpenHands) to sanity-check detectors. Crosses 80 the moment behavior thresholds are checked against the public failure datasets (nebius Unresolved, Toolathlon). Cheap. |
| **Production** | ~60 | ≥80 | **The real work.** Two independent routes: (a) **MEWMA inverse-covariance decorrelation** — buildable now, *label-free*, dissolves the P1 ballot-stuffing rigorously (§9) → lifts Production on its own; (b) **local outcome labels** via the harvester + small A/B (§7) → calibrates the thresholds. (a)+(b) together get Production over 80; (a) alone likely reaches ~70–75. |

The model is a **well-evidenced hypothesis**, not a validated model. We're confident in the
*diagnosis* (occupancy is a bad sole signal) and the *shape of the cure*; we are **not** yet
confident in the *numbers*. Design is met. Evidence is one cheap measurement pass away.
**Production is the one that requires real work** — and the honest, label-free first move is
MEWMA (P1), with outcome-label calibration (P2) the follow-through.

**Honest caveat on Production-80:** route (b) depends on generating outcome labels the operator's
current occupancy-recycling habit does not produce (§7), and route (a)'s label-free P2 cousin
(the weak-supervision model) may not be identifiable at our signal count (§9). So Production ≥80
is *achievable but not guaranteed* — if the label model proves unstable and the A/B isn't run,
Production realistically caps around 70–75 on MEWMA-decorrelation alone. That would still be a
clear improvement over today, just short of the 80 bar.

---

## 11. Open questions / decisions pending

- **Calibration strategy fork (future):** break circularity via (i) outcome-label A/B
  experiment, (ii) public-failure datasets only, or (iii) both.
- **Label-model identifiability:** prototype the triplet estimator on real transcripts before
  trusting P2 math. Biggest single uncertainty.
- **DeepNLP/Agent-Tool-Use dataset:** verify tool results + error flags, or drop.

## 12. Lore artifact map

- **STORY-012** — warn on stuck/spinning context. **STORY-013** — collect recycle labels.
- **REQ-016** — candidate signal catalog. **REQ-017** — compaction/recycle-label harvester.
  **REQ-005** — `--json` contract (gains tier/family_votes/decisive_override).
- **ADR-010** — original OR-gate (Accepted, not edited). **ADR-024** — behavioral gate
  (Accepted). **ADR-025** — vote-counter model (Accepted). **ADR-026** — amends ADR-025 with
  the M1/M3 amendments (Accepted; supersession recorded in prose, ADR-025 left unedited).
