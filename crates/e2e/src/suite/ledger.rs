//! The run ledger: durable, append-only-in-spirit attempt records under a run directory.
//!
//! Contract:
//!   * a fresh run creates its directory once and refuses to overwrite an occupied one;
//!   * an attempt is written *before* the child is launched, so an interrupted or
//!     crashed run leaves the case recorded as attempted;
//!   * resume refuses a changed identity or selection *before* any spawn, skips every
//!     case that already has an attempt (PASS included) and never retries a failed,
//!     interrupted or still-running attempt;
//!   * logs and capture metadata stay bounded; credentials never enter the ledger.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::identity::RunIdentity;
use super::receipt::CaptureRecord;
use super::SuiteResult;

pub const STATE_FILE: &str = "state.json";
pub const LOGS_DIR: &str = "logs";
pub const SHOTS_DIR: &str = "shots";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    /// Written before the launch; a run that dies here leaves it for a human.
    Running,
    Passed,
    PendingVisualReview,
    Failed,
    Timeout,
    Interrupted,
    /// Selected but not executable (no native adapter, excluded, operator decision,
    /// unverified fixture): recorded with its reason, nothing is substituted.
    Unavailable,
    /// Missing/malformed/conflicting terminal receipt, infrastructure signal or a
    /// requested capture that did not arrive: the shared harness is unhealthy.
    SharedFailure,
    /// A launched process tree that could not be reaped inside the bounded cleanup.
    CleanupFailed,
}

impl AttemptStatus {
    pub fn is_success(self) -> bool {
        matches!(
            self,
            AttemptStatus::Passed | AttemptStatus::PendingVisualReview
        )
    }

