//! Offline Behavior-family threshold calibration (FEATURE-004, ADR-025 §3 roadmap #3).
//! Data source: nebius/SWE-rebench-openhands-trajectories only (ADR-034).
//!
//! INPUT: JSONL file exported from HuggingFace. The dataset is stored as Parquet on
//! HuggingFace — the operator must export to JSONL first:
//!
//!   python -c "
//!   from datasets import load_dataset
//!   ds = load_dataset('nebius/SWE-rebench-openhands-trajectories', split='train')
//!   ds.to_json('trajectories.jsonl')
//!   "
//!
//! No parquet/arrow dependency is added (operator-approved decision; see task brief).
//!
//! OUTPUT: precision/recall sweep table + recommended threshold per sub-signal.
//! Advisory only — operator applies values to src/verdict.rs constants manually.

use crate::verdict::BehaviorSignals;
use crate::window::TREND_TAIL_K;
use std::{io::BufRead, path::Path};

// ── CLI types ────────────────────────────────────────────────────────────────

/// CLI arguments for `brim calibrate`.
#[derive(Debug, clap::Args)]
pub struct CalibrateArgs {
    /// Path to JSONL dataset file (operator-downloaded from HuggingFace; see module doc).
    #[arg(long)]
    pub dataset: std::path::PathBuf,

    /// Minimum precision (0.0–1.0) a threshold must achieve to be recommended.
    #[arg(long, default_value_t = 0.80)]
    pub precision_target: f32,

