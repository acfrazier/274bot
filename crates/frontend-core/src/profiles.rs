//! Durable profile writes off the caller's thread. The session stages each
//! change in its in-memory vault and hands it here; one writer thread
//! encrypts and writes the vault file in submission order. A job is one
//! transaction (a rename is its upsert and its remove together). Jobs
//! queued while a write runs are committed in one file write; a job whose
//! every profile a later job in that batch also writes is superseded: its
//! value never becomes the durable one on its own.

use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use vault::{Profile, VaultChange, VaultStore};

use crate::operations::OperationId;

struct Job {
    op: OperationId,
    changes: Vec<VaultChange>,
}

/// The result of one submitted job.
pub(crate) struct Written {
    pub(crate) op: OperationId,
    pub(crate) result: Result<(), String>,
    /// A later job in the same commit wrote every profile this one did.
    pub(crate) superseded: bool,
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
                    // Applied in submission order in one commit, so the file
                    // ends with each profile's last value.
                    let changes: Vec<VaultChange> = batch
                        .iter()
                        .flat_map(|job| job.changes.iter().cloned())
                        .collect();
                    let result = store.commit(&changes).map_err(|e| e.to_string());
                    for (index, job) in batch.iter().enumerate() {
                        let later = &batch[index + 1..];
                        let superseded = job.changes.iter().all(|change| {
                            later.iter().any(|next| {
                                next.changes
                                    .iter()
                                    .any(|c| c.username() == change.username())
                            })
                        });
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

    pub(crate) fn submit(&mut self, op: OperationId, changes: Vec<VaultChange>) {
        if let Some(jobs) = &self.jobs {
            if jobs.send(Job { op, changes }).is_ok() {
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
