//! Child output parsing and terminal-receipt validation.
//!
//! The native executables print their terminal decisions as plain lines (see
//! `crates/panel/src/app.rs`):
//!
//! ```text
//! PASS: live <live name> <scenario evidence json>
//! FAIL: live <live name> <scenario evidence json>
//! CATALOG_CORE: <live name> <core witness json>     (catalog_watch only)
//! PAIRED_CORE: <live name> <pair witness json>      (pair_watch only)
//! ```
//!
//! The panel's proof name is `live.name` — the `--live` name as given, i.e.
//! `script_thiever` — while the *inner* evidence and witness identities are the scenario
//! name (`thiever`) and the host enum's serde wire form (`CoreCase::Thiever` serializes as
//! `"thiever"` because of `#[serde(rename_all = "snake_case")]`). Those are three distinct
//! values; this module validates each against its own source, never against the others.
//!
//! A zero exit code alone is *not* qualification: the suite requires exactly one terminal
//! scenario receipt (never both PASS and FAIL), the witness the scenario declares, matching
//! identities, and — when a case declares one, or when the scenario itself declares a
//! terminal shot — a capture written *by this case* that decodes as a real PNG beside a
//! snapshot sidecar recorded at scene 2 while in game. Parsing is per-contract, never a
//! permissive "PASS" regex.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::manifest::{CaptureSpec, CaseEntry, RunnerKind};
use super::SuiteResult;

pub const PASS_PREFIX: &str = "PASS: live ";
pub const FAIL_PREFIX: &str = "FAIL: live ";
pub const CORE_PREFIX: &str = "CATALOG_CORE: ";
pub const PAIR_PREFIX: &str = "PAIRED_CORE: ";
pub const SHOT_PREFIX: &str = "[panel] shot ";
/// `scene_state` a capture must have been taken at (the client's in-world state).
pub const INGAME_SCENE_STATE: i32 = 2;

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
/// integrity, and a signature alone is not a decodable image (see [`CaptureRecord`]).
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Largest capture the suite will decode. A bigger file is recorded as undecodable rather
/// than read into memory without bound.
pub const CAPTURE_DECODE_CAP: u64 = 64 * 1024 * 1024;

/// The host enum's serde wire form for a scenario's witness identity.
///
/// The witness JSON carries `"case": "thiever"` (not `"Thiever"`): `CoreCase`/`PairCase`
/// derive `Serialize` with `rename_all = "snake_case"`. Deriving the expected value from
/// the real enums keeps the suite from encoding an invented receipt shape.
pub fn wire_case(scenario: &str, runner: RunnerKind) -> SuiteResult<String> {
    let value = match runner {
        RunnerKind::Core => {
            let case = host_play::catalog_core::CoreCase::parse(scenario)
                .map_err(|error| format!("{scenario}: {error}"))?;
            serde_json::to_value(case).map_err(|error| format!("{scenario}: {error}"))?
        }
        RunnerKind::Pair => {
            let case = host_play::paired_core::PairCase::parse(scenario)
                .map_err(|error| format!("{scenario}: {error}"))?;
            serde_json::to_value(case).map_err(|error| format!("{scenario}: {error}"))?
        }
    };
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{scenario}: the host enum does not serialize as a name"))
}

/// The capture a case must produce: its own declared spec, or — when the scenario itself
/// declares a terminal shot — the panel's terminal-shot contract for that label. The panel
/// holds a PASS until the shot is written (`hold_terminal_shot`), so a PASS without the
/// capture is a harness failure, not a pass.
pub fn declared_capture(case: &CaseEntry) -> Option<CaptureSpec> {
    if let Some(capture) = &case.capture {
        return Some(capture.clone());
    }
    let scenario = case.scenario.as_deref()?;
    let label = scenario::get(scenario)?.settings.terminal_shot?;
    Some(CaptureSpec {
        label: label.to_string(),
        required: true,
    })
}

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

