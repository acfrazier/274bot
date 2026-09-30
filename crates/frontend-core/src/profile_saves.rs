//! Operation-owned records of profile-form saves, and the structured failure
//! of a durable profile write.
//!
//! The session settles every queued profile write in
//! [`OperatorSession::poll`](crate::OperatorSession::poll). A form that
//! submitted one registers a [`SaveRecord`] under the write's operation id,
//! so the outcome is routed by that id and never by the text of a message: a
//! write that fails after it was queued reaches the form it came from, and
//! only that form, whatever the operator opened or closed meanwhile. The
//! commit result is shared by every write in one commit, so a save that a
//! later write superseded settles from its own commit too.

use std::collections::HashMap;
use std::fmt;

use vault::Profile;

use crate::operations::OperationId;

/// A durable profile write that failed after it was queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFailure {
    /// The failed write's operation.
    pub op: OperationId,
    /// The profile it was about (a rename's new name).
    pub target: String,
    /// The edit that queued it (`credentials`, `random`, `auto-login`, …).
    pub label: &'static str,
    /// The I/O or vault error.
    pub error: String,
}

impl fmt::Display for WriteFailure {
    /// The banner / status-line text: `label: error`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.label, self.error)
    }
}

/// One accepted profile-form save, owned by its write operation.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveRecord {
    /// The queued write.
    pub op: OperationId,
    /// The profile the form was editing when it saved; `None` for a new
    /// profile.
    pub source: Option<String>,
    /// The profile written (a rename's or a new profile's name).
    pub destination: String,
    /// The draft the write carries, as submitted. Later edits of the form
    /// never change it.
    pub draft: Profile,
    /// The form instance the save was submitted from
    /// ([`ProfileFormSave::form_changed`](crate::ProfileFormSave::form_changed)
    /// starts the next one).
    pub form: u64,
}

/// How a save's write ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveResult {
    /// The profile is durable.
    Saved,
    /// The write failed and the durable value was put back.
    Failed(WriteFailure),
    /// The commit succeeded, but the profile is no longer in the file: a
    /// later write removed it.
    Gone,
}

/// A settled [`SaveRecord`].
#[derive(Debug, Clone, PartialEq)]
pub struct SaveSettled {
    pub record: SaveRecord,
    pub result: SaveResult,
}

/// The session's records of saves in flight and the results not yet taken.
#[derive(Default)]
pub(crate) struct SaveBook {
    records: HashMap<OperationId, SaveRecord>,
    settled: HashMap<OperationId, SaveSettled>,
}

impl SaveBook {
    pub(crate) fn track(&mut self, record: SaveRecord) {
        self.records.insert(record.op, record);
    }

    pub(crate) fn tracks(&self, op: OperationId) -> bool {
        self.records.contains_key(&op)
    }

    /// `op`'s write ended; a record tracked under it settles.
    pub(crate) fn settle(&mut self, op: OperationId, result: SaveResult) {
        if let Some(record) = self.records.remove(&op) {
            self.settled.insert(op, SaveSettled { record, result });
        }
    }

    pub(crate) fn take(&mut self, op: OperationId) -> Option<SaveSettled> {
        self.settled.remove(&op)
    }
}
