# brim — Recycle Verdict: Mathematical Model

> Reference model consolidated from the ADRs/REQs cited below and the
> implementation; authoritative source is the code.

Exact to `src/window.rs` (`compute_window_info`, `compute_trend`) and
`src/verdict.rs` (`absolute_verdict`, constants), per ADR-006 (projection),
ADR-008 (cache signal), ADR-010 (OR-gate + 32k/128k anchors), ADR-011
(absolute tokens only — no advertised-window lookup), REQ-004 (configurable
thresholds), REQ-005 (JSON contract).

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

**Velocity.** From positive consecutive deltas of $P$:

$$D^+ = \big(\, w_{k} - w_{k-1} \ :\ s < k \le n,\ w_k > w_{k-1} \,\big)$$

$$
v =
\begin{cases}
\operatorname{med}^{+}(D^+) & |P| \ge 2 \ \wedge\ D^+ \neq \varnothing \\
\varnothing & \text{otherwise}
\end{cases}
$$

$\operatorname{med}^{+}$ = **upper median**: sort $D^+$ ascending $d_{(1)}\le\dots\le d_{(m)}$,
take $d_{(\lfloor m/2\rfloor + 1)}$ (zero-based index $\lfloor m/2\rfloor$).

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

- Internal end-to-end code trace (provider feed → aggregation → call-site
  confirming the single-session model is the actual per-node path) was NOT
  completed this session. Fidelity is corroborated indirectly by the two
  independent reimplementations above, but the internal trace remains TODO.
