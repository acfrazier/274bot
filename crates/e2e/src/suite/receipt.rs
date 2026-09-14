//! Child output parsing and terminal-receipt validation.
//!
//! The native executables print their terminal decisions as plain lines (see
//! `crates/panel/src/app.rs`):
//!
//! ```text
//! PASS: live <scenario> <evidence json>
//! FAIL: live <scenario> <evidence json>
//! CATALOG_CORE: <scenario> <core witness json>      (catalog_watch only)
//! PAIRED_CORE: <scenario> <pair witness json>       (pair_watch only)
//! ```
//!
//! A zero exit code alone is *not* qualification: the suite requires exactly one
//! terminal scenario receipt, the witness the manifest declares for that case, matching
//! identities, and — when the case requests one — a real PNG/JSON capture pair. Parsing
//! is per-contract, never a permissive "PASS" regex.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::manifest::{CaptureSpec, CaseEntry, RunnerKind};

pub const PASS_PREFIX: &str = "PASS: live ";
pub const FAIL_PREFIX: &str = "FAIL: live ";
pub const CORE_PREFIX: &str = "CATALOG_CORE: ";
pub const PAIR_PREFIX: &str = "PAIRED_CORE: ";
pub const SHOT_PREFIX: &str = "[panel] shot ";

/// Tokens that mean the shared harness (not the case) is unhealthy.
const SHARED_SIGNALS: &[&str] = &[
    "engine unavailable",
    "failed to prepare",
    "panicked at",
    "panic:",
    "infrastructure failure",
    "capture failed",
    "capture error",
    "capture failure",
    "catalog core and pair core watches are mutually exclusive",
];

/// PNG signature: existence and file magic are *not* visual approval, only capture
/// integrity.
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalReceipt {
    pub name: String,
    pub payload: Option<Value>,
    pub line: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedReceipts {
    pub scenario_pass: Option<TerminalReceipt>,
    pub scenario_fail: Option<TerminalReceipt>,
    pub core: Option<TerminalReceipt>,
    pub pair: Option<TerminalReceipt>,
    pub duplicates: Vec<String>,
    pub malformed: Vec<String>,
    pub shot_lines: Vec<String>,
    pub shared_signals: Vec<String>,
}

impl ParsedReceipts {
    /// Whether any terminal scenario decision was seen.
    pub fn has_scenario_terminal(&self) -> bool {
        self.scenario_pass.is_some() || self.scenario_fail.is_some()
    }
}

/// Parse one child's combined output. Later duplicates are recorded, never overwritten.
pub fn parse(output: &str) -> ParsedReceipts {
    let mut receipts = ParsedReceipts::default();
    for raw in output.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(SHOT_PREFIX) {
            receipts.shot_lines.push(line.to_string());
        }
        let lower = line.to_lowercase();
        if let Some(signal) = SHARED_SIGNALS.iter().find(|token| lower.contains(*token)) {
            if !receipts.shared_signals.iter().any(|seen| seen == signal) {
                receipts.shared_signals.push((*signal).to_string());
            }
        }
        let slot = if line.starts_with(PASS_PREFIX) {
            Slot::ScenarioPass
        } else if line.starts_with(FAIL_PREFIX) {
            Slot::ScenarioFail
        } else if line.starts_with(CORE_PREFIX) {
            Slot::Core
        } else if line.starts_with(PAIR_PREFIX) {
            Slot::Pair
        } else {
            continue;
        };
        let rest = match slot {
            Slot::ScenarioPass => &line[PASS_PREFIX.len()..],
            Slot::ScenarioFail => &line[FAIL_PREFIX.len()..],
            Slot::Core => &line[CORE_PREFIX.len()..],
            Slot::Pair => &line[PAIR_PREFIX.len()..],
        };
        let (name, payload_text) = match rest.split_once(' ') {
            Some((name, text)) => (name.trim().to_string(), text.trim()),
            None => (rest.trim().to_string(), ""),
        };
        let payload = if payload_text.is_empty() {
            None
        } else {
            match serde_json::from_str::<Value>(payload_text) {
                Ok(value) => Some(value),
                Err(error) => {
                    receipts
                        .malformed
                        .push(format!("{name}: receipt payload is not JSON ({error})"));
                    None
                }
            }
        };
        let receipt = TerminalReceipt {
            name,
            payload,
            line: line.to_string(),
        };
        let target = match slot {
            Slot::ScenarioPass => &mut receipts.scenario_pass,
            Slot::ScenarioFail => &mut receipts.scenario_fail,
            Slot::Core => &mut receipts.core,
            Slot::Pair => &mut receipts.pair,
        };
        if let Some(existing) = target {
            receipts.duplicates.push(format!(
                "duplicate terminal receipt ({} then {})",
                summarize(existing),
                summarize(&receipt)
            ));
        } else {
            *target = Some(receipt);
        }
    }
    receipts
}