    /// Emit structured JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

// ── Serde types: nebius ──────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct NebiusTraj {
    #[serde(default)]
    trajectory: Vec<NebiusMsg>,
    #[serde(default)]
    resolved: u8,
    #[serde(default)]
    exit_status: Option<String>,
    #[serde(default)]
    gen_tests_correct: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct NebiusMsg {
    role: String,
    #[serde(default)]
    tool_calls: Vec<NebiusToolCall>,
    #[serde(default)]
    content: serde_json::Value,
}

#[derive(serde::Deserialize)]
struct NebiusToolCall {
    function: NebiusFunction,
}

#[derive(serde::Deserialize)]
struct NebiusFunction {
    name: String,
    #[serde(default)]
    arguments: String,
}

// ── Internal calibration types ───────────────────────────────────────────────

#[derive(Debug, Default)]
struct ExtractionStats {
    n_lines: usize,
    n_skipped: usize,
    n_excluded: usize,
}

#[derive(Debug, Clone)]
struct CalibSample {
    repetition_run: u32,
    failure_streak: u32,
    ping_pong_count: u32,
    is_positive: bool,
}

impl CalibSample {
    fn from_behavior(signals: Option<BehaviorSignals>, is_positive: bool) -> Self {
        let (rep, streak, pp) = signals.map_or((0, 0, 0), |s| {
            (
                s.repetition_run.unwrap_or(0),
                s.failure_streak.unwrap_or(0),
                s.ping_pong_count.unwrap_or(0),
            )
        });
        CalibSample {
            repetition_run: rep,
            failure_streak: streak,
            ping_pong_count: pp,
            is_positive,
        }
    }
}

// ── Hash + content helpers ────────────────────────────────────────────────────

/// FNV-1a 64-bit: deterministic, no external dep, stable across runs.
fn fnv1a_64(data: &[u8]) -> u64 {
    const BASIS: u64 = 14695981039346656037u64;
    const PRIME: u64 = 1099511628211u64;
    data.iter()
        .fold(BASIS, |h, &b| (h ^ b as u64).wrapping_mul(PRIME))
}

/// Stable hash of a JSON args string: parse → re-serialize (BTreeMap-sorted keys) → FNV-64.
/// Falls back to hashing the raw string on parse error.
fn stable_hash_args_str(args_json: &str) -> u64 {
    let v: serde_json::Value = serde_json::from_str(args_json)
        .unwrap_or_else(|_| serde_json::Value::String(args_json.to_string()));
    let canonical = serde_json::to_string(&v).unwrap_or_else(|_| args_json.to_string());
    fnv1a_64(canonical.as_bytes())
}

/// Extract plain text from a message content field (string or content-part array).
fn content_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// True if content signals a tool-execution error.
/// Runs only on downloaded dataset content — NOT on live session transcripts.
///
/// Patterns (from ADR-025 dataset-vs-runtime split):
///   [... exit code N]  where N is a positive integer, `[` and `]` on the same line
///   Lines starting with ERROR: / Traceback / Exception:
pub fn is_error_content(content: &str) -> bool {
    for line in content.lines() {
        // [... exit code N] — all three: '[' before, digits, ']' after, on one line.
        if let Some(ec_pos) = line.find("exit code ") {
            let before = &line[..ec_pos];
            if before.contains('[') {
                let after = &line[ec_pos + 10..];
                let n_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !n_str.is_empty() && after[n_str.len()..].contains(']') {
                    let n: u64 = n_str.parse().unwrap_or(0);
                    if n > 0 {
                        return true;
                    }
                }
            }
        }
        if line.starts_with("ERROR:")
            || line.starts_with("Traceback")
            || line.starts_with("Exception:")
        {
            return true;
        }
    }
    false
}

// ── Label predicates ─────────────────────────────────────────────────────────

/// Nebius "clearly-stuck" predicate (operator-approved label definition, task brief).
///
/// Returns:
///   Some(true)  — positive: agent failed, ran tests, did not submit (clearly stuck/spinning)
///   Some(false) — negative: agent resolved the issue (resolved == 1)
///   None        — excluded: failed but didn't try, or edge-case submitted without resolving
pub fn nebius_label(resolved: u8, exit_status: &str, gen_tests_correct: u64) -> Option<bool> {
    if resolved == 1 {
        return Some(false);
    }
    let not_submitted = exit_status != "submit";
    let has_tests = gen_tests_correct > 0;
    if not_submitted && has_tests {
        Some(true)
    } else {
        None
    }
}

fn value_as_u64(v: &Option<serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::Number(n)) => {
            n.as_u64()
                .or_else(|| n.as_f64().map(|f| f as u64))
                .unwrap_or(0)
        }
        Some(serde_json::Value::Bool(b)) => *b as u64,
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Maximum JSONL line size (4 MiB). Lines exceeding this are skipped and counted in
/// n_skipped. Guards against a single multi-GB line exhausting process memory; uses
/// fill_buf/consume to avoid growing an unbounded String before detecting the limit.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

// ── Signal extraction ─────────────────────────────────────────────────────────

/// Extract tool_calls and error_flags from a nebius trajectory message sequence.
fn nebius_signals_from_msgs(
    msgs: &[NebiusMsg],
    exit_status: &str,
) -> (Vec<(String, u64)>, Vec<bool>, bool) {
    let mut tool_calls: Vec<(String, u64)> = Vec::new();
    let mut error_flags: Vec<bool> = Vec::new();
    let stop_reason = exit_status != "submit";
    for msg in msgs {
        match msg.role.as_str() {
            "assistant" => {
                for tc in &msg.tool_calls {
                    let hash = stable_hash_args_str(&tc.function.arguments);
                    tool_calls.push((tc.function.name.clone(), hash));
                }
            }
            "tool" => {
                let text = content_text(&msg.content);
                error_flags.push(is_error_content(&text));
            }
            _ => {}
        }
    }
    (tool_calls, error_flags, stop_reason)
}

fn extract_nebius_signals(path: &Path) -> (Vec<CalibSample>, ExtractionStats) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("calibrate: cannot open {:?}: {e}", path);
            return (Vec::new(), ExtractionStats::default());
        }
    };
    let mut reader = std::io::BufReader::new(file);
    let mut samples: Vec<CalibSample> = Vec::new();
    let mut stats = ExtractionStats::default();
    let mut acc: Vec<u8> = Vec::with_capacity(8192);

    loop {
        // Read one line via fill_buf/consume, capped at MAX_LINE_BYTES.
        // This avoids growing an unbounded buffer when a line is multi-GB.
        acc.clear();
        let mut oversize = false;
        let mut io_err = false;
        loop {
            let avail = match reader.fill_buf() {
                Ok([]) => break, // EOF
                Ok(b) => b,
                Err(_) => { io_err = true; break; }
            };
            let nl = avail.iter().position(|&b| b == b'\n');
            let chunk = nl.map(|p| p + 1).unwrap_or(avail.len());
            if !oversize && acc.len() + chunk <= MAX_LINE_BYTES {
                acc.extend_from_slice(&avail[..chunk]);
            } else {
                oversize = true;
            }
            reader.consume(chunk);
            if nl.is_some() { break; }
        }

        if io_err { stats.n_skipped += 1; break; }
        if acc.is_empty() { break; } // EOF
        if oversize { stats.n_skipped += 1; continue; }

        let line_str = match std::str::from_utf8(&acc) {
            Ok(s) => s,
            Err(_) => { stats.n_skipped += 1; continue; }
        };
        let line = line_str.trim();
        if line.is_empty() { continue; }
        stats.n_lines += 1;

        let traj: NebiusTraj = match serde_json::from_str(line) {
            Ok(t) => t,
            Err(_) => {
                stats.n_skipped += 1;
                continue;
            }
        };

        let gen_tests = value_as_u64(&traj.gen_tests_correct);
        let exit = traj.exit_status.as_deref().unwrap_or("");
        let label = match nebius_label(traj.resolved, exit, gen_tests) {
            Some(l) => l,
            None => {
                stats.n_excluded += 1;
                continue;
            }
        };

        let (tool_calls, error_flags, stop_reason) =
            nebius_signals_from_msgs(&traj.trajectory, exit);
        // Window to last TREND_TAIL_K entries to match the live Behavior detection path
        // (copilot.rs:312-315, codex.rs:197-200). Thresholds fit on full-trajectory counts
        // would not transfer to the windowed runtime.
        let tc_start = tool_calls.len().saturating_sub(TREND_TAIL_K);
        let ef_start = error_flags.len().saturating_sub(TREND_TAIL_K);
        let signals =
            BehaviorSignals::from_signals(&tool_calls[tc_start..], &error_flags[ef_start..], stop_reason);
        samples.push(CalibSample::from_behavior(signals, label));
    }

    (samples, stats)
}