/// A capture pair written for this run, with the structural evidence the panel's shot
/// contract records beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub label: String,
    pub png: String,
    pub json: String,
    pub png_bytes: u64,
    pub png_magic: bool,
    /// SHA-256 of the PNG bytes: the ledger keeps a content address, not just a path.
    #[serde(default)]
    pub png_sha256: String,
    /// Whether the file decoded as a real image, not only that it carried the signature.
    #[serde(default)]
    pub decoded: bool,
    #[serde(default)]
    pub png_width: u32,
    #[serde(default)]
    pub png_height: u32,
    pub sidecar_ok: bool,
    pub sidecar_ingame: Option<bool>,
    #[serde(default)]
    pub sidecar_scene_state: Option<i32>,
    /// Pair captures carry the actor whose slot was rendered.
    #[serde(default)]
    pub sidecar_actor: Option<String>,
    /// PNG mtime (unix millis), used to show the capture belongs to this case's window.
    #[serde(default)]
    pub modified_ms: u64,
}

impl CaptureRecord {
    /// Whether the pair is the structural evidence a terminal shot must carry: a decodable
    /// PNG with pixels, written beside a snapshot sidecar recorded in game at scene 2.
    pub fn complete(&self) -> bool {
        self.structural_reason().is_none()
    }

    /// Why this capture is not complete; `None` when it is.
    pub fn structural_reason(&self) -> Option<String> {
        if !self.png_magic {
            return Some(format!("{}: not a PNG", self.png));
        }
        if self.png_bytes <= PNG_MAGIC.len() as u64 {
            return Some(format!(
                "{}: {} bytes, no image payload",
                self.png, self.png_bytes
            ));
        }
        if !self.decoded {
            return Some(format!("{}: the PNG did not decode", self.png));
        }
        if self.png_width == 0 || self.png_height == 0 {
            return Some(format!(
                "{}: decoded to an empty frame ({}x{})",
                self.png, self.png_width, self.png_height
            ));
        }
        if !self.sidecar_ok {
            return Some(format!(
                "{}: the snapshot sidecar is missing or not JSON",
                self.json
            ));
        }
        match self.sidecar_ingame {
            Some(true) => {}
            Some(false) => return Some(format!("{}: recorded while not in game", self.json)),
            None => return Some(format!("{}: carries no ingame flag", self.json)),
        }
        match self.sidecar_scene_state {
            Some(state) if state == INGAME_SCENE_STATE => {}
            Some(state) => {
                return Some(format!(
                    "{}: recorded at scene_state {state}, not {INGAME_SCENE_STATE}",
                    self.json
                ))
            }
            None => return Some(format!("{}: carries no scene_state", self.json)),
        }
        None
    }

    /// Whether this capture's file stem belongs to `label`. The panel writes
    /// `<stamp>_<safe label>.png`, and pair runs append the actor (`<base>-<actor>`), so the
    /// safe label must appear as a whole token, optionally followed by `-<actor>`.
    pub fn matches_label(&self, label: &str) -> bool {
        let safe = scenario::shot::safe_label(label);
        let stem = self.label.as_bytes();
        let needle = safe.as_bytes();
        if needle.is_empty() || needle.len() > stem.len() {
            return false;
        }
        for index in 0..=(stem.len() - needle.len()) {
            if &stem[index..index + needle.len()] != needle {
                continue;
            }
            let before_ok = index == 0 || matches!(stem[index - 1], b'_' | b'-');
            let after = index + needle.len();
            let after_ok = after == stem.len() || stem[after] == b'-';
            if before_ok && after_ok {
                return true;
            }
        }
        false
    }
}

/// Find capture pairs for `label` under the suite's shot root. The panel writes
/// `<stamp>_<safeLabel>.png` + `.json` into its per-run directory
/// (`scenario::shot::write_shot`), and the suite points `274BOT_SMOKE_DIR` at its own
/// root. Prefer [`new_captures`] when attributing captures to one case: a same-named file
/// written by an earlier case or an earlier run is not this case's evidence.
pub fn find_captures(shot_root: &Path, label: &str) -> Vec<CaptureRecord> {
    captures_for_label(shot_root, label)
}

/// Capture pairs whose file stem matches `label`, sorted by path.
pub fn captures_for_label(shot_root: &Path, label: &str) -> Vec<CaptureRecord> {
    scan_captures(shot_root)
        .into_values()
        .filter(|record| record.matches_label(label))
        .collect()
}