fn summarize(receipt: &TerminalReceipt) -> String {
    receipt.line.chars().take(120).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    ScenarioPass,
    ScenarioFail,
    Core,
    Pair,
}

/// A captured PNG/JSON pair written by the panel for this run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub label: String,
    pub png: String,
    pub json: String,
    pub png_bytes: u64,
    pub png_magic: bool,
    pub sidecar_ok: bool,
    pub sidecar_ingame: Option<bool>,
}

impl CaptureRecord {
    /// Structurally complete capture: real PNG bytes plus a parseable snapshot sidecar.
    pub fn complete(&self) -> bool {
        self.png_magic && self.sidecar_ok && self.png_bytes > PNG_MAGIC.len() as u64
    }
}

/// Find capture pairs for `label` under the suite's shot root. The panel writes
/// `<stamp>_<safeLabel>.png` + `.json` into its per-run directory
/// (`scenario::shot::write_shot`), and the suite points `274BOT_SMOKE_DIR` at its own
/// root, so a case's captures are exactly the pairs whose stem ends in the safe label.
pub fn find_captures(shot_root: &Path, label: &str) -> Vec<CaptureRecord> {
    captures_for_label(shot_root, label)
}

/// Capture pairs whose file stem matches `label`, sorted by path.
pub fn captures_for_label(shot_root: &Path, label: &str) -> Vec<CaptureRecord> {
    let safe = scenario::shot::safe_label(label);
    scan_captures(shot_root)
        .into_iter()
        .filter(|(stem, _)| stem.ends_with(safe.as_str()))
        .map(|(_, record)| record)
        .collect()
}

/// Every capture pair currently under the shot root, keyed by file stem. Captures are
/// attributed to a case by diffing this map before and after that case's child runs.
pub fn scan_captures(shot_root: &Path) -> std::collections::BTreeMap<String, CaptureRecord> {
    let mut records = std::collections::BTreeMap::new();
    let Ok(run_dirs) = std::fs::read_dir(shot_root) else {
        return records;
    };
    for run_dir in run_dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(run_dir.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(png) = std::fs::read(&path) else {
                continue;
            };
            let json_path = path.with_extension("json");
            let (sidecar_ok, sidecar_ingame) = match std::fs::read_to_string(&json_path) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(Value::Object(map)) => {
                        let ingame = map.get("ingame").and_then(Value::as_bool);
                        (ingame.is_some(), ingame)
                    }
                    _ => (false, None),
                },
                Err(_) => (false, None),
            };
            records.insert(
                stem.to_string(),
                CaptureRecord {
                    label: stem.to_string(),
                    png: path.display().to_string(),
                    json: json_path.display().to_string(),
                    png_bytes: png.len() as u64,
                    png_magic: png.starts_with(&PNG_MAGIC),
                    sidecar_ok,
                    sidecar_ingame,
                },
            );
        }
    }
    records
}

/// Captures that appeared while a case ran.
pub fn observed_captures(
    before: &std::collections::BTreeMap<String, CaptureRecord>,
    after: &std::collections::BTreeMap<String, CaptureRecord>,
) -> Vec<CaptureRecord> {
    after
        .iter()
        .filter(|(stem, _)| !before.contains_key(stem.as_str()))
        .map(|(_, record)| record.clone())
        .collect()
}

/// The outcome a case's receipts support.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Successful terminal receipts, no capture involved.
    Passed,
    /// Successful terminal receipts plus captures that a human has not read back yet.
    PendingVisualReview { captures: Vec<CaptureRecord> },
    /// The case asserted a failure: recorded and the suite continues while the shared
    /// harness is healthy.
    CaseFailure { reason: String },
    /// The shared harness failed: missing/malformed/conflicting receipts, an
    /// infrastructure signal, or a requested capture that did not arrive.
    SharedFailure { kind: &'static str, reason: String },
}