// ── Threshold sweep ───────────────────────────────────────────────────────────

const MAX_THRESHOLD: u32 = 10;
/// Minimum positive-sample count to recommend a threshold (small-sample guard per plan §3).
const MIN_POSITIVE_SAMPLES: usize = 50;

struct ThresholdRow {
    t: u32,
    tp: u32,
    fp: u32,
    fn_: u32,
    precision: f32,
    recall: f32,
    f1: f32,
}

struct SignalTable {
    signal_name: &'static str,
    n_positive: usize,
    n_negative: usize,
    rows: Vec<ThresholdRow>,
    recommended: Option<u32>,
    note: &'static str,
}

struct ThresholdTable {
    repetition: SignalTable,
    streak: SignalTable,
    ping_pong: SignalTable,
}

fn sweep_signal(
    samples: &[CalibSample],
    get_val: impl Fn(&CalibSample) -> u32,
    signal_name: &'static str,
    min_precision: f32,
) -> SignalTable {
    let n_positive = samples.iter().filter(|s| s.is_positive).count();
    let n_negative = samples.iter().filter(|s| !s.is_positive).count();

    let mut rows: Vec<ThresholdRow> = Vec::with_capacity(MAX_THRESHOLD as usize);
    for t in 1..=MAX_THRESHOLD {
        let tp = samples
            .iter()
            .filter(|s| s.is_positive && get_val(s) >= t)
            .count() as u32;
        let fp = samples
            .iter()
            .filter(|s| !s.is_positive && get_val(s) >= t)
            .count() as u32;
        let fn_ = samples
            .iter()
            .filter(|s| s.is_positive && get_val(s) < t)
            .count() as u32;
        let precision = if tp + fp == 0 {
            1.0f32
        } else {
            tp as f32 / (tp + fp) as f32
        };
        let recall = if tp + fn_ == 0 {
            0.0f32
        } else {
            tp as f32 / (tp + fn_) as f32
        };
        let f1 = if precision + recall < 1e-9 {
            0.0f32
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        rows.push(ThresholdRow {
            t,
            tp,
            fp,
            fn_,
            precision,
            recall,
            f1,
        });
    }

    let (recommended, note) = if n_positive < MIN_POSITIVE_SAMPLES {
        (
            None,
            "insufficient positive samples (<50); retain candidate T=3",
        )
    } else {
        // Require tp > 0: a threshold with zero true positives is evidence-free
        // and must never be recommended regardless of its vacuous precision.
        let rec = rows
            .iter()
            .filter(|r| r.tp > 0 && r.precision >= min_precision)
            .min_by_key(|r| r.t)
            .map(|r| r.t);
        let no_tp = rows.iter().all(|r| r.tp == 0);
        let note = if rec.is_none() {
            if no_tp {
                "insufficient signal evidence — no recommendation"
            } else {
                "no threshold achieves precision target; retain candidate T=3"
            }
        } else {
            ""
        };
        (rec, note)
    };

    SignalTable {
        signal_name,
        n_positive,
        n_negative,
        rows,
        recommended,
        note,
    }
}

fn sweep_thresholds(samples: &[CalibSample], min_precision: f32) -> ThresholdTable {
    ThresholdTable {
        repetition: sweep_signal(
            samples,
            |s| s.repetition_run,
            "repetition_run",
            min_precision,
        ),
        streak: sweep_signal(
            samples,
            |s| s.failure_streak,
            "failure_streak",
            min_precision,
        ),
        ping_pong: sweep_signal(
            samples,
            |s| s.ping_pong_count,
            "ping_pong_count",
            min_precision,
        ),
    }
}

// ── Output ────────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct CalibrateJsonReport {
    pub dataset: String,
    pub format: String,
    pub n_lines_read: usize,
    pub n_malformed_skipped: usize,
    pub n_excluded: usize,
    pub n_positive: usize,
    pub n_negative: usize,
    pub precision_target: f32,
    pub signals: Vec<SignalJsonReport>,
    pub recommended: RecommendedJson,
    pub note: String,
}

