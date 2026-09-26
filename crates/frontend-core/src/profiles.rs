//! Durable profile writes off the caller's thread. The session stages each
//! change in its in-memory vault and hands it here; one writer thread
//! encrypts and writes the vault file in submission order. Changes queued
//! while a write is running are committed together, so the last write per
//! profile wins without an extra file rewrite.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use vault::{Profile, VaultChange, VaultStore};

use crate::operations::OperationId;

struct Job {
    op: OperationId,
    change: VaultChange,
}

/// The result of one submitted change.
pub(crate) struct Written {
    pub(crate) op: OperationId,
    pub(crate) username: String,
    pub(crate) result: Result<(), String>,
    /// The profile's durable value after this commit, so a failed write can
    /// put the in-memory view back.
    pub(crate) durable: Option<Profile>,
}

pub(crate) struct ProfileWriter {
    jobs: Option<Sender<Job>>,
    done: Receiver<Written>,
    in_flight: usize,
    thread: Option<JoinHandle<()>>,
}

impl ProfileWriter {
    pub(crate) fn spawn(mut store: VaultStore) -> Self {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (report, done) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("profile-writer".into())
            .spawn(move || {
                while let Ok(first) = inbox.recv() {
                    let mut batch = vec![first];
                    batch.extend(inbox.try_iter());
                    // Last write per profile wins; earlier changes to the
                    // same profile are contained in it (the session stages
                    // each change on top of the previous one).
                    let mut last: HashMap<&str, usize> = HashMap::new();
                    for (index, job) in batch.iter().enumerate() {
                        last.insert(job.change.username(), index);
                    }
                    let changes: Vec<VaultChange> = batch
                        .iter()
                        .enumerate()
                        .filter(|(index, job)| last[job.change.username()] == *index)
                        .map(|(_, job)| job.change.clone())
                        .collect();
                    let result = store.commit(&changes).map_err(|e| e.to_string());
                    for job in batch {
                        let username = job.change.username().to_string();
                        let durable = store.get(&username).cloned();
                        let written = Written {
                            op: job.op,
                            username,
                            result: result.clone(),
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

    pub(crate) fn submit(&mut self, op: OperationId, change: VaultChange) {
        if let Some(jobs) = &self.jobs {
            if jobs.send(Job { op, change }).is_ok() {
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
