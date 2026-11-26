//! Test-only orchestration for a file scenario. The slot still owns its VM.

mod fixtures;
mod plan;
mod role;

pub(crate) use plan::in_walk_ground;
pub use plan::{FileOptions, FilePlan};
pub(crate) use role::{OwnerInputs, Role, WalkObservation, WalkSend, WalkTrace};

use std::{
    fs,
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

pub use scenario::RunnerStatus;
use serde_json::{json, Value};

use crate::Play;

#[derive(Clone, Copy)]
pub(crate) struct FileDispatch<'a> {
    pub(crate) run: &'a FileRun,
    pub(crate) role: &'a Role,
    pub(crate) source: Option<(u64, u64, u64)>,
}

/// Basename of the atomic File runner receipt in the selected evidence directory.
pub const FILE_RECEIPT_NAME: &str = "scenario-file-receipt.json";

const MAX_SIGNALS: usize = 4096;
static NEXT_PLAN: AtomicU64 = AtomicU64::new(1);
const RETIREMENT_OBSERVE_CAP: Duration = Duration::from_secs(12);

#[derive(Clone)]
pub struct FileRun(Arc<Run>);

struct Run {
    id: u64,
    plan: Box<plan::Plan>,
    deadline: Instant,
    roles: Vec<Role>,
    fixtures: OnceLock<fixtures::FixtureDefinitions>,
    failure: OnceLock<String>,
    fence_attempt: OnceLock<Instant>,
    retirement_started: OnceLock<Instant>,
    signals: parking_lot::Mutex<std::collections::BTreeMap<String, u16>>,
    receipt: OnceLock<Value>,
}

