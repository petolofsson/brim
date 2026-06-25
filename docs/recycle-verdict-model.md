# brim — Recycle Verdict: Mathematical Model

> Reference model consolidated from the ADRs/REQs cited below and the
> implementation; authoritative source is the code.

Exact to `src/window.rs` (`compute_window_info`, `compute_trend`) and
`src/verdict.rs` (`absolute_verdict`, constants), per ADR-006 (projection),
ADR-008 (cache signal), ADR-010 (OR-gate + 32k/128k anchors), ADR-011
(absolute tokens only — no advertised-window lookup), ADR-021 (velocity = max,
no headroom gate), REQ-004 (configurable thresholds), REQ-005 (JSON contract).

## Definitions

Per-turn token components of the latest assistant turn $n$:

$$i_n = \text{input}, \quad r_n = \text{cache\_read}, \quad c_n = \text{cache\_create}$$

**Window occupancy** (active tokens in context; `compute_window_info`,
`src/window.rs`):

$$w_n = i_n + r_n + c_n$$

**Cache-hit ratio** (ADR-008):

$$
\rho_n =
\begin{cases}
\min\!\left(1,\ \dfrac{r_n}{w_n}\right) & w_n > 0 \ \wedge\ (r_n + c_n) > 0 \\[2mm]
\varnothing & \text{otherwise}
\end{cases}
$$

## Trend (ADR-006)

Computed in `compute_trend` (`src/window.rs`). Bounded timeline
$W = (w_1, \dots, w_n)$, with $|W| \le K = 8$ (`TREND_TAIL_K`).

**Post-reset segment.** A reset (compaction) is an index where occupancy
drops; the segment starts at the floor immediately after the last such drop
(matching `points[post_reset_start..]`, where `post_reset_start` is the index
of the lower point):

$$s = \max\Big(\{1\} \cup \{\, k \in [2,n] : w_k < w_{k-1} \,\}\Big)$$

(index of the post-compaction floor; $1$ if no reset). Post-reset segment:
$P = (w_{s}, \dots, w_n)$.

**Velocity.** The **maximum** of positive consecutive deltas of $P$ (ADR-021,
superseding ADR-018's p90 and ADR-006's upper-median):

$$D^+ = \big(\, w_{k} - w_{k-1} \ :\ s < k \le n,\ w_k > w_{k-1} \,\big)$$

$$
v =
\begin{cases}
\max(D^+) & |P| \ge 2 \ \wedge\ D^+ \neq \varnothing \\
\varnothing & \text{otherwise}
\end{cases}
$$

`max` is the deliberate pessimistic estimator: an advisory should err early on
bursts. At $K = 8$ the positive-delta vector has length $\le 7$, so a nearest-rank
p90 would collapse to this same last order statistic — `max` *is* what p90 denotes
in brim's regime (ADR-021). Trade-off: an isolated large delta keeps $v$ high for
up to $K = 8$ turns until it scrolls off the tail — sticky over-warning, accepted
for an advisory that sits before the host's hard backstop.

**Projection** (turns to recycle backstop $B$); $\dot\div$ = integer division,
$a \mathbin{\dot-} b = \max(0, a-b)$ (saturating):

$$
\tau =
\begin{cases}
\min\!\big(\,(B \mathbin{\dot-} w_n)\ \dot\div\ v,\ \ 2^{32}-1\,\big) & v \neq \varnothing \\
\varnothing & v = \varnothing
\end{cases}
$$

Note $w_n \ge B \Rightarrow \tau = 0$.

## Verdict (ADR-010 OR-gate)

Computed in `absolute_verdict` (`src/verdict.rs`). Per ADR-011 the verdict keys
entirely off absolute tokens — no advertised-window fill ratio is consulted.
The OR-gate has **5 gates**; ADR-021 dropped ADR-018's proposed headroom guard
(dead by construction once $v = \max$, always preempted by the projection-recycle
gate).

Parameters: watch $T_w = 32{,}000$, backstop $B = 128{,}000$,
$\tau_R = 2$, $\tau_N = 5$, cache floor $\theta = 0.20$.

Guarded comparison: $P \le^{\!*} x \equiv (P \neq \varnothing) \wedge (P \le x)$
(false when $P = \varnothing$); similarly $P <^{\!*} x$.

$$
V(w_n, \tau, \rho_n) =
\begin{cases}
\textbf{Over} & w_n \ge B \\
\textbf{Over} & \tau \le^{\!*} \tau_R \\
\textbf{Nearing} & w_n \ge T_w \\
\textbf{Nearing} & \tau \le^{\!*} \tau_N \\
\textbf{Nearing} & (\tau \neq \varnothing) \wedge (\rho_n <^{\!*} \theta) \\
\textbf{Ok} & \text{otherwise}
\end{cases}
$$

Evaluated **top-to-bottom, first true wins** (severity order).

Equivalently, as a lattice join over independent gate firings with
$\textbf{Over} \succ \textbf{Nearing} \succ \textbf{Ok}$:

