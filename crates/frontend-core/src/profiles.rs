//! Durable profile writes off the caller's thread. The session stages each
//! change in its in-memory vault and hands it here; one writer thread
//! encrypts and writes the vault file in submission order. A job is one
//! transaction (a rename is its upsert and its remove together). Jobs
//! queued while a write runs are committed in one file write; a job whose
//! every profile a later job in that batch also writes is superseded: its
//! value never becomes the durable one on its own. Each result names the
//! later jobs of its commit that wrote one of its profiles, so the session
//! can tell which of the job's settings they replace.

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

/// An assignment transition owned by the last job that changed the assignment
/// in a durable batch, even if later parameter-only jobs write the profile.
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

struct AssignmentNameState<'a> {
    username: &'a str,
    uid: Option<i32>,
}

/// Assignment snapshots are keyed by the profile UID, which is unchanged
/// across a rename. References point into the durable store or queued changes;
/// only the final log records clone assignment fields.
struct AssignmentState<'a> {
    uid: i32,
    initial: Option<&'a ScriptAssignment>,
    current: Option<&'a ScriptAssignment>,
    current_name: Option<&'a str>,
    log_name: Option<&'a str>,
    owner: Option<(OperationId, &'static str)>,
}

fn durable_assignment_changes<'a>(
    store: &'a VaultStore,
    batch: &'a [Job],
) -> Vec<(OperationId, &'static str, AssignmentChange)> {
    // Keep parameter-only batches allocation-free. The conservative check
    // allows rename pairs through when their destination does not exist yet.
    let might_change = batch.iter().any(|job| {
        job.changes.iter().any(|change| match change {
            VaultChange::Upsert(profile) => {
                let after = profile.settings.script_assignment.as_ref();
                match store.get(&profile.username) {
                    Some(before) => {
                        before.uid != profile.uid
                            || before.settings.script_assignment.as_ref() != after
                    }
                    None => after.is_some(),
                }
            }
            VaultChange::Remove(username) => store
                .get(username)
                .is_some_and(|profile| profile.settings.script_assignment.is_some()),
        })
    });
    if !might_change {
        return Vec::new();
    }

    let mut names: Vec<AssignmentNameState<'a>> = Vec::new();
    let mut states: Vec<AssignmentState<'a>> = Vec::new();
    for job in batch {
        for change in &job.changes {
            let username = change.username();
            if names.iter().any(|name| name.username == username) {
                continue;
            }
            let profile = store.get(username);
            names.push(AssignmentNameState {
                username,
                uid: profile.map(|profile| profile.uid),
            });
            if let Some(profile) = profile {
                if !states.iter().any(|state| state.uid == profile.uid) {
                    let assignment = profile.settings.script_assignment.as_ref();
                    states.push(AssignmentState {
                        uid: profile.uid,
                        initial: assignment,
                        current: assignment,
                        current_name: Some(profile.username.as_str()),
                        log_name: Some(profile.username.as_str()),
                        owner: None,
                    });
                }
            }
        }
    }

    for job in batch {
        for change in &job.changes {
            match change {
                VaultChange::Upsert(profile) => {
                    let username = profile.username.as_str();
                    let name_index = names
                        .iter()
                        .position(|name| name.username == username)
                        .expect("every changed profile name was collected");
                    if let Some(previous_uid) = names[name_index].uid {
                        if previous_uid != profile.uid {
                            if let Some(state_index) =
                                states.iter().position(|state| state.uid == previous_uid)
                            {
                                let state = &mut states[state_index];
                                if state.current_name == Some(username) {
                                    if state.current.is_some() {
                                        state.owner = Some((job.op, job.assignment_action));
                                    }
                                    state.current = None;
                                    state.current_name = None;
                                }
                            }
                        }
                    }
                    names[name_index].uid = Some(profile.uid);

                    let state_index = match states.iter().position(|state| state.uid == profile.uid)
                    {
                        Some(index) => index,
                        None => {
                            states.push(AssignmentState {
                                uid: profile.uid,
                                initial: None,
                                current: None,
                                current_name: None,
                                log_name: None,
                                owner: None,
                            });
                            states.len() - 1
                        }
                    };
                    let state = &mut states[state_index];
                    let after = profile.settings.script_assignment.as_ref();
                    if state.current != after {
                        state.owner = Some((job.op, job.assignment_action));
                    }
                    state.current = after;
                    state.current_name = Some(username);
                    state.log_name = Some(username);
                }
                VaultChange::Remove(username) => {
                    let name_index = names
                        .iter()
                        .position(|name| name.username == username)
                        .expect("every changed profile name was collected");
                    if let Some(uid) = names[name_index].uid.take() {
                        if let Some(state_index) = states.iter().position(|state| state.uid == uid)
                        {
                            let state = &mut states[state_index];
                            if state.current_name == Some(username.as_str()) {
                                if state.current.is_some() {
                                    state.owner = Some((job.op, job.assignment_action));
                                }
                                state.current = None;
                                state.current_name = None;
                            }
                        }
                    }
                }
            }
        }
    }

    states
        .into_iter()
        .filter_map(|state| {
            if state.initial == state.current {
                return None;
            }
            let (op, action) = state.owner?;
            let username = state.current_name.or(state.log_name)?;
            Some((
                op,
                action,
                AssignmentChange::new(username, state.initial, state.current),
            ))
        })
        .collect()
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
                    let assignment_changes = durable_assignment_changes(&store, &batch);

                    // Applied in submission order in one commit, so the file
                    // ends with each profile's last value.
                    let changes: Vec<VaultChange> = batch
                        .iter()
                        .flat_map(|job| job.changes.iter().cloned())
                        .collect();
                    let result = store.commit(&changes).map_err(|e| e.to_string());
                    if result.is_ok() {
                        for (op, action, change) in &assignment_changes {
                            change.log(*op, action);
                        }
                    }

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