    /// Statuses that make the run unsuccessful (a case was attempted and did not pass,
    /// or the shared harness failed).
    pub fn is_unsuccessful(self) -> bool {
        matches!(
            self,
            AttemptStatus::Failed
                | AttemptStatus::Timeout
                | AttemptStatus::Interrupted
                | AttemptStatus::SharedFailure
                | AttemptStatus::CleanupFailed
                | AttemptStatus::Running
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AttemptStatus::Running => "running",
            AttemptStatus::Passed => "passed",
            AttemptStatus::PendingVisualReview => "pending_visual_review",
            AttemptStatus::Failed => "failed",
            AttemptStatus::Timeout => "timeout",
            AttemptStatus::Interrupted => "interrupted",
            AttemptStatus::Unavailable => "unavailable",
            AttemptStatus::SharedFailure => "shared_failure",
            AttemptStatus::CleanupFailed => "cleanup_failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub case_id: String,
    pub attempt: u32,
    pub status: AttemptStatus,
    pub reason: String,
    /// What the suite asked this case to do (`core:script_thiever:budget=720s`).
    pub requested_operation: String,
    /// What actually happened (`core:script_thiever:exit=0:receipts=scenario+catalog_core`).
    pub completed_operation: Option<String>,
    #[serde(default)]
    pub command: Vec<String>,
    /// Environment variable *names* handed to the child; never values that could carry
    /// credentials.
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub signal: Option<i32>,
    #[serde(default)]
    pub log: Option<String>,
    #[serde(default)]
    pub log_truncated: bool,
    #[serde(default)]
    pub receipts: Option<ReceiptSummary>,
    #[serde(default)]
    pub captures: Vec<CaptureRecord>,
    #[serde(default)]
    pub case_identity: Value,
    pub started_at: u64,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub cleanup: Option<CleanupSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReceiptSummary {
    pub scenario: Option<String>,
    pub catalog_core: Option<String>,
    pub paired_core: Option<String>,
    pub shot_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupSummary {
    pub killed_signal: Option<i32>,
    pub escalated_to_sigkill: bool,
    pub reaped: bool,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub selected: usize,
    pub attempted: usize,
    pub passed: usize,
    pub pending_visual_review: usize,
    pub failed: usize,
    pub timeout: usize,
    pub interrupted: usize,
    pub unavailable: usize,
    pub shared_failure: usize,
    /// Selected cases an earlier run already attempted: resume skips them untouched.
    pub skipped_existing: usize,
    /// Of the skipped ones, how many had ended successfully.
    pub carried_success: usize,
    /// Of the skipped ones, how many are failed/interrupted/still-running: retained,
    /// never retried automatically.
    pub carried_unsuccessful: usize,
    /// Selected cases this run never reached (the run stopped first).
    pub not_reached: usize,
    /// Attempts still marked `running`: a previous process died mid-launch.
    pub stuck_running: usize,
    pub stop_reason: Option<String>,
    pub failures: Vec<FailureRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRow {
    pub case_id: String,
    pub status: AttemptStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerState {
    pub schema_version: u32,
    pub suite_id: String,
    pub identity: Value,
    pub identity_hash: String,
    pub selection: Vec<String>,
    #[serde(default)]
    pub attempts: BTreeMap<String, Attempt>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub summary: Option<Summary>,
    pub started_at: u64,
    #[serde(default)]
    pub finished_at: Option<u64>,
}

/// A run directory plus its state file.
#[derive(Debug)]
pub struct Ledger {
    pub dir: PathBuf,
    pub state: LedgerState,
    /// Case ids this process wrote an attempt for (attempted, or recorded unavailable).
    /// Not persisted: it separates "attempted now" from "carried in by resume".
    attempted_now: std::collections::BTreeSet<String>,
}

/// The terminal record for one attempt, kept in one struct so the ledger write stays
/// readable at the call site.
#[derive(Debug, Clone)]
pub struct AttemptEnd {
    pub status: AttemptStatus,
    pub reason: String,
    pub completed_operation: Option<String>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub receipts: Option<ReceiptSummary>,
    pub captures: Vec<CaptureRecord>,
    pub elapsed_ms: u64,
    pub cleanup: Option<CleanupSummary>,
    pub log_truncated: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl Ledger {
    /// Create a fresh run directory. Refuses to reuse an occupied one: a run directory is
    /// a durable record, not scratch space.
    pub fn create(
        dir: &Path,
        suite_id: &str,
        identity: &RunIdentity,
        selection: &[String],
    ) -> SuiteResult<Self> {
        if dir.join(STATE_FILE).exists() {
            return Err(format!(
                "refusing to overwrite the occupied run directory {} (state.json exists); use --resume",
                dir.display()
            ));
        }
        if dir.exists() {
            let mut entries = std::fs::read_dir(dir)
                .map_err(|error| format!("run dir {}: {error}", dir.display()))?;
            if entries.next().is_some() {
                return Err(format!(
                    "refusing to reuse the non-empty run directory {}; use --resume",
                    dir.display()
                ));
            }
        }
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("run dir {}: {error}", dir.display()))?;
        std::fs::create_dir_all(dir.join(LOGS_DIR))
            .map_err(|error| format!("run dir logs {}: {error}", dir.display()))?;
        std::fs::create_dir_all(dir.join(SHOTS_DIR))
            .map_err(|error| format!("run dir shots {}: {error}", dir.display()))?;
        let ledger = Ledger {
            dir: dir.to_path_buf(),
            state: LedgerState {
                schema_version: SCHEMA_VERSION,
                suite_id: suite_id.to_string(),
                identity: identity.value(),
                identity_hash: identity.hash(),
                selection: selection.to_vec(),
                attempts: BTreeMap::new(),
                stop_reason: None,
                summary: None,
                started_at: now(),
                finished_at: None,
            },
            attempted_now: std::collections::BTreeSet::new(),
        };
        ledger.write()?;
        Ok(ledger)
    }

    /// Open an existing run directory for resume.
    pub fn resume(dir: &Path) -> SuiteResult<Self> {
        let path = dir.join(STATE_FILE);
        let bytes = std::fs::read(&path).map_err(|error| {
            format!("cannot resume: {} is unreadable ({error})", path.display())
        })?;
        let state: LedgerState = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "cannot resume: {} is not a valid ledger ({error})",
                path.display()
            )
        })?;
        if state.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "cannot resume: ledger schema_version {} is not the supported {SCHEMA_VERSION}",
                state.schema_version
            ));
        }
        Ok(Ledger {
            dir: dir.to_path_buf(),
            state,
            attempted_now: std::collections::BTreeSet::new(),
        })
    }