impl FileRun {
    pub fn new(plan: FilePlan, accounts: &[String]) -> Result<Self, String> {
        if accounts.len() != plan.role_count() {
            return Err("scenario account roster does not match the role roster".into());
        }
        let mut unique = std::collections::BTreeSet::new();
        if accounts
            .iter()
            .any(|account| account.is_empty() || !unique.insert(account.to_ascii_lowercase()))
        {
            return Err("scenario requires a distinct account for each role".into());
        }
        let id = NEXT_PLAN
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "scenario plan identities exhausted")?;
        let deadline = Instant::now()
            .checked_add(plan.deadline())
            .ok_or("scenario deadline overflow")?;
        let plan = plan.0;
        let roles = plan
            .scenario
            .roles
            .iter()
            .zip(accounts)
            .enumerate()
            .map(|(index, ((name, options), account))| {
                Role::new(index as u64 + 1, name, account, options.clone())
            })
            .collect();
        Ok(Self(Arc::new(Run {
            id,
            plan,
            deadline,
            roles,
            fixtures: OnceLock::new(),
            failure: OnceLock::new(),
            signals: parking_lot::Mutex::default(),
            fence_attempt: OnceLock::new(),
            retirement_started: OnceLock::new(),
            receipt: OnceLock::new(),
        })))
    }

    pub fn status(&self) -> RunnerStatus {
        let run = &self.0;
        run.advance();
        if let Some(receipt) = run.receipt.get() {
            if receipt["outcome"] == "PASS" {
                return RunnerStatus::Passed;
            }
            return RunnerStatus::Failed(
                receipt["failure"]
                    .as_str()
                    .unwrap_or("scenario failed")
                    .to_owned(),
            );
        }
        if run.roles.iter().all(Role::not_started) {
            RunnerStatus::Seeding
        } else {
            RunnerStatus::Running {
                step: run.roles.iter().filter(|role| role.ready()).count(),
                total: run.roles.len(),
            }
        }
    }

    pub fn receipt(&self) -> Option<Value> {
        self.0.receipt.get().cloned()
    }

    pub(crate) fn role(&self, account: &str) -> Option<&Role> {
        self.0.roles.iter().find(|role| role.account() == account)
    }

    pub fn pending_shots(&self) -> Vec<FileShotRequest> {
        self.0.roles.iter().filter_map(Role::pending_shot).collect()
    }

    pub fn complete_shot(
        &self,
        request: &FileShotRequest,
        presented_tick: u64,
        output: Result<std::path::PathBuf, String>,
    ) -> Result<(), String> {
        let result = if request.plan_id != self.0.id {
            Err("capture request belongs to a different File plan".into())
        } else if let Some(role) = self.role(&request.account) {
            role.complete_shot(request, presented_tick, output)
        } else {
            Err("capture role is unavailable".into())
        };
        if let Err(error) = &result {
            if self.0.receipt.get().is_none() {
                self.0.fail(format!("capture completion refused: {error}"));
            }
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileShotRequest {
    pub id: u64,
    pub account: String,
    pub label: String,
    pub session: u64,
    pub tick: u64,
    plan_id: u64,
}

impl Play {
    pub fn attach_file_scenario(&mut self, run: FileRun) -> Result<(), String> {
        let profile = self
            .connection
            .profile()
            .filter(|profile| run.0.plan.provenance.matches(profile))
            .ok_or("File plan provenance does not match this Play's guarded profile")?;
        if self.file_scenario.get().is_some() || run.0.fixtures.get().is_some() {
            return Err("file scenario already attached".into());
        }
        if run
            .0
            .roles
            .iter()
            .any(|role| !self.profiles.contains_key(role.account()))
        {
            return Err("file scenario role account is not owned by this Play".into());
        }
        let fixtures = run.0.plan.load_fixtures(profile)?;
        let _admission = self
            .script_start_hold
            .lock()
            .map_err(|_| "script start admission lock poisoned")?;
        run.0
            .fixtures
            .set(fixtures)
            .map_err(|_| "file scenario already attached")?;
        self.file_scenario
            .set(run)
            .map_err(|_| "Play already owns a file scenario".into())
    }
}

impl Run {
    fn fail(&self, failure: impl Into<String>) {
        self.failure.get_or_init(|| {
            let _ = self.retirement_started.set(Instant::now());
            for role in &self.roles {
                role.revoke();
            }
            failure.into()
        });
    }

    fn check_deadline(&self) -> bool {
        if Instant::now() >= self.deadline {
            self.fail("whole-run deadline expired");
        }
        self.failure.get().is_none()
    }

    fn advance(&self) {
        if self.receipt.get().is_some() {
            return;
        }
        if self.check_deadline()
            && self.roles.iter().all(Role::ready)
            && self.retirement_started.get().is_none()
        {
            self.fence_attempt.get_or_init(Instant::now);
            // Try-only; no slot work, source drain, output, or Stop in this hold.
            with_boundaries(&self.roles, || {
                if self.check_deadline() && self.roles.iter().all(Role::ready) {
                    if self.roles.iter().any(Role::gameplay_matched) {
                        for role in &self.roles {
                            role.publish_fence();
                        }
                        let _ = self.retirement_started.set(Instant::now());
                    } else {
                        self.fail("the sealed fleet admitted no gameplay check");
                    }
                }
            });
        }
        let timeout = self
            .retirement_started
            .get()
            .is_some_and(|at| at.elapsed() >= RETIREMENT_OBSERVE_CAP)
            && !self.roles.iter().all(Role::stopped);
        if timeout {
            self.fail("owned retirement exceeded its observation cap");
        }
        let tail_expired = Instant::now() >= self.deadline && self.roles.iter().all(Role::stopped);
        if !self.roles.iter().all(Role::done) && !timeout && !tail_expired {
            return;
        }
        self.receipt.get_or_init(|| {
            if !self.roles.iter().all(Role::resolved) { self.fail("one or more role retirements remain unresolved"); }
            self.check_deadline();
            let failure = self.failure.get().map(String::as_str);
            let mut receipt = json!({
                "format": "luau-file-scenario/v1",
                "plan_id": self.id,
                "scenario": self.plan.scenario.name,
                "case": self.plan.scenario.case,
                "profile_provenance": self.plan.provenance,
                "fixture_definition_sha256": self.fixtures.get().map(fixtures::FixtureDefinitions::artifact_sha256),
                "quiesce_policy": self.plan.scenario.quiesce,
                "fence_attempted": self.fence_attempt.get().is_some(),
                "author_limits": self.plan.scenario.limits,
                "retirement_observation_expired": timeout,
                "roles": self.roles.iter().map(Role::receipt).collect::<Vec<_>>(),
                "outcome": if failure.is_none() { "PASS" } else { "FAIL" },
                "failure": failure,
            });
            if let Some(dir) = &self.plan.evidence {
                let write = || -> Result<(), String> {
                    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
                    let staging = dir.join("scenario-file-receipt.tmp");
                    let target = dir.join(FILE_RECEIPT_NAME);
                    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&staging).map_err(|err| err.to_string())?;
                    serde_json::to_writer_pretty(&mut file, &receipt).map_err(|err| err.to_string())?;
                    file.write_all(b"\n").and_then(|()| file.sync_all()).map_err(|err| err.to_string())?;
                    if self.failure.get().is_none() && Instant::now() >= self.deadline {
                        fs::remove_file(&staging).map_err(|err| err.to_string())?;
                        return Err("whole-run deadline expired during receipt write".into());
                    }
                    fs::rename(staging, target).map_err(|err| err.to_string())
                };
                if let Err(error) = write() {
                    self.fail(format!("scenario receipt write: {error}"));
                    receipt["outcome"] = json!("FAIL");
                    receipt["failure"] = json!(self.failure.get());
                    receipt["receipt_write_error"] = json!(error);
                }
            }
            receipt
        });
    }
}

fn with_boundaries(roles: &[Role], publish: impl FnOnce()) {
    if let Some((role, rest)) = roles.split_first() {
        if let Some(_boundary) = role.try_boundary() {
            with_boundaries(rest, publish);
        }
    } else {
        publish();
    }
}