/// Every capture pair currently under the shot root, keyed by file stem.
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
            let (decoded, png_width, png_height) = decode_png(&png);
            let json_path = path.with_extension("json");
            let (sidecar_ok, sidecar_ingame, sidecar_scene_state, sidecar_actor) =
                match std::fs::read_to_string(&json_path) {
                    Ok(text) => match serde_json::from_str::<Value>(&text) {
                        Ok(Value::Object(map)) => (
                            true,
                            map.get("ingame").and_then(Value::as_bool),
                            map.get("scene_state")
                                .and_then(Value::as_i64)
                                .map(|state| state as i32),
                            map.get("actor").and_then(Value::as_str).map(str::to_string),
                        ),
                        _ => (false, None, None, None),
                    },
                    Err(_) => (false, None, None, None),
                };
            let modified_ms = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0);
            records.insert(
                stem.to_string(),
                CaptureRecord {
                    label: stem.to_string(),
                    png: path.display().to_string(),
                    json: json_path.display().to_string(),
                    png_bytes: png.len() as u64,
                    png_magic: png.starts_with(&PNG_MAGIC),
                    png_sha256: super::identity::sha256(&png),
                    decoded,
                    png_width,
                    png_height,
                    sidecar_ok,
                    sidecar_ingame,
                    sidecar_scene_state,
                    sidecar_actor,
                    modified_ms,
                },
            );
        }
    }
    records
}

/// Decode a PNG for real: a signature and a size are not an image.
fn decode_png(bytes: &[u8]) -> (bool, u32, u32) {
    if bytes.len() as u64 > CAPTURE_DECODE_CAP || !bytes.starts_with(&PNG_MAGIC) {
        return (false, 0, 0);
    }
    match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
        Ok(image) => (true, image.width(), image.height()),
        Err(_) => (false, 0, 0),
    }
}

/// Captures that appeared while a case ran. This is the attribution the suite uses: a
/// file that was already there belongs to whatever wrote it, not to this case.
pub fn new_captures(
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
    /// infrastructure signal, or a contracted capture that did not arrive.
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
/// `exit_code` is `None` when the child was killed by the suite (timeout/interrupt); those
/// paths never reach validation. `captures` are the captures attributed to *this* case.
pub fn validate(
    case: &CaseEntry,
    receipts: &ParsedReceipts,
    exit_code: Option<i32>,
    captures: &[CaptureRecord],
) -> Verdict {
    let shared = |kind: &'static str, reason: String| Verdict::SharedFailure { kind, reason };
    let live = case.live.as_deref().unwrap_or_default();
    let scenario = case.scenario.as_deref().unwrap_or_default();

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
    if receipts.scenario_pass.is_some() && receipts.scenario_fail.is_some() {
        return shared(
            "receipt",
            format!(
                "{live} printed both a PASS and a FAIL terminal receipt ({} / {})",
                receipts
                    .scenario_pass
                    .as_ref()
                    .map(summarize)
                    .unwrap_or_default(),
                receipts
                    .scenario_fail
                    .as_ref()
                    .map(summarize)
                    .unwrap_or_default(),
            ),
        );
    }

    let Some(pass) = &receipts.scenario_pass else {
        // No PASS: either a FAIL receipt or nothing at all.
        let Some(fail) = &receipts.scenario_fail else {
            return shared(
                "receipt",
                format!(
                    "no terminal scenario receipt for {live} (exit {})",
                    exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "n/a".into())
                ),
            );
        };
        if fail.name != live {
            return shared(
                "receipt",
                format!("FAIL receipt names {:?}, expected {live:?}", fail.name),
            );
        }
        if let Some(observed) = evidence_scenario(fail) {
            if observed != scenario {
                return shared(
                    "receipt",
                    format!("FAIL evidence names scenario {observed:?}, expected {scenario:?}"),
                );
            }
        }
        if exit_code == Some(0) {
            return shared(
                "receipt",
                format!("{live} printed a FAIL receipt but exited 0"),
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
                "{live} printed a PASS receipt but exited {}",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".into())
            ),
        );
    }
    if pass.name != live {
        return shared(
            "receipt",
            format!("PASS receipt names {:?}, expected {live:?}", pass.name),
        );
    }
    if !matches!(&pass.payload, Some(Value::Object(_))) {
        return shared(
            "receipt",
            format!("{live} PASS evidence is not a JSON object"),
        );
    }
    match evidence_scenario(pass) {
        Some(observed) if observed == scenario => {}
        Some(observed) => {
            return shared(
                "receipt",
                format!("evidence names scenario {observed:?}, expected {scenario:?}"),
            )
        }
        None => {
            return shared(
                "receipt",
                format!("{live} PASS evidence carries no scenario name"),
            )
        }
    }

    // The witness identity: the outer name is the live name, the inner `case` is the
    // scenario's host-enum wire form.
    let expected_case = match wire_case(scenario, case.runner()) {
        Ok(identity) => identity,
        Err(error) => {
            return shared(
                "receipt",
                format!("cannot derive the {scenario} witness identity: {error}"),
            )
        }
    };
    let declared = match case.runner() {
        RunnerKind::Core => case.core_case.as_deref().unwrap_or_default(),
        RunnerKind::Pair => case.pair_case.as_deref().unwrap_or_default(),
    };
    if !declared.is_empty() && declared != expected_case {
        return shared(
            "receipt",
            format!(
                "manifest witness identity {declared:?} disagrees with the host enum wire form {expected_case:?}"
            ),
        );
    }
    match case.runner() {
        RunnerKind::Core => {
            let Some(core) = &receipts.core else {
                return shared(
                    "receipt",
                    format!("missing CATALOG_CORE witness for {live}"),
                );
            };
            if let Some(reason) = witness_mismatch("CATALOG_CORE", core, live, &expected_case) {
                return shared("receipt", reason);
            }
            if receipts.pair.is_some() {
                return shared(
                    "receipt",
                    format!("{live} is a core case but printed a PAIRED_CORE witness"),
                );
            }
        }
        RunnerKind::Pair => {
            let Some(pair) = &receipts.pair else {
                return shared("receipt", format!("missing PAIRED_CORE witness for {live}"));
            };
            if let Some(reason) = witness_mismatch("PAIRED_CORE", pair, live, &expected_case) {
                return shared("receipt", reason);
            }
            if receipts.core.is_some() {
                return shared(
                    "receipt",
                    format!("{live} is a pair case but printed a CATALOG_CORE witness"),
                );
            }
        }
    }

    capture_verdict(case, captures)
}