    /// Resume gate: identity and ordered selection must both be unchanged, and the
    /// recorded identity must be content-bound. Called before any child is launched.
    pub fn check_resume(&self, identity: &RunIdentity, selection: &[String]) -> SuiteResult<()> {
        let stored: RunIdentity = serde_json::from_value(self.state.identity.clone())
            .map_err(|error| format!("cannot resume: stored identity is unreadable ({error})"))?;
        for (label, unresolved) in [
            ("recorded", stored.unresolved()),
            ("current", identity.unresolved()),
        ] {
            if !unresolved.is_empty() {
                return Err(format!(
                    "refusing resume: the {label} identity has unresolved components ({}); a run \
                     that could not bind an input cannot prove the input is unchanged",
                    unresolved.join(", ")
                ));
            }
        }
        let differences = stored.differences(identity);
        if !differences.is_empty() {
            return Err(format!(
                "refusing resume: {} changed since this run started",
                differences.join(", ")
            ));
        }
        if self.state.selection != selection {
            return Err(format!(
                "refusing resume: selection changed ({} recorded, {} selected)",
                self.state.selection.len(),
                selection.len()
            ));
        }
        Ok(())
    }

    pub fn attempt(&self, case_id: &str) -> Option<&Attempt> {
        self.state.attempts.get(case_id)
    }

    /// Record a case the suite will not launch (unavailable / no adapter). It keeps its
    /// explicit reason so a gap is visible rather than silently skipped.
    pub fn record_unavailable(&mut self, case_id: &str, reason: &str) -> SuiteResult<()> {
        self.state.attempts.insert(
            case_id.to_string(),
            Attempt {
                case_id: case_id.to_string(),
                attempt: 1,
                status: AttemptStatus::Unavailable,
                reason: reason.to_string(),
                requested_operation: format!("unavailable:{case_id}"),
                completed_operation: None,
                command: Vec::new(),
                env_keys: Vec::new(),
                exit_code: None,
                signal: None,
                log: None,
                log_truncated: false,
                receipts: None,
                captures: Vec::new(),
                case_identity: Value::Null,
                started_at: now(),
                elapsed_ms: 0,
                cleanup: None,
            },
        );
        self.attempted_now.insert(case_id.to_string());
        self.write()
    }

    /// Start an attempt: written before the launch, so a crash or interrupt still leaves
    /// evidence that this case was attempted.
    #[allow(clippy::too_many_arguments)]
    pub fn start_attempt(
        &mut self,
        case_id: &str,
        requested_operation: String,
        command: &[String],
        env_keys: &[String],
        case_identity: Value,
        log: Option<String>,
    ) -> SuiteResult<()> {
        let previous = self
            .state
            .attempts
            .get(case_id)
            .map(|a| a.attempt)
            .unwrap_or(0);
        self.state.attempts.insert(
            case_id.to_string(),
            Attempt {
                case_id: case_id.to_string(),
                attempt: previous + 1,
                status: AttemptStatus::Running,
                reason: String::new(),
                requested_operation,
                completed_operation: None,
                command: command.to_vec(),
                env_keys: env_keys.to_vec(),
                exit_code: None,
                signal: None,
                log,
                log_truncated: false,
                receipts: None,
                captures: Vec::new(),
                case_identity,
                started_at: now(),
                elapsed_ms: 0,
                cleanup: None,
            },
        );
        self.attempted_now.insert(case_id.to_string());
        self.write()
    }

    pub fn finish_attempt(&mut self, case_id: &str, end: AttemptEnd) -> SuiteResult<()> {
        let Some(attempt) = self.state.attempts.get_mut(case_id) else {
            return Err(format!("no attempt recorded for {case_id}"));
        };
        attempt.status = end.status;
        attempt.reason = end.reason;
        attempt.completed_operation = end.completed_operation;
        attempt.exit_code = end.exit_code;
        attempt.signal = end.signal;
        attempt.receipts = end.receipts;
        attempt.captures = end.captures;
        attempt.elapsed_ms = end.elapsed_ms;
        attempt.cleanup = end.cleanup;
        attempt.log_truncated = end.log_truncated;
        self.write()
    }

    pub fn set_stop_reason(&mut self, reason: Option<String>) -> SuiteResult<()> {
        self.state.stop_reason = reason;
        self.write()
    }

