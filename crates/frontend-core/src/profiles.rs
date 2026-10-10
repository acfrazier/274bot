//! Durable profile writes off the caller's thread. The session stages each
//! change in its in-memory vault and hands it here; one writer thread
//! encrypts and writes the vault file in submission order. A job is one
//! transaction (a rename is its upsert and its remove together). Jobs
//! queued while a write runs are committed in one file write; a job whose
//! every profile a later job in that batch also writes is superseded: its
//! value never becomes the durable one on its own. Each result names the
//! later jobs of its commit that wrote one of its profiles, so the session
//! can tell which of the job's settings they replace.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use vault::{Profile, ScriptAssignment, VaultChange, VaultStore};

use crate::operations::OperationId;

struct Job {
    op: OperationId,
    changes: Vec<VaultChange>,
    assignment_action: &'static str,
}

/// An assignment transition owned by the last job that wrote its profile
/// in a durable batch.
pub(crate) struct AssignmentChange {
    pub(crate) profile: String,
    pub(crate) old_key: Option<String>,
    pub(crate) new_key: Option<String>,
    pub(crate) availability: Option<(Option<String>, Option<String>)>,
}

impl AssignmentChange {
    fn new(
        profile: &str,
        before: Option<&ScriptAssignment>,
        after: Option<&ScriptAssignment>,
    ) -> Self {
        let old_unavailable = before.and_then(|assignment| assignment.unavailable.as_deref());
        let new_unavailable = after.and_then(|assignment| assignment.unavailable.as_deref());
        let availability = (old_unavailable != new_unavailable).then(|| {
            (
                before.and_then(|assignment| assignment.unavailable.clone()),
                after.and_then(|assignment| assignment.unavailable.clone()),
            )
        });
        Self {
            profile: profile.to_string(),
            old_key: before.map(ScriptAssignment::key),
            new_key: after.map(ScriptAssignment::key),
            availability,
        }
    }

    fn log(&self, op: OperationId, action: &str) {
        use api::hostlog::{Level, Source};
        use std::fmt::Write as _;

        let mut message = format!(
            "script assignment {} -> {}",
            self.old_key.as_deref().unwrap_or("None"),
            self.new_key.as_deref().unwrap_or("None"),
        );
        if let Some((before, after)) = &self.availability {
            message.push_str("; availability ");
            for (index, unavailable) in [before, after].into_iter().enumerate() {
                if index != 0 {
                    message.push_str(" -> ");
                }
                if let Some(reason) = unavailable {
                    message.push_str("unavailable: ");
                    message.push_str(reason);
                } else {
                    message.push_str("available");
                }
            }
        }
        let _ = write!(message, "; action {action}; op#{}", op.0);
        if message.contains(['\n', '\r']) {
            message = message.replace(['\n', '\r'], " ");
        }
        crate::log::global().slot_line(&self.profile, Source::Host, Level::Info, &message);
    }
}

/// The result of one submitted job.
pub(crate) struct Written {
    pub(crate) op: OperationId,
    pub(crate) result: Result<(), String>,
    /// A later job in the same commit wrote every profile this one did.
    pub(crate) superseded: bool,
    /// The later jobs in the same commit that wrote a profile this one
    /// did, in submission order.
    pub(crate) later: Vec<OperationId>,
    /// Each touched profile's durable value after the commit, so a failed
    /// write can put the in-memory view back.
    pub(crate) durable: Vec<(String, Option<Profile>)>,
}

pub(crate) struct ProfileWriter {
    jobs: Option<Sender<Job>>,
    done: Receiver<Written>,
    in_flight: usize,
    thread: Option<JoinHandle<()>>,
}