/// The `scenario` field of a receipt payload, when it is a string.
fn evidence_scenario(receipt: &TerminalReceipt) -> Option<&str> {
    match &receipt.payload {
        Some(Value::Object(map)) => map.get("scenario").and_then(Value::as_str),
        _ => None,
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
    expected_live: &str,
    expected_case: &str,
) -> Option<String> {
    if receipt.name != expected_live {
        return Some(format!(
            "{kind} witness names {:?}, expected {expected_live:?}",
            receipt.name
        ));
    }
    let Some(Value::Object(map)) = &receipt.payload else {
        return Some(format!(
            "{kind} witness for {expected_live} carries no JSON object"
        ));
    };
    match map.get("case").and_then(Value::as_str) {
        Some(observed) if observed == expected_case => None,
        Some(observed) => Some(format!(
            "{kind} witness case {observed:?}, expected {expected_case:?}"
        )),
        None => Some(format!(
            "{kind} witness for {expected_live} carries no case identity"
        )),
    }
}

/// Capture contract for a successful case.
///
/// Fail-closed rules: an attributed capture that is not the structural terminal-shot
/// evidence is a shared failure, and a contracted capture that this case did not write is
/// a shared failure too. A capture that *is* written keeps the result in
/// [`Verdict::PendingVisualReview`]: file magic is capture integrity, never visual
/// approval.
fn capture_verdict(case: &CaseEntry, captures: &[CaptureRecord]) -> Verdict {
    let malformed: Vec<String> = captures
        .iter()
        .filter_map(|record| record.structural_reason())
        .collect();
    if !malformed.is_empty() {
        return Verdict::SharedFailure {
            kind: "capture",
            reason: format!(
                "{} capture(s) of this case are not usable terminal-shot evidence: {}",
                malformed.len(),
                malformed.join("; ")
            ),
        };
    }
    let Some(spec) = declared_capture(case) else {
        return if captures.is_empty() {
            Verdict::Passed
        } else {
            Verdict::PendingVisualReview {
                captures: captures.to_vec(),
            }
        };
    };
    let matching: Vec<&CaptureRecord> = captures
        .iter()
        .filter(|record| record.matches_label(&spec.label))
        .collect();
    if matching.is_empty() {
        if !spec.required {
            return if captures.is_empty() {
                Verdict::Passed
            } else {
                Verdict::PendingVisualReview {
                    captures: captures.to_vec(),
                }
            };
        }
        let seen = if captures.is_empty() {
            "the case wrote no capture at all".to_string()
        } else {
            format!(
                "captures written by this case: {}",
                captures
                    .iter()
                    .map(|record| record.label.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Verdict::SharedFailure {
            kind: "capture",
            reason: format!(
                "contracted capture {:?} was not written by this case ({seen})",
                spec.label
            ),
        };
    }
    Verdict::PendingVisualReview {
        captures: captures.to_vec(),
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

    /// A real panel PASS line: the outer name is the live name (`script_*`), the inner
    /// evidence names the scenario, and the witness case is the snake_case wire form.
    fn panel_core_pass(live: &str, scenario: &str) -> String {
        let wire = wire_case(scenario, RunnerKind::Core).unwrap();
        format!(
            "PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\",\"scene\":2}}\n\
             CATALOG_CORE: {live} {{\"case\":\"{wire}\",\"post_start_observations\":2}}\n"
        )
    }

    #[test]
    fn wire_identities_come_from_the_host_enums() {
        assert_eq!(
            wire_case("thiever", RunnerKind::Core).unwrap(),
            "thiever",
            "CoreCase::Thiever serializes snake_case"
        );
        assert_eq!(
            wire_case("nature_crafter_air", RunnerKind::Pair).unwrap(),
            "air"
        );
        assert_eq!(
            wire_case("mule_crafter_air", RunnerKind::Pair).unwrap(),
            "mule"
        );
        assert!(wire_case("not_a_scenario", RunnerKind::Core).is_err());
        // The manifest's declared identities agree with the enum wire form.
        let manifest = manifest();
        for case in manifest.cases.iter().filter(|case| case.is_runnable()) {
            let scenario = case.scenario.as_deref().unwrap();
            assert_eq!(
                wire_case(scenario, case.runner()).unwrap(),
                match case.runner() {
                    RunnerKind::Core => case.core_case.clone().unwrap(),
                    RunnerKind::Pair => case.pair_case.clone().unwrap(),
                },
                "{scenario}"
            );
        }
    }

    #[test]
    fn parses_terminal_receipts_without_a_permissive_pass_regex() {
        let output = "\
live script_thiever: running step 1/2
PASS: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"PASS\"}
CATALOG_CORE: script_thiever {\"case\":\"thiever\",\"post_start_observations\":2}
[panel] shot thiever paint -> /tmp/shots/run/thiever_paint.png
";
        let receipts = parse(output);
        assert!(receipts.has_scenario_terminal());
        let pass = receipts.scenario_pass.as_ref().unwrap();
        assert_eq!(pass.name, "script_thiever");
        assert_eq!(evidence_scenario(pass), Some("thiever"));
        assert_eq!(receipts.core.as_ref().unwrap().name, "script_thiever");
        assert_eq!(receipts.shot_lines.len(), 1);
        assert!(receipts.duplicates.is_empty());

        // "PASS" as a bare word is not a receipt.
        assert!(!parse("PASS\n").has_scenario_terminal());
        assert!(!parse("all good, PASS(ed) the case\n").has_scenario_terminal());
    }

    #[test]
    fn the_outer_live_name_and_the_inner_scenario_are_checked_separately() {
        let case = core_case("thiever");
        // The old invented shape (outer name = scenario, PascalCase witness) must fail.
        let invented = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nCATALOG_CORE: thiever {\"case\":\"Thiever\"}\n",
        );
        match validate(&case, &invented, Some(0), &[]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("expected \"script_thiever\""), "{reason}");
            }
            other => panic!("{other:?}"),
        }

        // A PascalCase witness under the right outer name fails on the wire form.
        let pascal = parse(
            &panel_core_pass("script_thiever", "thiever")
                .replace("\"case\":\"thiever\"", "\"case\":\"Thiever\""),
        );
        match validate(&case, &pascal, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"thiever\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // Inner evidence naming another scenario fails too.
        let wrong_inner = parse(
            &panel_core_pass("script_thiever", "thiever")
                .replace("\"scenario\":\"thiever\"", "\"scenario\":\"alcher\""),
        );
        match validate(&case, &wrong_inner, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("evidence names scenario"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_real_panel_core_pass_is_valid_and_keeps_the_shots_contract() {
        let case = core_case("thiever");
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        // No capture attributed: thiever's scenario declares a terminal shot, so a PASS
        // without one cannot qualify.
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("thiever paint"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        let shot = capture_record();
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[shot]),
            Verdict::PendingVisualReview { .. }
        ));
    }

    #[test]
    fn shared_signals_are_detected_from_child_output() {
        let receipts = parse("FATAL: engine unavailable\n");
        assert_eq!(receipts.shared_signals, vec!["engine unavailable"]);
    }

    #[test]
    fn zero_exit_without_a_receipt_is_a_shared_receipt_failure() {
        let case = core_case("thiever");
        let receipts = parse("live script_thiever: running step 1/2\n");
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
        let receipts =
            parse("FAIL: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"FAIL\"}\n");
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure {
                kind: "receipt",
                ..
            }
        ));
    }

    #[test]
    fn a_dual_terminal_receipt_is_a_shared_failure() {
        let case = core_case("thiever");
        let receipts = parse(
            "PASS: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"PASS\"}\n\
             FAIL: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"FAIL\"}\n\
             CATALOG_CORE: script_thiever {\"case\":\"thiever\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[capture_record()]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("both a PASS and a FAIL"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn assertion_failure_is_case_local_and_keeps_the_message() {
        let case = core_case("thiever");
        let receipts = parse(
            "FAIL: live script_thiever {\"scenario\":\"thiever\",\"message\":\"no Coins gained\"}\n",
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
        let mut receipts = parse("PASS: live script_thiever {\"scenario\":\"thiever\"}\n");
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure {
                kind: "receipt",
                ..
            }
        ));

        receipts = parse(
            "PASS: live script_thiever {\"scenario\":\"thiever\"}\n\
             CATALOG_CORE: script_thiever {\"case\":\"bank_fletcher\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"thiever\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // A core case must not present a pair witness.
        receipts = parse(
            "PASS: live script_thiever {\"scenario\":\"thiever\"}\n\
             CATALOG_CORE: script_thiever {\"case\":\"thiever\"}\n\
             PAIRED_CORE: script_thiever {\"case\":\"air\"}\n",
        );
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure { .. }
        ));
    }

    #[test]
    fn duplicate_terminal_receipts_are_rejected() {
        let case = core_case("thiever");
        let line = "PASS: live script_thiever {\"scenario\":\"thiever\"}\n";
        let receipts = parse(&format!("{line}{line}"));
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("duplicate"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pair_cases_require_their_own_witness_under_the_live_name() {
        let case = core_case("nature_crafter_air");
        let receipts = parse(
            "PASS: live script_nature_crafter_air {\"scenario\":\"nature_crafter_air\"}\n\
             PAIRED_CORE: script_nature_crafter_air {\"case\":\"air\"}\n",
        );
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[pair_capture_record()]),
            Verdict::PendingVisualReview { .. }
        ));

        let wrong = parse(
            "PASS: live script_nature_crafter_air {\"scenario\":\"nature_crafter_air\"}\n\
             PAIRED_CORE: script_nature_crafter_air {\"case\":\"mule\"}\n",
        );
        match validate(&case, &wrong, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"air\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// A structurally complete capture record for `thiever`'s declared shot.
    fn capture_record() -> CaptureRecord {
        CaptureRecord {
            label: "2026-09-14T00-00-01_thiever_paint".into(),
            png: "/tmp/a.png".into(),
            json: "/tmp/a.json".into(),
            png_bytes: 128,
            png_magic: true,
            png_sha256: "digest".into(),
            decoded: true,
            png_width: 2,
            png_height: 2,
            sidecar_ok: true,
            sidecar_ingame: Some(true),
            sidecar_scene_state: Some(2),
            sidecar_actor: None,
            modified_ms: 1,
        }
    }

    fn pair_capture_record() -> CaptureRecord {
        CaptureRecord {
            label: "2026-09-14T00-00-01_nature_crafter_air-alice".into(),
            ..capture_record()
        }
    }

    #[test]
    fn a_capture_that_is_not_the_terminal_shot_evidence_cannot_pass() {
        let mut broken = capture_record();
        broken.decoded = false;
        broken.png_magic = false;
        broken.png_bytes = 4;
        let case = core_case("thiever");
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        match validate(&case, &receipts, Some(0), &[broken]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("not a PNG"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_capture_taken_outside_the_world_or_off_scene_two_is_not_evidence() {
        let case = core_case("thiever");
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        let off_world = {
            let mut record = capture_record();
            record.sidecar_ingame = Some(false);
            record
        };
        let off_scene = {
            let mut record = capture_record();
            record.sidecar_scene_state = Some(1);
            record
        };
        let no_scene = {
            let mut record = capture_record();
            record.sidecar_scene_state = None;
            record
        };
        let no_sidecar = {
            let mut record = capture_record();
            record.sidecar_ok = false;
            record
        };
        let empty_frame = {
            let mut record = capture_record();
            record.png_height = 0;
            record
        };
        for (name, needle, record) in [
            ("ingame=false", "not in game", off_world),
            ("scene_state=1", "scene_state 1", off_scene),
            ("scene_state absent", "no scene_state", no_scene),
            ("sidecar missing", "sidecar is missing", no_sidecar),
            ("empty frame", "empty frame", empty_frame),
        ] {
            match validate(&case, &receipts, Some(0), &[record]) {
                Verdict::SharedFailure { kind, reason } => {
                    assert_eq!(kind, "capture");
                    assert!(reason.contains(needle), "{name} not in: {reason}");
                }
                other => panic!("{name} -> {other:?}"),
            }
        }
    }

    #[test]
    fn captures_are_attributed_by_content_magic_and_sidecar_evidence() {
        let dir = std::env::temp_dir().join(format!("274bot-suite-caps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = dir.join("2026-09-14T00-00-00_1234");
        std::fs::create_dir_all(&run).unwrap();
        let png = run.join("2026-09-14T00-00-01_thiever_paint.png");
        std::fs::write(&png, tiny_png()).unwrap();
        std::fs::write(
            run.join("2026-09-14T00-00-01_thiever_paint.json"),
            "{\"ingame\":true,\"scene_state\":2}",
        )
        .unwrap();
        std::fs::write(run.join("2026-09-14T00-00-01_other.png"), tiny_png()).unwrap();

        let found = find_captures(&dir, "thiever paint");
        assert_eq!(found.len(), 1);
        assert!(found[0].complete(), "{:?}", found[0].structural_reason());
        assert_eq!(found[0].sidecar_ingame, Some(true));
        assert_eq!(found[0].png_width, 1);
        assert_eq!(found[0].png_sha256.len(), 64);
        assert!(find_captures(&dir, "absent label").is_empty());

        // A torn PNG beside a valid sidecar is not a capture.
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n truncated").unwrap();
        let broken = find_captures(&dir, "thiever paint");
        assert_eq!(broken.len(), 1);
        assert!(!broken[0].complete());
        assert!(
            broken[0]
                .structural_reason()
                .unwrap()
                .contains("did not decode"),
            "{:?}",
            broken[0].structural_reason()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_captures_written_during_the_case_are_attributed() {
        use std::collections::BTreeMap;
        let mut before = BTreeMap::new();
        let mut earlier = capture_record();
        earlier.label = "2026-09-14T00-00-01_thiever_paint".to_string();
        before.insert(earlier.label.clone(), earlier.clone());
        let mut after = before.clone();
        let mut later = capture_record();
        later.label = "2026-09-14T00-01-00_thiever_paint".to_string();
        after.insert(later.label.clone(), later.clone());
        let new = new_captures(&before, &after);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].label, "2026-09-14T00-01-00_thiever_paint");
        assert!(
            new_captures(&after, &after).is_empty(),
            "a same-named capture from an earlier case is never reused"
        );
    }

    #[test]
    fn an_optional_capture_that_was_not_requested_leaves_the_receipt_passed() {
        let mut case = core_case("thiever");
        case.capture = Some(CaptureSpec {
            label: "thiever paint".into(),
            required: false,
        });
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        assert_eq!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::Passed,
            "a declared-but-optional capture does not turn a receipt into a failure"
        );
    }

    /// A real 1x1 RGBA PNG.
    fn tiny_png() -> Vec<u8> {
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }
}