#[derive(serde::Serialize)]
pub struct SignalJsonReport {
    pub signal: &'static str,
    pub n_positive: usize,
    pub n_negative: usize,
    pub rows: Vec<RowJson>,
    pub recommended: Option<u32>,
    pub note: &'static str,
}

#[derive(serde::Serialize)]
pub struct RowJson {
    pub t: u32,
    pub tp: u32,
    pub fp: u32,
    pub fn_: u32,
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
}

#[derive(serde::Serialize)]
pub struct RecommendedJson {
    pub repetition: Option<u32>,
    pub streak: Option<u32>,
    pub ping_pong: Option<u32>,
}

fn signal_to_json(t: &SignalTable) -> SignalJsonReport {
    SignalJsonReport {
        signal: t.signal_name,
        n_positive: t.n_positive,
        n_negative: t.n_negative,
        rows: t
            .rows
            .iter()
            .map(|r| RowJson {
                t: r.t,
                tp: r.tp,
                fp: r.fp,
                fn_: r.fn_,
                precision: r.precision,
                recall: r.recall,
                f1: r.f1,
            })
            .collect(),
        recommended: t.recommended,
        note: t.note,
    }
}

fn print_human(
    table: &ThresholdTable,
    stats: &ExtractionStats,
    args: &CalibrateArgs,
    n_positive: usize,
    n_negative: usize,
) {
    println!("# brim calibrate — Behavior-family threshold sweep");
    println!("# dataset: {}  format: nebius", args.dataset.display());
    println!(
        "# lines_read: {}  malformed_skipped: {}  excluded: {}",
        stats.n_lines, stats.n_skipped, stats.n_excluded
    );
    println!("# n_positive: {n_positive}  n_negative: {n_negative}");
    println!(
        "# precision_target: {:.2}",
        args.precision_target
    );
    println!(
        "# FORMAT NOTE: HF datasets are Parquet; export to JSONL first (see module doc)."
    );
    println!();

    for st in [&table.repetition, &table.streak, &table.ping_pong] {
        println!(
            "## {} (positive={} negative={})",
            st.signal_name, st.n_positive, st.n_negative
        );
        println!(
            "{:>3}  {:>6}  {:>6}  {:>6}  {:>9}  {:>7}  {:>7}",
            "T", "TP", "FP", "FN", "precision", "recall", "F1"
        );
        for row in &st.rows {
            println!(
                "{:>3}  {:>6}  {:>6}  {:>6}  {:>9.3}  {:>7.3}  {:>7.3}",
                row.t, row.tp, row.fp, row.fn_, row.precision, row.recall, row.f1
            );
        }
        match st.recommended {
            Some(t) => println!("RECOMMENDED: {}={t}", st.signal_name),
            None => println!("NOTE: {}", st.note),
        }
        println!();
    }

    let fmt_rec = |r: Option<u32>| {
        r.map(|v| v.to_string())
            .unwrap_or_else(|| "3 (candidate — no calibrated value)".to_string())
    };
    println!("RECOMMENDED thresholds (apply to src/verdict.rs — advisory only):");
    println!(
        "  BEHAVIOR_REPETITION_THRESHOLD = {}",
        fmt_rec(table.repetition.recommended)
    );
    println!(
        "  BEHAVIOR_STREAK_THRESHOLD     = {}",
        fmt_rec(table.streak.recommended)
    );
    println!(
        "  BEHAVIOR_PING_PONG_THRESHOLD  = {}",
        fmt_rec(table.ping_pong.recommended)
    );
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run_calibration(args: &CalibrateArgs) -> anyhow::Result<()> {
    let (samples, stats) = extract_nebius_signals(&args.dataset);

    let table = sweep_thresholds(&samples, args.precision_target);
    let n_positive = samples.iter().filter(|s| s.is_positive).count();
    let n_negative = samples.iter().filter(|s| !s.is_positive).count();

    if args.json {
        let report = CalibrateJsonReport {
            dataset: args.dataset.display().to_string(),
            format: "nebius".to_string(),
            n_lines_read: stats.n_lines,
            n_malformed_skipped: stats.n_skipped,
            n_excluded: stats.n_excluded,
            n_positive,
            n_negative,
            precision_target: args.precision_target,
            signals: vec![
                signal_to_json(&table.repetition),
                signal_to_json(&table.streak),
                signal_to_json(&table.ping_pong),
            ],
            recommended: RecommendedJson {
                repetition: table.repetition.recommended,
                streak: table.streak.recommended,
                ping_pong: table.ping_pong.recommended,
            },
            note: "HF datasets are Parquet; export to JSONL first. Advisory only.".to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&table, &stats, args, n_positive, n_negative);
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── T1: content-regex ────────────────────────────────────────────────────

    #[test]
    fn calibrate_content_regex_exit_code_positive() {
        assert!(is_error_content("[some output exit code 1]"));
        assert!(is_error_content("running\n[exit code 127]\ndone"));
        assert!(is_error_content("[Command failed exit code 2]"));
    }

    #[test]
    fn calibrate_content_regex_exit_code_zero_is_clean() {
        assert!(!is_error_content("[exit code 0]")); // 0 is not a positive integer
        assert!(!is_error_content("exit code 1")); // no '[' before
    }

    #[test]
    fn calibrate_content_regex_requires_closing_bracket() {
        // no closing ']' on the same line → no match
        assert!(!is_error_content("[exit code 1 no closing bracket"));
        // '[' on a prior line must not count — anchored to same line
        assert!(!is_error_content("open bracket[\nnext line exit code 1"));
    }

    #[test]
    fn calibrate_content_regex_error_lines() {
        assert!(is_error_content("ERROR: file not found"));
        assert!(is_error_content("output\nERROR: segfault\nmore"));
        assert!(is_error_content("Traceback (most recent call last):"));
        assert!(is_error_content("Exception: unexpected value"));
    }

    #[test]
    fn calibrate_content_regex_clean_output() {
        assert!(!is_error_content("normal output"));
        assert!(!is_error_content("no error: here")); // 'no error:' not 'ERROR:'
        assert!(!is_error_content(""));
        assert!(!is_error_content("SUCCESS: all tests passed"));
    }

    // ── T2: nebius stuck predicate ───────────────────────────────────────────

    #[test]
    fn calibrate_nebius_label_positive_clearly_stuck() {
        // not submitted + has tests + unresolved → positive
        assert_eq!(nebius_label(0, "error", 2), Some(true));
        assert_eq!(nebius_label(0, "", 5), Some(true));
        assert_eq!(nebius_label(0, "timeout", 1), Some(true));
    }

    #[test]
    fn calibrate_nebius_label_negative_resolved() {
        // resolved=1 → negative regardless of other fields
        assert_eq!(nebius_label(1, "submit", 0), Some(false));
        assert_eq!(nebius_label(1, "error", 5), Some(false));
    }

    #[test]
    fn calibrate_nebius_label_excluded_no_tests() {
        // not submitted but no tests → excluded
        assert_eq!(nebius_label(0, "error", 0), None);
    }

    #[test]
    fn calibrate_nebius_label_excluded_submitted_unresolved() {
        // submitted but unresolved (edge case) → excluded
        assert_eq!(nebius_label(0, "submit", 2), None);
    }

    // ── T3: nebius signal extraction from message sequence ──────────────────

    #[test]
    fn calibrate_nebius_signals_from_msgs_tool_calls_and_errors() {
        let msgs_json = r#"[
          {"role":"assistant","tool_calls":[{"function":{"name":"bash","arguments":"{\"command\":\"python test.py\"}"}}],"content":null},
          {"role":"tool","content":"[Command output exit code 1]"},
          {"role":"user","content":"continue"},
          {"role":"assistant","tool_calls":[{"function":{"name":"bash","arguments":"{\"command\":\"python test.py\"}"}}],"content":null},
          {"role":"tool","content":"[Command output exit code 1]"}
        ]"#;
        let msgs: Vec<NebiusMsg> = serde_json::from_str(msgs_json).unwrap();
        let (tool_calls, error_flags, stop_reason) =
            nebius_signals_from_msgs(&msgs, "error");

        assert_eq!(tool_calls.len(), 2, "two assistant tool calls");
        // both calls are identical (same name + same args) → same hash
        assert_eq!(tool_calls[0].0, "bash");
        assert_eq!(tool_calls[0], tool_calls[1], "identical calls must hash equal");
        assert_eq!(error_flags.len(), 2);
        assert!(error_flags[0], "exit code 1 is an error");
        assert!(error_flags[1]);
        // exit_status != "submit" → stop_reason = true
        assert!(stop_reason);
    }

    #[test]
    fn calibrate_nebius_signals_skips_user_messages() {
        let msgs_json = r#"[
          {"role":"user","content":"please fix this"},
          {"role":"assistant","tool_calls":[{"function":{"name":"edit","arguments":"{\"path\":\"a.py\"}"}}],"content":null},
          {"role":"tool","content":"success"}
        ]"#;
        let msgs: Vec<NebiusMsg> = serde_json::from_str(msgs_json).unwrap();
        let (tool_calls, error_flags, _) = nebius_signals_from_msgs(&msgs, "submit");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(error_flags.len(), 1);
        assert!(!error_flags[0], "clean output is not an error");
    }

    #[test]
    fn calibrate_nebius_stop_reason_proxy_submit() {
        let msgs: Vec<NebiusMsg> = Vec::new();
        let (_, _, stop_reason) = nebius_signals_from_msgs(&msgs, "submit");
        assert!(!stop_reason, "submit → stop_reason = false");
    }

    // ── T4: full nebius extraction from JSONL fixture ────────────────────────

    #[test]
    fn calibrate_nebius_extraction_from_file() {
        // Two trajectories: one positive (stuck), one negative (resolved).
        // Positive: exit_status=error, gen_tests_correct=2, resolved=0
        //   - 2 identical assistant tool calls → repetition_run=2 → streak of 2 errors
        // Negative: resolved=1
        let line_positive = r#"{"trajectory":[{"role":"assistant","tool_calls":[{"function":{"name":"bash","arguments":"{\"cmd\":\"run\"}"}}],"content":null},{"role":"tool","content":"[exit code 1]"},{"role":"assistant","tool_calls":[{"function":{"name":"bash","arguments":"{\"cmd\":\"run\"}"}}],"content":null},{"role":"tool","content":"[exit code 1]"}],"resolved":0,"exit_status":"error","gen_tests_correct":2}"#;
        let line_negative =
            r#"{"trajectory":[],"resolved":1,"exit_status":"submit","gen_tests_correct":0}"#;
        let line_excluded =
            r#"{"trajectory":[],"resolved":0,"exit_status":"error","gen_tests_correct":0}"#;
        let line_malformed = r#"{"bad json"#;

        let path = std::env::temp_dir().join("brim_calibrate_test_nebius.jsonl");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            format!("{line_positive}\n{line_negative}\n{line_excluded}\n{line_malformed}\n"),
        )
        .unwrap();

        let (samples, stats) = extract_nebius_signals(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(stats.n_lines, 4, "4 non-empty lines");
        assert_eq!(stats.n_skipped, 1, "1 malformed line");
        assert_eq!(stats.n_excluded, 1, "1 excluded line");
        assert_eq!(samples.len(), 2, "1 positive + 1 negative");

        let pos = samples.iter().find(|s| s.is_positive).unwrap();
        let neg = samples.iter().find(|s| !s.is_positive).unwrap();

        // positive sample: 2 identical tool calls → repetition_run=2; 2 errors → failure_streak=2
        assert_eq!(pos.repetition_run, 2, "two identical calls → run=2");
        assert_eq!(pos.failure_streak, 2, "two errors → streak=2");

        // negative sample: empty trajectory
        assert_eq!(neg.repetition_run, 0);
        assert_eq!(neg.failure_streak, 0);
    }

    // ── T4b: window truncation ───────────────────────────────────────────────

    #[test]
    fn calibrate_window_truncates_to_trend_tail_k() {
        // Trajectory: TREND_TAIL_K + 4 consecutive identical tool calls and error flags.
        // Full-trajectory failure_streak = TREND_TAIL_K + 4.
        // Windowed to last TREND_TAIL_K entries: failure_streak = TREND_TAIL_K.
        // This proves windowing is applied before from_signals, not after.
        use crate::window::TREND_TAIL_K;

        let total = TREND_TAIL_K + 4;
        let repeat_args = r#"{"cmd":"repeat"}"#;
        let tool_calls: Vec<(String, u64)> = (0..total)
            .map(|_| ("bash".to_string(), stable_hash_args_str(repeat_args)))
            .collect();
        let error_flags = vec![true; total]; // all errors

        // Full trajectory
        let sig_full = BehaviorSignals::from_signals(&tool_calls, &error_flags, false);
        let streak_full = sig_full.as_ref().and_then(|s| s.failure_streak).unwrap_or(0);
        assert_eq!(streak_full, total as u32, "full trajectory streak = total count");

        // Windowed (production path)
        let tc_start = tool_calls.len().saturating_sub(TREND_TAIL_K);
        let ef_start = error_flags.len().saturating_sub(TREND_TAIL_K);
        let sig_win = BehaviorSignals::from_signals(
            &tool_calls[tc_start..],
            &error_flags[ef_start..],
            false,
        );
        let streak_win = sig_win.as_ref().and_then(|s| s.failure_streak).unwrap_or(0);
        assert_eq!(
            streak_win,
            TREND_TAIL_K as u32,
            "windowed streak must be capped at TREND_TAIL_K"
        );
        assert!(
            streak_win < streak_full,
            "windowed streak must be smaller than full-trajectory streak"
        );
    }

    // ── T5: threshold sweep with synthetic fixture ───────────────────────────
    //
    // Fixture: 60 positives with repetition_run=5, failure_streak=0
    //          10 negatives with repetition_run=1, failure_streak=0
    //          10 negatives with repetition_run=2, failure_streak=0
    //
    // Expected at precision_target=0.80:
    //   T=1: TP=60, FP=20, precision=60/80=0.75 < 0.80 ✗
    //   T=2: TP=60, FP=10, precision=60/70≈0.857 ≥ 0.80 ✓  ← recommended
    //   T=3: TP=60, FP=0,  precision=1.0 ≥ 0.80 ✓
    // → smallest T meeting target = 2

    #[test]
    fn calibrate_sweep_picks_expected_threshold() {
        let mut samples: Vec<CalibSample> = (0..60)
            .map(|_| CalibSample {
                repetition_run: 5,
                failure_streak: 0,
                ping_pong_count: 0,
                is_positive: true,
            })
            .collect();
        samples.extend((0..10).map(|_| CalibSample {
            repetition_run: 1,
            failure_streak: 0,
            ping_pong_count: 0,
            is_positive: false,
        }));
        samples.extend((0..10).map(|_| CalibSample {
            repetition_run: 2,
            failure_streak: 0,
            ping_pong_count: 0,
            is_positive: false,
        }));

        let table = sweep_thresholds(&samples, 0.80);
        assert_eq!(
            table.repetition.recommended,
            Some(2),
            "smallest T meeting 0.80 precision is T=2"
        );
        // streak/ping_pong have all-zero values in this fixture → tp=0 at every threshold.
        // Fix 1: tp==0 rows are excluded from selection; no recommendation emitted.
        assert_eq!(table.streak.recommended, None, "all-zero signal: no evidence, no recommendation");
        assert_eq!(table.ping_pong.recommended, None, "all-zero signal: no evidence, no recommendation");
        assert_eq!(table.streak.note, "insufficient signal evidence — no recommendation");
        assert_eq!(table.ping_pong.note, "insufficient signal evidence — no recommendation");

        // Verify the row values for repetition at T=2
        let row_t2 = table
            .repetition
            .rows
            .iter()
            .find(|r| r.t == 2)
            .unwrap();
        assert_eq!(row_t2.tp, 60);
        assert_eq!(row_t2.fp, 10);
        assert!((row_t2.precision - 60.0 / 70.0).abs() < 0.001);
    }

    #[test]
    fn calibrate_sweep_insufficient_samples_returns_none() {
        // Only 10 positive samples → below MIN_POSITIVE_SAMPLES=50 → recommended=None
        let samples: Vec<CalibSample> = (0..10)
            .map(|_| CalibSample {
                repetition_run: 5,
                failure_streak: 5,
                ping_pong_count: 5,
                is_positive: true,
            })
            .chain((0..5).map(|_| CalibSample {
                repetition_run: 0,
                failure_streak: 0,
                ping_pong_count: 0,
                is_positive: false,
            }))
            .collect();

        let table = sweep_thresholds(&samples, 0.80);
        assert_eq!(table.repetition.recommended, None);
        assert_eq!(table.streak.recommended, None);
        assert_eq!(table.ping_pong.recommended, None);
        assert!(!table.repetition.note.is_empty(), "note explains why");
    }

    #[test]
    fn calibrate_sweep_empty_input_produces_empty_report() {
        let samples: Vec<CalibSample> = Vec::new();
        let table = sweep_thresholds(&samples, 0.80);
        assert_eq!(table.repetition.n_positive, 0);
        assert_eq!(table.repetition.n_negative, 0);
        assert_eq!(table.repetition.rows.len(), MAX_THRESHOLD as usize);
        assert_eq!(table.repetition.recommended, None);
    }

    // ── T6: JSON report shape ────────────────────────────────────────────────

    #[test]
    fn calibrate_json_report_serializes_required_fields() {
        let report = CalibrateJsonReport {
            dataset: "/data/trajectories.jsonl".to_string(),
            format: "nebius".to_string(),
            n_lines_read: 100,
            n_malformed_skipped: 2,
            n_excluded: 10,
            n_positive: 50,
            n_negative: 38,
            precision_target: 0.80,
            signals: vec![SignalJsonReport {
                signal: "repetition_run",
                n_positive: 50,
                n_negative: 38,
                rows: vec![RowJson {
                    t: 1,
                    tp: 50,
                    fp: 5,
                    fn_: 0,
                    precision: 0.909,
                    recall: 1.0,
                    f1: 0.952,
                }],
                recommended: Some(1),
                note: "",
            }],
            recommended: RecommendedJson {
                repetition: Some(1),
                streak: None,
                ping_pong: None,
            },
            note: "advisory only".to_string(),
        };

        let json = serde_json::to_string(&report).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(v.get("n_positive").is_some());
        assert!(v.get("n_negative").is_some());
        assert!(v.get("signals").is_some());
        assert!(v.get("recommended").is_some());
        assert!(v.get("precision_target").is_some());
        assert!(v.get("note").is_some());

        let recommended = &v["recommended"];
        assert!(recommended.get("repetition").is_some());
        assert!(recommended.get("streak").is_some());
        assert!(recommended.get("ping_pong").is_some());

        let signals = v["signals"].as_array().unwrap();
        assert!(!signals.is_empty());
        let sig0 = &signals[0];
        assert!(sig0.get("signal").is_some());
        assert!(sig0.get("rows").is_some());
        assert!(sig0.get("recommended").is_some());

        let rows = sig0["rows"].as_array().unwrap();
        let row0 = &rows[0];
        assert!(row0.get("t").is_some());
        assert!(row0.get("tp").is_some());
        assert!(row0.get("fp").is_some());
        assert!(row0.get("fn_").is_some());
        assert!(row0.get("precision").is_some());
        assert!(row0.get("recall").is_some());
        assert!(row0.get("f1").is_some());
    }

    // ── T7: stable hash determinism ──────────────────────────────────────────

    #[test]
    fn calibrate_stable_hash_deterministic() {
        // Same args JSON string → same hash every time.
        let args = r#"{"command":"python test.py","cwd":"/repo"}"#;
        let h1 = stable_hash_args_str(args);
        let h2 = stable_hash_args_str(args);
        assert_eq!(h1, h2);
    }

    #[test]
    fn calibrate_stable_hash_key_order_independent() {
        // Two JSON strings with different key order → same canonical form → same hash.
        let a = r#"{"b":2,"a":1}"#;
        let b = r#"{"a":1,"b":2}"#;
        // serde_json::Map uses BTreeMap (no preserve_order feature), so re-serialization sorts keys.
        assert_eq!(stable_hash_args_str(a), stable_hash_args_str(b));
    }

    #[test]
    fn calibrate_stable_hash_different_args_differ() {
        let h1 = stable_hash_args_str(r#"{"command":"ls"}"#);
        let h2 = stable_hash_args_str(r#"{"command":"pwd"}"#);
        assert_ne!(h1, h2);
    }
}