impl Verdict {
    pub fn is_success(&self) -> bool {
        matches!(self, Verdict::Passed | Verdict::PendingVisualReview { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Passed => "passed",
            Verdict::PendingVisualReview { .. } => "pending_visual_review",
            Verdict::CaseFailure { .. } => "failed",
            Verdict::SharedFailure { .. } => "shared_failure",
        }
    }
}

/// Validate one child's result against the case contract.
///
/// `exit_code` is `None` when the child was killed by the suite (timeout/interrupt);
/// those paths never reach validation.
pub fn validate(
    case: &CaseEntry,
    receipts: &ParsedReceipts,
    exit_code: Option<i32>,
    captures: &[CaptureRecord],
) -> Verdict {
    let shared = |kind: &'static str, reason: String| Verdict::SharedFailure { kind, reason };

    if !receipts.duplicates.is_empty() {
        return shared("receipt", receipts.duplicates.join("; "));
    }
    if !receipts.malformed.is_empty() {
        return shared("receipt", receipts.malformed.join("; "));
    }
    if !receipts.shared_signals.is_empty() {
        return shared(
            "infrastructure",
            format!(
                "child reported a shared harness failure: {}",
                receipts.shared_signals.join(", ")
            ),
        );
    }

    let expected = case.scenario.as_deref().unwrap_or_default();
    let Some(pass) = &receipts.scenario_pass else {
        // No PASS: either a FAIL receipt or nothing at all.
        let Some(fail) = &receipts.scenario_fail else {
            return shared(
                "receipt",
                format!(
                    "no terminal scenario receipt for {expected} (exit {})",
                    exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "n/a".into())
                ),
            );
        };
        if fail.name != expected {
            return shared(
                "receipt",
                format!("FAIL receipt names {:?}, expected {expected:?}", fail.name),
            );
        }
        if exit_code == Some(0) {
            return shared(
                "receipt",
                format!("{expected} printed a FAIL receipt but exited 0"),
            );
        }
        return Verdict::CaseFailure {
            reason: format!("case assertion failure: {}", failure_detail(fail, receipts)),
        };
    };

    if exit_code != Some(0) {
        return shared(
            "receipt",
            format!(
                "{expected} printed a PASS receipt but exited {}",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".into())
            ),
        );
    }
    if pass.name != expected {
        return shared(
            "receipt",
            format!("PASS receipt names {:?}, expected {expected:?}", pass.name),
        );
    }
    if let Some(Value::Object(map)) = &pass.payload {
        if let Some(observed) = map.get("scenario").and_then(Value::as_str) {
            if observed != expected {
                return shared(
                    "receipt",
                    format!("evidence names scenario {observed:?}, expected {expected:?}"),
                );
            }
        }
    }

    match case.runner() {
        RunnerKind::Core => {
            let identity = case.core_case.as_deref().unwrap_or_default();
            let Some(core) = &receipts.core else {
                return shared(
                    "receipt",
                    format!("missing CATALOG_CORE witness for {expected}"),
                );
            };
            if let Some(reason) = witness_mismatch("CATALOG_CORE", core, expected, identity) {
                return shared("receipt", reason);
            }
            if receipts.pair.is_some() {
                return shared(
                    "receipt",
                    format!("{expected} is a core case but printed a PAIRED_CORE witness"),
                );
            }
        }
        RunnerKind::Pair => {
            let identity = case.pair_case.as_deref().unwrap_or_default();
            let Some(pair) = &receipts.pair else {
                return shared(
                    "receipt",
                    format!("missing PAIRED_CORE witness for {expected}"),
                );
            };
            if let Some(reason) = witness_mismatch("PAIRED_CORE", pair, expected, identity) {
                return shared("receipt", reason);
            }
            if receipts.core.is_some() {
                return shared(
                    "receipt",
                    format!("{expected} is a pair case but printed a CATALOG_CORE witness"),
                );
            }
        }
    }

    if let Some(capture) = &case.capture {
        return capture_verdict(capture, captures, receipts);
    }
    if captures.is_empty() {
        Verdict::Passed
    } else {
        // The scenario asked for a capture of its own: it is recorded and stays
        // pending until a human actually reads it back.
        Verdict::PendingVisualReview {
            captures: captures.to_vec(),
        }
    }
}

fn failure_detail(fail: &TerminalReceipt, receipts: &ParsedReceipts) -> String {
    if let Some(Value::Object(map)) = &fail.payload {
        if let Some(message) = map.get("message").and_then(Value::as_str) {
            if !message.is_empty() {
                return message.to_string();
            }
        }
        if let Some(outcome) = map.get("outcome").and_then(Value::as_str) {
            return format!("outcome {outcome}");
        }
    }
    if !receipts.shot_lines.is_empty() {
        return format!("{} (see capture log)", fail.line);
    }
    fail.line.clone()
}

