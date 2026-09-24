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

use super::identity::ExternalSource;
use super::manifest::{CaptureSpec, CaseEntry, RunnerKind};
use super::SuiteResult;
mod external;
use external::validate_external;

pub const PASS_PREFIX: &str = "PASS: live ";
pub const FAIL_PREFIX: &str = "FAIL: live ";
pub const CORE_PREFIX: &str = "CATALOG_CORE: ";
pub const PAIR_PREFIX: &str = "PAIRED_CORE: ";
/// The dedicated external loader smoke's own witness line (`host_play::external_loader::RECEIPT_PREFIX`).
pub const EXTERNAL_PREFIX: &str = "EXTERNAL_LOADER: ";
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
        // The loader smoke has no host enum at all: deriving an identity for it would invent one.
        RunnerKind::External => {
            return Err(format!(
                "{scenario}: the external loader smoke has no catalog/pair witness identity"
            ))
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
///
/// The external loader smoke has no scenario to ask: its case declares the producer's own
/// terminal-shot label, and a row that declares anything else (or nothing) falls back to the
/// mandatory producer label rather than relaxing the contract.
pub fn declared_capture(case: &CaseEntry) -> Option<CaptureSpec> {
    if case.runner() == RunnerKind::External {
        return Some(match &case.capture {
            Some(capture) if capture.label == host_play::external_loader::TERMINAL_SHOT => {
                capture.clone()
            }
            _ => CaptureSpec {
                label: host_play::external_loader::TERMINAL_SHOT.to_string(),
                required: true,
            },
        });
    }
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
    /// The external loader smoke's own witness (`EXTERNAL_LOADER:`), a distinct contract from
    /// the scenario/CoreCase receipts above.
    pub external: Option<TerminalReceipt>,
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
        } else if line.starts_with(EXTERNAL_PREFIX) {
            Slot::External
        } else {
            continue;
        };
        let rest = match slot {
            Slot::ScenarioPass => &line[PASS_PREFIX.len()..],
            Slot::ScenarioFail => &line[FAIL_PREFIX.len()..],
            Slot::Core => &line[CORE_PREFIX.len()..],
            Slot::Pair => &line[PAIR_PREFIX.len()..],
            Slot::External => &line[EXTERNAL_PREFIX.len()..],
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
            Slot::External => &mut receipts.external,
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
    External,
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
/// paths never reach validation. `captures` are the captures attributed to *this* case, and
/// `external` is the bound external loader source — `Some` only for the loader smoke, which has
/// its own receipt contract and never a `CoreCase`/`PairCase` witness.
pub fn validate(
    case: &CaseEntry,
    receipts: &ParsedReceipts,
    exit_code: Option<i32>,
    captures: &[CaptureRecord],
    external: Option<&ExternalSource>,
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
    if case.runner() == RunnerKind::External {
        // A different contract: the loader smoke prints its own record, not a scenario receipt.
        return validate_external(case, receipts, exit_code, captures, external);
    }
    if receipts.external.is_some() {
        return shared(
            "receipt",
            format!(
                "{live} printed an EXTERNAL_LOADER witness; only the dedicated external loader \
                 smoke prints one"
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
        RunnerKind::Pair => case.pair_case.as_deref().unwrap_or_default(),
        _ => case.core_case.as_deref().unwrap_or_default(),
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
            if expected_case == "duel" {
                if let Some(reason) = duel_witness_mismatch(pair) {
                    return shared("receipt", reason);
                }
            }
            if receipts.core.is_some() {
                return shared(
                    "receipt",
                    format!("{live} is a pair case but printed a CATALOG_CORE witness"),
                );
            }
        }
        // The external loader smoke returns above; reaching here means a case was presented as
        // an external row and as a catalog case at once, which is refused rather than guessed.
        RunnerKind::External => {
            return shared(
                "receipt",
                format!(
                    "{live} is an external loader case but reached the catalog witness contract"
                ),
            )
        }
    }

    let verdict = capture_verdict(case, captures);
    if case.pair_case.as_deref() == Some("duel") {
        if let Some(pair) = &receipts.pair {
            return duel_pair_captures(case, pair, captures, verdict);
        }
    }
    verdict
}

/// The `scenario` field of a receipt payload, when it is a string.
fn evidence_scenario(receipt: &TerminalReceipt) -> Option<&str> {
    match &receipt.payload {
        Some(Value::Object(map)) => map.get("scenario").and_then(Value::as_str),
        _ => None,
    }
}

fn failure_detail(fail: &TerminalReceipt, receipts: &ParsedReceipts) -> String {
    if let Some(detail) = receipts.pair.as_ref().and_then(pair_qualification_detail) {
        return detail;
    }
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

/// Actual pair qualification/error from the PAIRED_CORE witness. The FAIL
/// live line keeps the scenario JSON contract, which can say outcome PASS
/// for a passed prerequisite while the pair itself failed.
fn pair_qualification_detail(pair: &TerminalReceipt) -> Option<String> {
    let Value::Object(map) = pair.payload.as_ref()? else {
        return None;
    };
    if let Some(error) = map.get("error").and_then(Value::as_str) {
        if !error.is_empty() {
            return Some(error.to_string());
        }
    }
    if let Some(qualification) = map.get("qualification").and_then(Value::as_str) {
        if !qualification.is_empty() {
            return Some(qualification.to_string());
        }
    }
    None
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

/// Duel PASS must be a real native witness: decode `DuelPairWitness` with the
/// fields those types require, then call `qualify_supported` /
/// `qualify_full_cycle`. Missing or type-invalid safety fields fail closed.
/// FirstCombat does not require reset/further combat.
fn duel_witness_mismatch(pair: &TerminalReceipt) -> Option<String> {
    use host_play::paired_core::{DuelClaim, DuelPairWitness};

    let Some(Value::Object(map)) = &pair.payload else {
        return Some("PAIRED_CORE Duel witness carries no JSON object".into());
    };
    let body = map.get("witness").and_then(Value::as_object).unwrap_or(map);
    if body.get("a").is_none() {
        return Some("PAIRED_CORE Duel witness is missing actor a".into());
    }
    if body.get("b").is_none() {
        return Some("PAIRED_CORE Duel witness is missing actor b".into());
    }
    let witness: DuelPairWitness = match serde_json::from_value(serde_json::json!({
        "a": body.get("a"),
        "b": body.get("b"),
    })) {
        Ok(witness) => witness,
        Err(err) => {
            return Some(format!(
                "PAIRED_CORE Duel witness is not a native DuelPairWitness: {err}"
            ))
        }
    };
    let claim = body.get("claim").and_then(Value::as_str);
    let supported = body.get("supported").and_then(Value::as_str);
    let full = body.get("full").and_then(Value::as_str);
    match claim {
        Some("first-combat") => {
            if supported != Some("first-combat") || full.is_some() {
                return Some(
                    "PAIRED_CORE Duel first-combat claim is not backed by supported/full fields"
                        .into(),
                );
            }
            match witness.qualify_supported() {
                Ok(DuelClaim::FirstCombat) => None,
                Ok(other) => Some(format!(
                    "PAIRED_CORE Duel first-combat claim is not backed by native qualification ({other:?})"
                )),
                Err(reason) => Some(format!(
                    "PAIRED_CORE Duel first-combat claim is not backed by native qualification: {reason}"
                )),
            }
        }
        Some("reset-and-further") => {
            if full != Some("reset-and-further") || supported != Some("first-combat") {
                return Some(
                    "PAIRED_CORE Duel reset-and-further claim is not backed by supported/full fields"
                        .into(),
                );
            }
            match witness.qualify_full_cycle() {
                Ok(DuelClaim::ResetAndFurther) => None,
                Ok(other) => Some(format!(
                    "PAIRED_CORE Duel reset-and-further claim is not backed by native qualification ({other:?})"
                )),
                Err(reason) => Some(format!(
                    "PAIRED_CORE Duel reset-and-further claim is not backed by reset and further combat: {reason}"
                )),
            }
        }
        Some(other) => Some(format!(
            "PAIRED_CORE Duel claim {other:?} is not first-combat or reset-and-further"
        )),
        None => Some(
            "PAIRED_CORE Duel witness carries no first-combat or reset-and-further claim".into(),
        ),
    }
}

fn slot_actor<'a>(slot: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    slot.get(key).and_then(Value::as_str)
}

fn actor_eq(left: &str, right: &str) -> bool {
    left == right
        || client::util::JString::to_screen_name(left)
            == client::util::JString::to_screen_name(right)
}

/// Duel PASS keeps both owned-actor terminal captures. One matching label, a
/// missing sidecar actor, or a capture bound to the wrong actor is not enough.
fn duel_pair_captures(
    case: &CaseEntry,
    pair: &TerminalReceipt,
    _captures: &[CaptureRecord],
    verdict: Verdict,
) -> Verdict {
    let Verdict::PendingVisualReview { captures: kept } = verdict else {
        return verdict;
    };
    let Some(Value::Object(map)) = &pair.payload else {
        return Verdict::SharedFailure {
            kind: "receipt",
            reason: "PAIRED_CORE Duel witness carries no JSON object".into(),
        };
    };
    let body = map.get("witness").and_then(Value::as_object).unwrap_or(map);
    let Some(a) = body
        .get("a")
        .and_then(Value::as_object)
        .and_then(|slot| slot_actor(slot, "account"))
    else {
        return Verdict::SharedFailure {
            kind: "receipt",
            reason: "PAIRED_CORE Duel witness is missing actor a".into(),
        };
    };
    let Some(b) = body
        .get("b")
        .and_then(Value::as_object)
        .and_then(|slot| slot_actor(slot, "account"))
    else {
        return Verdict::SharedFailure {
            kind: "receipt",
            reason: "PAIRED_CORE Duel witness is missing actor b".into(),
        };
    };
    let Some(spec) = declared_capture(case) else {
        return Verdict::PendingVisualReview { captures: kept };
    };
    let matching: Vec<&CaptureRecord> = kept
        .iter()
        .filter(|record| record.matches_label(&spec.label))
        .collect();
    if matching.len() < 2 {
        return Verdict::SharedFailure {
            kind: "capture",
            reason: format!(
                "Duel terminal capture for both actors was not written (found {})",
                matching.len()
            ),
        };
    }
    for actor in [a, b] {
        match matching.iter().find(|record| {
            record
                .sidecar_actor
                .as_deref()
                .is_some_and(|name| actor_eq(name, actor))
        }) {
            Some(_) => {}
            None => {
                let wrong = matching
                    .iter()
                    .find_map(|record| record.sidecar_actor.as_deref());
                let reason = match wrong {
                    None => format!("Duel terminal capture for {actor:?} carries no actor binding"),
                    Some(other) if !actor_eq(other, actor) => {
                        format!("Duel terminal capture names actor {other:?}, expected {actor:?}")
                    }
                    Some(_) => format!("missing Duel terminal capture for actor {actor:?}"),
                };
                return Verdict::SharedFailure {
                    kind: "capture",
                    reason,
                };
            }
        }
    }
    Verdict::PendingVisualReview { captures: kept }
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


/// The exit code as a receipt line prints it.
fn exit_text(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "n/a".into())
}

/// The serde wire form of a producer enum value. The receipt carries the real enum, so the
/// expected token is derived from that enum instead of hard-coding a spelling that could drift.
fn wire<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn require(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

/// A producer hash field: a lowercase SHA-256 hex digest (`{:x}` of 32 bytes). An arbitrary or
/// mis-cased string is not a hash, and a receipt that carries one is refused.
fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}



#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