impl ProfileWriter {
    /// Test builds pass a gate the writer holds while it gathers and
    /// commits a batch; tests hold it to queue several jobs into one batch.
    pub(crate) fn spawn(
        mut store: VaultStore,
        #[cfg(any(test, feature = "test-support"))] gate: Arc<Mutex<()>>,
    ) -> Self {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (report, done) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("profile-writer".into())
            .spawn(move || {
                while let Ok(first) = inbox.recv() {
                    #[cfg(any(test, feature = "test-support"))]
                    let _batch = gate.lock().unwrap_or_else(PoisonError::into_inner);
                    let mut batch = vec![first];
                    batch.extend(inbox.try_iter());
                    // Snapshot only assignment changes and attribute each
                    // one to its final writer in this batch. Compare before
                    // cloning so unchanged assignments allocate nothing.
                    let mut assignment_changes = HashMap::new();
                    for (index, job) in batch.iter().enumerate() {
                        for (change_index, change) in job.changes.iter().enumerate() {
                            let name = change.username();
                            let later_in_job = job.changes[change_index + 1..]
                                .iter()
                                .any(|next| next.username() == name);
                            let later_in_batch = batch[index + 1..].iter().any(|next| {
                                next.changes.iter().any(|change| change.username() == name)
                            });
                            if later_in_job || later_in_batch {
                                continue;
                            }
                            let before = store
                                .get(name)
                                .and_then(|profile| profile.settings.script_assignment.as_ref());
                            let after = match change {
                                VaultChange::Upsert(profile) => {
                                    profile.settings.script_assignment.as_ref()
                                }
                                VaultChange::Remove(_) => None,
                            };
                            if before != after {
                                assignment_changes
                                    .entry(job.op)
                                    .or_insert_with(Vec::new)
                                    .push(AssignmentChange::new(name, before, after));
                            }
                        }
                    }
                    // Applied in submission order in one commit, so the file
                    // ends with each profile's last value.
                    let changes: Vec<VaultChange> = batch
                        .iter()
                        .flat_map(|job| job.changes.iter().cloned())
                        .collect();
                    let result = store.commit(&changes).map_err(|e| e.to_string());
                    for (index, job) in batch.iter().enumerate() {
                        let rest = &batch[index + 1..];
                        let writes = |next: &Job, name: &str| {
                            next.changes.iter().any(|c| c.username() == name)
                        };
                        let superseded = job
                            .changes
                            .iter()
                            .all(|change| rest.iter().any(|next| writes(next, change.username())));
                        let later = rest
                            .iter()
                            .filter(|next| {
                                job.changes
                                    .iter()
                                    .any(|change| writes(next, change.username()))
                            })
                            .map(|next| next.op)
                            .collect();
                        let durable = job
                            .changes
                            .iter()
                            .map(|change| {
                                let name = change.username();
                                (name.to_string(), store.get(name).cloned())
                            })
                            .collect();
                        if result.is_ok() {
                            if let Some(changes) = assignment_changes.remove(&job.op) {
                                for change in changes {
                                    change.log(job.op, job.assignment_action);
                                }
                            }
                        }
                        let written = Written {
                            op: job.op,
                            result: result.clone(),
                            superseded,
                            later,
                            durable,
                        };
                        if report.send(written).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("spawn profile writer");
        Self {
            jobs: Some(jobs),
            done,
            in_flight: 0,
            thread: Some(thread),
        }
    }

    pub(crate) fn submit(
        &mut self,
        op: OperationId,
        changes: Vec<VaultChange>,
        assignment_action: &'static str,
    ) {
        if let Some(jobs) = &self.jobs {
            if jobs
                .send(Job {
                    op,
                    changes,
                    assignment_action,
                })
                .is_ok()
            {
                self.in_flight += 1;
            }
        }
    }

    /// Results that are ready, without waiting.
    pub(crate) fn try_take(&mut self) -> Option<Written> {
        let written = self.done.try_recv().ok()?;
        self.in_flight -= 1;
        Some(written)
    }

    /// The next result, waiting for it; `None` when nothing is in flight.
    pub(crate) fn wait_take(&mut self) -> Option<Written> {
        if self.in_flight == 0 {
            return None;
        }
        let written = self.done.recv().ok()?;
        self.in_flight -= 1;
        Some(written)
    }
}

impl Drop for ProfileWriter {
    /// Queued writes still reach the disk: closing the queue lets the
    /// writer finish them before it exits.
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