    pub fn summarize(&mut self) -> SuiteResult<Summary> {
        let mut summary = Summary {
            selected: self.state.selection.len(),
            ..Summary::default()
        };
        for id in &self.state.selection {
            match self.state.attempts.get(id) {
                // Selected but never reached: the run stopped (or ended) before it.
                None => summary.not_reached += 1,
                Some(attempt) if !self.attempted_now.contains(id) => {
                    // Carried in by resume: kept exactly as recorded, never re-launched.
                    summary.skipped_existing += 1;
                    match attempt.status {
                        AttemptStatus::Unavailable => summary.unavailable += 1,
                        status if status.is_success() => summary.carried_success += 1,
                        _ => summary.carried_unsuccessful += 1,
                    }
                }
                Some(attempt) => match attempt.status {
                    AttemptStatus::Unavailable => summary.unavailable += 1,
                    AttemptStatus::Running => summary.stuck_running += 1,
                    status => {
                        summary.attempted += 1;
                        match status {
                            AttemptStatus::Passed => summary.passed += 1,
                            AttemptStatus::PendingVisualReview => {
                                summary.pending_visual_review += 1
                            }
                            AttemptStatus::Failed => summary.failed += 1,
                            AttemptStatus::Timeout => summary.timeout += 1,
                            AttemptStatus::Interrupted => summary.interrupted += 1,
                            AttemptStatus::SharedFailure | AttemptStatus::CleanupFailed => {
                                summary.shared_failure += 1
                            }
                            _ => {}
                        }
                    }
                },
            }
        }
        summary.failures = self
            .state
            .attempts
            .values()
            .filter(|attempt| attempt.status.is_unsuccessful())
            .map(|attempt| FailureRow {
                case_id: attempt.case_id.clone(),
                status: attempt.status,
                reason: attempt.reason.clone(),
            })
            .collect();
        summary.stop_reason = self.state.stop_reason.clone();
        self.state.summary = Some(summary.clone());
        self.state.finished_at = Some(now());
        self.write()?;
        Ok(summary)
    }