$$
V = \bigsqcup
\begin{cases}
\textbf{Over} & w_n \ge B \ \vee\ \tau \le^{\!*} \tau_R \\
\textbf{Nearing} & w_n \ge T_w \ \vee\ \tau \le^{\!*} \tau_N \ \vee\ \big((\tau \neq \varnothing) \wedge (\rho_n < \theta)\big) \\
\textbf{Ok} & \text{always}
\end{cases}
$$

The cache-thrash term carries the guard $\tau \neq \varnothing$ — no verdict from
the cache ratio without a real trend (cold-start suppression).

The verdict surfaces as the `verdict` enum (`ok | nearing | over_recycle`) in
the REQ-005 JSON contract.

## Boundary conventions

- Token gates: inclusive, $\ge$.
- Projection gates: inclusive, $\le$.
- Cache floor: strict, $<$ (so $\rho_n = \theta \Rightarrow$ no fire).
- $\varnothing$ (missing $\tau$ or $\rho$) never fires a gate.

## Configurability

| Symbol | Constant | Value | Tunable |
|--------|----------|-------|---------|
| $T_w$ | `ABSOLUTE_WATCH_TOKENS` | 32,000 | yes (REQ-004) |
| $B$ | `ABSOLUTE_RECYCLE_BACKSTOP` | 128,000 | yes (REQ-004) |
| $\tau_N$ | `PROJECTION_NEARING_TURNS` | 5 | no |
| $\tau_R$ | `PROJECTION_RECYCLE_TURNS` | 2 | no |
| $\theta$ | `CACHE_THRASH_THRESHOLD` | 0.20 | no |
| $K$ | `TREND_TAIL_K` | 8 | no |

## Validation

Three independent validation sources were obtained; an honest gap remains.

### External literature (research agent) — empirical anchors ~78/100

- **32k watch band: SUPPORTED.** NoLiMa (Modarressi et al., ICML 2025,
  arxiv 2502.05167): at 32K, 11 of 13 models drop below 50% of short-length
  baseline. RULER (Hsieh et al., NVIDIA, COLM 2024, arxiv 2404.06654): only
  half of models claiming ≥32K maintain performance at 32K.
- **128k backstop: SUPPORTED (conservative)** — RULER effective lengths +
  InfiniteBench (arxiv 2402.13718): near-universal collapse before 128K.
- **Token proxy `input+cache_read+cache_create`: SUPPORTED** — matches
  Anthropic's own `total_input_tokens` definition exactly; caveats: excludes
  output tokens (occupy next turn) and extended-thinking blocks auto-stripped
  across turns.
- **CAVEAT (biggest weakness):** ADR-011's "degradation is absolute, not
  window-fraction" OVERCLAIMS as a universal law. Refuted for literal recall
  (Gemini 1.5 holds >99.7% to 1M); defensible only for reasoning/agentic
  context — which is brim's domain. Recommend narrowing ADR-011 rationale
  wording (ADR-011 is Accepted; would need a superseding ADR).

### Adversarial reimplementation + fuzz critique (and addendum)

- Independent reimplementation from this spec + 250k random fuzz traces:
  absolute OR-gates never breached, cold-start guard intact, OR-gate/lattice
  equivalence confirmed — corroborates spec fidelity.
- Projection layer is optimistic on bursty agentic traffic (calm→spike
  structurally invisible; upper-median undershoots spiky deltas). Addendum
  reframes brim as an ADVISORY pre-warning before the host's hard
  auto-compaction, making lead-time the product and the optimistic estimator
  the central finding. Tracked in ADR-018 (Draft). Cache-thrash gate
  under-wired tracked in ADR-019 (Draft). Spec note #4 fixed (above); #5
  tail-truncation and #6 `cache_read`-as-occupancy documented as stated
  assumptions (note: #6 is correct by design — cached tokens occupy and are
  attended identically per Anthropic docs).

### Statistical critique (third perspective)

- Bayesian/parametric uncertainty approaches (beta-binomial cache ratio,
  Theil-Sen, Mann-Kendall, Normal-error projection, probabilistic thresholds)
  evaluated and REJECTED for the n≤8, heavy-tailed, deterministic-by-requirement
  regime; risk-aware intent met deterministically by the ADR-018 headroom
  guard. See ADR-018/ADR-019 "Alternatives considered (rejected)".

### Scope / non-goals (stated honestly)

- **(A)** Token-economics health check, NOT a reasoning-quality/relevance-decay
  monitor: a window full of stale tool output reads as healthy
  occupancy/growth/cache-ratio while reasoning degrades.
- **(B)** Short horizon: $K=8$ tail + trend reset on every compaction catches
  fast local symptoms, largely misses slow session-long drift.

### Outstanding