fn witness_mismatch(
    kind: &str,
    receipt: &TerminalReceipt,
    expected_scenario: &str,
    expected_identity: &str,
) -> Option<String> {
    if receipt.name != expected_scenario {
        return Some(format!(
            "{kind} witness names {:?}, expected {expected_scenario:?}",
            receipt.name
        ));
    }
    let Some(Value::Object(map)) = &receipt.payload else {
        return Some(format!(
            "{kind} witness for {expected_scenario} carries no JSON object"
        ));
    };
    match map.get("case").and_then(Value::as_str) {
        Some(observed) if observed == expected_identity => None,
        Some(observed) => Some(format!(
            "{kind} witness case {observed:?}, expected {expected_identity:?}"
        )),
        None => Some(format!(
            "{kind} witness for {expected_scenario} carries no case identity"
        )),
    }
}

fn capture_verdict(
    capture: &CaptureSpec,
    captures: &[CaptureRecord],
    receipts: &ParsedReceipts,
) -> Verdict {
    let complete: Vec<CaptureRecord> = captures
        .iter()
        .filter(|record| record.complete())
        .cloned()
        .collect();
    if capture.required && complete.is_empty() {
        let detail = if captures.is_empty() {
            "no capture file was written".to_string()
        } else {
            format!(
                "{} capture file(s) were present but incomplete ({}), shot lines: {}",
                captures.len(),
                captures
                    .iter()
                    .map(|record| format!(
                        "{} png_magic={} sidecar_ok={}",
                        record.png, record.png_magic, record.sidecar_ok
                    ))
                    .collect::<Vec<_>>()
                    .join("; "),
                receipts.shot_lines.len()
            )
        };
        return Verdict::SharedFailure {
            kind: "capture",
            reason: format!("requested capture {:?}: {detail}", capture.label),
        };
    }
    if captures.is_empty() {
        Verdict::Passed
    } else {
        Verdict::PendingVisualReview {
            captures: captures.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::manifest::SuiteManifest;

    fn manifest() -> SuiteManifest {
        SuiteManifest::parse(super::super::EMBEDDED_MANIFEST.as_bytes(), "embedded").unwrap()
    }

    fn core_case(id: &str) -> CaseEntry {
        manifest().case(id).expect("case").clone()
    }

    #[test]
    fn parses_terminal_receipts_without_a_permissive_pass_regex() {
        let output = "\
live thiever: running step 1/2
PASS: live thiever {\"scenario\":\"thiever\",\"outcome\":\"pass\"}
CATALOG_CORE: thiever {\"case\":\"Thiever\",\"post_start_observations\":2}
[panel] shot thiever paint -> /tmp/shots/run/thiever paint.png
";
        let receipts = parse(output);
        assert!(receipts.has_scenario_terminal());
        let pass = receipts.scenario_pass.as_ref().unwrap();
        assert_eq!(pass.name, "thiever");
        assert_eq!(
            pass.payload
                .as_ref()
                .unwrap()
                .get("scenario")
                .and_then(Value::as_str),
            Some("thiever")
        );
        assert_eq!(receipts.core.as_ref().unwrap().name, "thiever");
        assert_eq!(receipts.shot_lines.len(), 1);
        assert!(receipts.duplicates.is_empty());

        // "PASS" as a bare word is not a receipt.
        assert!(!parse("PASS\n").has_scenario_terminal());
        assert!(!parse("all good, PASS(ed) the case\n").has_scenario_terminal());
    }

    #[test]
    fn shared_signals_are_detected_from_child_output() {
        let receipts = parse("FATAL: engine unavailable\n");
        assert_eq!(receipts.shared_signals, vec!["engine unavailable"]);
    }

    #[test]
    fn zero_exit_without_a_receipt_is_a_shared_receipt_failure() {
        let case = core_case("thiever");
        let receipts = parse("live thiever: running step 1/2\n");
        let verdict = validate(&case, &receipts, Some(0), &[]);
        match verdict {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("no terminal scenario receipt"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn exit_zero_with_a_fail_receipt_is_contradictory() {
        let case = core_case("thiever");
        let receipts = parse("FAIL: live thiever {\"outcome\":\"fail\"}\n");
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure {
                kind: "receipt",
                ..
            }
        ));
    }

    #[test]
    fn assertion_failure_is_case_local_and_keeps_the_message() {
        let case = core_case("thiever");
        let receipts = parse(
            "FAIL: live thiever {\"scenario\":\"thiever\",\"message\":\"no Coins gained\"}\n",
        );
        match validate(&case, &receipts, Some(1), &[]) {
            Verdict::CaseFailure { reason } => {
                assert!(reason.contains("no Coins gained"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_or_wrong_witness_stops_the_run() {
        let case = core_case("thiever");
        let mut receipts = parse("PASS: live thiever {\"scenario\":\"thiever\"}\n");
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure {
                kind: "receipt",
                ..
            }
        ));

        receipts = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nCATALOG_CORE: thiever {\"case\":\"BankFletcher\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"Thiever\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // A core case must not present a pair witness.
        receipts = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nCATALOG_CORE: thiever {\"case\":\"Thiever\"}\nPAIRED_CORE: thiever {\"case\":\"Air\"}\n",
        );
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure { .. }
        ));
    }

    #[test]
    fn duplicate_terminal_receipts_are_rejected() {
        let case = core_case("thiever");
        let receipts = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nPASS: live thiever {\"scenario\":\"thiever\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("duplicate"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn valid_core_case_passes_and_captures_stay_pending() {
        let case = core_case("thiever");
        let receipts = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nCATALOG_CORE: thiever {\"case\":\"Thiever\"}\n",
        );
        assert_eq!(validate(&case, &receipts, Some(0), &[]), Verdict::Passed);

        let capture = CaptureRecord {
            label: "thiever paint".into(),
            png: "/tmp/a.png".into(),
            json: "/tmp/a.json".into(),
            png_bytes: 32,
            png_magic: true,
            sidecar_ok: true,
            sidecar_ingame: Some(true),
        };
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[capture]),
            Verdict::PendingVisualReview { .. }
        ));
    }

    #[test]
    fn a_requested_capture_that_is_missing_or_broken_is_a_shared_failure() {
        let mut case = core_case("thiever");
        case.capture = Some(CaptureSpec {
            label: "thiever paint".into(),
            required: true,
        });
        let receipts = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nCATALOG_CORE: thiever {\"case\":\"Thiever\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { kind, .. } => assert_eq!(kind, "capture"),
            other => panic!("{other:?}"),
        }

        let broken = CaptureRecord {
            label: "thiever paint".into(),
            png: "/tmp/a.png".into(),
            json: "/tmp/a.json".into(),
            png_bytes: 4,
            png_magic: false,
            sidecar_ok: false,
            sidecar_ingame: None,
        };
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[broken]),
            Verdict::SharedFailure {
                kind: "capture",
                ..
            }
        ));
    }

    #[test]
    fn pair_cases_require_their_own_witness() {
        let case = core_case("nature_crafter_air");
        let receipts = parse(
            "PASS: live nature_crafter_air {\"scenario\":\"nature_crafter_air\"}\nPAIRED_CORE: nature_crafter_air {\"case\":\"Air\"}\n",
        );
        assert_eq!(validate(&case, &receipts, Some(0), &[]), Verdict::Passed);

        let wrong = parse(
            "PASS: live nature_crafter_air {\"scenario\":\"nature_crafter_air\"}\nPAIRED_CORE: nature_crafter_air {\"case\":\"Mule\"}\n",
        );
        assert!(matches!(
            validate(&case, &wrong, Some(0), &[]),
            Verdict::SharedFailure { .. }
        ));
    }

    #[test]
    fn captures_are_found_by_label_magic_and_sidecar() {
        let dir = std::env::temp_dir().join(format!("274bot-suite-caps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = dir.join("2026-09-14T00-00-00_1234");
        std::fs::create_dir_all(&run).unwrap();
        let png = run.join("2026-09-14T00-00-01_thiever_paint.png");
        std::fs::write(&png, [PNG_MAGIC.as_slice(), &[0u8; 32]].concat()).unwrap();
        std::fs::write(
            run.join("2026-09-14T00-00-01_thiever_paint.json"),
            "{\"ingame\":true}",
        )
        .unwrap();
        std::fs::write(run.join("2026-09-14T00-00-01_other.png"), PNG_MAGIC).unwrap();

        let found = find_captures(&dir, "thiever paint");
        assert_eq!(found.len(), 1);
        assert!(found[0].complete());
        assert_eq!(found[0].sidecar_ingame, Some(true));
        assert!(find_captures(&dir, "absent label").is_empty());

        std::fs::write(
            run.join("2026-09-14T00-00-01_thiever_paint.json"),
            "not json",
        )
        .unwrap();
        let broken = find_captures(&dir, "thiever paint");
        assert_eq!(broken.len(), 1);
        assert!(
            !broken[0].complete(),
            "file magic alone is not capture integrity"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