    /// Write the ledger atomically: a crash mid-write must not corrupt the record.
    pub fn write(&self) -> SuiteResult<()> {
        let path = self.dir.join(STATE_FILE);
        let tmp = self.dir.join(format!(".{STATE_FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(&self.state)
            .map_err(|error| format!("ledger serialize: {error}"))?;
        std::fs::write(&tmp, &bytes)
            .map_err(|error| format!("ledger write {}: {error}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|error| format!("ledger rename {}: {error}", path.display()))?;
        Ok(())
    }

    pub fn log_path(&self, case_id: &str) -> PathBuf {
        self.dir
            .join(LOGS_DIR)
            .join(format!("{}.log", safe_file_name(case_id)))
    }

    pub fn shots_root(&self) -> PathBuf {
        self.dir.join(SHOTS_DIR)
    }
}

/// Case ids come from the manifest, but a run directory is still a filesystem boundary:
/// keep the file name inside it.
pub fn safe_file_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    out.truncate(80);
    if out.is_empty() {
        out.push_str("case");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::identity::RunIdentity;
    use serde_json::json;

    fn identity(selection: &[&str]) -> RunIdentity {
        serde_json::from_value(json!({
            "manifest_sha256": "a",
            "suite_id": "s",
            "reference_commit": "c",
            "reference_tree": "t",
            "reference_archive_sha256": "z",
            "host": {"available": true, "commit": "h", "branch": "b", "dirty": false, "content_sha256": "hc", "note": null},
            "client": {"available": true, "commit": "k", "branch": null, "dirty": false, "content_sha256": "kc", "note": null},
            "binaries": {},
            "profile": {
                "profile": "local-274", "revision": null, "host": null, "port": null,
                "selection": "local-274",
                "resolved": {"selection": "local-274", "cache": "/cache", "vault": "/vault", "nav_pack": "/nav", "nav_flags": "/navflags", "content": "/content", "unpack": "/unpack"},
                "engine": null,
                "cache": {"target": "/cache", "sha256": "cc", "bytes": 2, "files": 1, "note": null},
                "catalog": {"target": "/catalog", "sha256": "cat", "bytes": 2, "files": 1, "note": null},
                "vault": {"target": "/vault", "sha256": "vv", "bytes": 2, "files": 1, "note": null},
                "nav_pack": {"target": "/nav", "sha256": "np", "bytes": 1, "files": 1, "note": null},
                "nav_flags": {"target": "/navflags", "sha256": "nf", "bytes": 1, "files": 1, "note": null},
                "content": {"target": "/content", "sha256": "cn", "bytes": 1, "files": 1, "note": null},
                "unpack": {"target": "/unpack", "sha256": "un", "bytes": 1, "files": 1, "note": null},
                "lowmem": true, "mainland": false, "jobs": 1
            },
            "settings": {
                "level": "quick", "only": [], "changed_paths": [], "changed_source": "supplied",
                "child_args": [], "child_env_keys": []
            },
            "selection": selection
        }))
        .unwrap()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "274bot-ledger-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn fresh_run_refuses_an_occupied_run_directory() {
        let dir = temp_dir("occupied");
        let selection = vec!["thiever".to_string()];
        Ledger::create(&dir, "suite", &identity(&["thiever"]), &selection).unwrap();
        let error = Ledger::create(&dir, "suite", &identity(&["thiever"]), &selection).unwrap_err();
        assert!(error.contains("refusing to overwrite"), "{error}");

        // A directory with unrelated content is occupied too.
        let other = temp_dir("content");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("notes.txt"), "x").unwrap();
        let error =
            Ledger::create(&other, "suite", &identity(&["thiever"]), &selection).unwrap_err();
        assert!(error.contains("non-empty run directory"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn resume_refuses_a_changed_identity_or_selection_before_any_launch() {
        let dir = temp_dir("resume");
        let selection = vec!["thiever".to_string()];
        Ledger::create(&dir, "suite", &identity(&["thiever"]), &selection).unwrap();

        let ledger = Ledger::resume(&dir).unwrap();
        ledger
            .check_resume(&identity(&["thiever"]), &selection)
            .unwrap();

        let mut changed = identity(&["thiever"]);
        changed.manifest_sha256 = "other".into();
        let error = ledger.check_resume(&changed, &selection).unwrap_err();
        assert!(error.contains("manifest_sha256"), "{error}");

        let error = ledger
            .check_resume(
                &identity(&["thiever"]),
                &["thiever".into(), "ardy_fighter".into()],
            )
            .unwrap_err();
        assert!(error.contains("selection changed"), "{error}");

        let error = Ledger::resume(&temp_dir("absent")).unwrap_err();
        assert!(error.contains("cannot resume"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn attempts_are_durable_before_launch_and_resume_skips_them() {
        let dir = temp_dir("attempts");
        let selection = vec!["thiever".to_string(), "ardy_fighter".to_string()];
        let mut ledger =
            Ledger::create(&dir, "suite", &identity(&["thiever"]), &selection).unwrap();
        ledger
            .start_attempt(
                "thiever",
                "core:script_thiever".into(),
                &["cargo".into(), "run".into()],
                &["BOT_DEADLINE".into()],
                json!({"scenario": "thiever"}),
                Some("logs/thiever.log".into()),
            )
            .unwrap();

        // A crash right here leaves the attempt on disk as `running`.
        let reread = Ledger::resume(&dir).unwrap();
        let attempt = reread
            .attempt("thiever")
            .expect("attempt persisted before launch");
        assert_eq!(attempt.status, AttemptStatus::Running);
        assert_eq!(attempt.requested_operation, "core:script_thiever");
        assert_eq!(attempt.env_keys, vec!["BOT_DEADLINE".to_string()]);

        ledger
            .finish_attempt(
                "thiever",
                AttemptEnd {
                    status: AttemptStatus::Failed,
                    reason: "case assertion failure".into(),
                    completed_operation: Some("core:script_thiever:exit=1".into()),
                    exit_code: Some(1),
                    signal: None,
                    receipts: None,
                    captures: Vec::new(),
                    elapsed_ms: 10,
                    cleanup: None,
                    log_truncated: false,
                },
            )
            .unwrap();
        ledger
            .record_unavailable("ardy_fighter", "fixture_prerequisite: no verified stand")
            .unwrap();

        let summary = ledger.summarize().unwrap();
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.unavailable, 1);
        assert_eq!(summary.skipped_existing, 0);
        assert_eq!(summary.failures.len(), 1);

        // A later resume sees both attempts and would skip them (no silent retry).
        let reread = Ledger::resume(&dir).unwrap();
        assert!(reread.attempt("thiever").is_some());
        assert!(reread.attempt("ardy_fighter").is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ledger_files_stay_inside_the_run_directory_and_log_names_are_safe() {
        assert_eq!(safe_file_name("thiever"), "thiever");
        assert_eq!(safe_file_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(safe_file_name(""), "case");
        let dir = temp_dir("paths");
        let ledger = Ledger::create(
            &dir,
            "suite",
            &identity(&["thiever"]),
            &["thiever".to_string()],
        )
        .unwrap();
        assert!(ledger.log_path("thiever").starts_with(&dir));
        assert!(ledger.shots_root().starts_with(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