- Internal end-to-end code trace (provider feed → aggregation → call-site)
  **COMPLETED**. Model fidelity is **CONFIRMED** as the actual per-node decision
  path:
  - **Area 2 — aggregation (97/100):** verdict is computed per-node via
    `absolute_verdict`; no roll-up alters a node's own verdict.
  - **Area 3 — call-site (92/100):** `compute_window_info` → `compute_trend` →
    `absolute_verdict` is wired exactly; thresholds come from the CLI only, and
    nothing mutates the verdict after computation.
  - **Area 1 — provider feed (88/100):** surfaced one bug —
    `projected_turns_to_recycle` was computed against the hardcoded 128k backstop
    instead of the configurable `--recycle-backstop` (REQ-004) at `claude.rs:170`,
    `codex.rs:338`, `opencode.rs:366`, `copilot.rs:134`. Fixed in a separate commit.

## Roadmap / Future improvements

Five improvements agreed 2026-06-25, ranked by value, flagged by cost. The first
three keep the current invariant (deterministic, transcript-tokens-only); #4
changes the REQ-005 JSON verdict contract; #5 abandons the transcript-only
invariant outright.

### Cheap — keeps the deterministic / transcript-tokens-only design

1. **Learn the real context ceiling from compaction events.** brim already
   detects host auto-compaction as an occupancy drop in the timeline (the reset
   index $s$ in *Trend*). Reuse that point as an observed estimate of *this*
   agent's true window on its current plan. Use it for an **honest occupancy %
   display only — NOT to scale thresholds**: per ADR-011 degradation is ~absolute
   for reasoning/agentic work, so a bigger window just means more room to bloat
   past the anchor, not a higher anchor. No model→window table, no manual entry,
   works across providers. Today the displayed % is relative to the fixed 128k
   backstop $B$.
2. **Long-horizon drift signal.** The $K=8$ trend tail resets on every
   compaction (scope gap **B**), so it catches fast local symptoms but misses
   slow session-long drift. Add an EWMA / floor-trend over the full timeline
   *across* resets. Stays token-only and deterministic.
3. **Empirical calibration.** Anchors (watch $T_w$ / backstop $B$ and the
   presentation tiers) are research-anchored, not tuned to observed sessions. Log
   verdict-at-recycle-time vs whether recycling actually helped, then calibrate
   anchors to outcomes.

### Medium — changes the REQ-005 JSON verdict contract

4. **Promote the 5 presentation tiers into the engine.** The recycle-advisory
   recipe renders 5 severity stages (lean / drift / bloated / stale / critical)
   but these live ONLY in the example scripts as a presentation layer over the
   3-value engine verdict (`ok | nearing | over_recycle`); stages 4–5 are
   pure-occupancy because the engine enum saturates at `over_recycle`. Promoting
   to a 5-level verdict in REQ-005 + a new ADR would make the tiers
   machine-readable for any consumer and let velocity/cache-thrash refine
   stale-vs-critical above the backstop. Marginal decision-value; do only when a
   second consumer exists.

### Deep — BREAKS the deterministic / transcript-only invariant

5. **Actually measure bloat instead of proxying it.** Today a big-and-useful
   context and a big-and-bloated one are indistinguishable to brim (token counts
   can't see staleness) — scope gap **A**. A real measure requires content
   inspection (semantic analysis or an LLM-judge) to detect stale/repeated tool
   output, dead-end attempts, superseded code. Highest ceiling, heaviest
   architectural cost; abandons the transcript-tokens-only invariant. A different
   product — decide deliberately, not a next step.

**Sequencing:** #1 and #3 first (low-risk sharpening); #2 to close the slow-drift
blind spot (gap B); #4 only on a second consumer; #5 as a deliberate fork.

### Realization path (lore)

The behavioral-degradation work is now formalized in lore as the realization
path for the cheap/medium roadmap items:

- **STORY-012** — operator intent: warn on a STUCK / SPINNING context (looping,
  failing tool calls, unproductive growth), not only on raw token volume; and
  sharpen the warning stages by draining transcript signals brim already parses
  but discards.
- **REQ-016** — *Candidate verdict-signal variables*: the Tier A / Tier B / Tier
  C catalog as the candidate signal set (source field, what it predicts, verdict
  use, cross-provider availability). Tier A signals (incl. **compaction-drop
  magnitude** → #1, and the cross-reset trend inputs → #2) keep the invariant;
  Tier B realizes ADR-010 signal (a) without eval probing; Tier C is the #5 fork.
- **ADR-024** *(Draft)* — *Behavioral degradation gate from tool-call structure*:
  extends (does NOT supersede) ADR-010 by un-deferring signal (a) as a
  deterministic behavioral gate; closes the stage-4/5 (stale vs critical)
  pure-occupancy blindness.

Mapping to the items above: REQ-016's compaction-drop-magnitude signal realizes
**#1 (drop-magnitude ceiling)**; its cross-reset trajectory inputs feed **#2
(long-horizon drift)**; ADR-024's separate-behavioral-axis option is the path to
**#4 (promote the 5 tiers into the engine)**. Anchors stay candidates pending
**#3 (empirical calibration)** — none is asserted as accepted fact.
