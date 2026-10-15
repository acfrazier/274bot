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

use std::fmt;

use vault::Profile;

use crate::operations::OperationId;

/// A durable profile write that failed after it was queued.
#[derive(Debug, PartialEq, Eq)]
pub struct WriteFailure {
    /// The failed write's operation.
    op: OperationId,
    /// The profile it was about (a rename's new name).
    target: String,
    /// The edit that queued it (`credentials`, `random`, `auto-login`, …).
    label: &'static str,
    /// The I/O or vault error.
    error: String,
}

impl WriteFailure {
    pub(crate) fn new(
        op: OperationId,
        target: impl Into<String>,
        label: &'static str,
        error: impl Into<String>,
    ) -> Self {
        Self {
            op,
            target: target.into(),
            label,
            error: error.into(),
        }
    }

    /// The failed write's operation.
    pub fn op(&self) -> OperationId {
        self.op
    }

    /// The profile it was about (a rename's new name).
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The edit that queued it (`credentials`, `random`, `auto-login`, …).
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The I/O or vault error.
    pub fn error(&self) -> &str {
        &self.error
    }

    /// A second owner of this failure. Not [`Clone`].
    pub(crate) fn duplicate(&self) -> Self {
        Self {
            op: self.op,
            target: self.target.clone(),
            label: self.label,
            error: self.error.clone(),
        }
    }
}

impl fmt::Display for WriteFailure {
    /// The banner / status-line text: `label: error`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.label, self.error)
    }
}

/// One accepted profile-form save, owned by its write operation.
#[derive(Debug, PartialEq)]
pub struct SaveRecord {
    /// The queued write.
    op: OperationId,
    /// The profile the form was editing when it saved; `None` for a new
    /// profile.
    source: Option<String>,
    /// The profile written (a rename's or a new profile's name).
    destination: String,
    /// The draft the write carries, as submitted. Later edits of the form
    /// never change it.
    draft: Profile,
    /// The form instance the save was submitted from
    /// ([`ProfileFormSave::form_changed`](crate::ProfileFormSave::form_changed)
    /// starts the next one).
    form: u64,
}

impl SaveRecord {
    pub(crate) fn new(
        op: OperationId,
        source: Option<String>,
        destination: impl Into<String>,
        draft: Profile,
        form: u64,
    ) -> Self {
        Self {
            op,
            source,
            destination: destination.into(),
            draft,
            form,
        }
    }

    /// The queued write.
    pub fn op(&self) -> OperationId {
        self.op
    }

    /// The profile the form was editing when it saved; `None` for a new profile.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// The profile written (a rename's or a new profile's name).
    pub fn destination(&self) -> &str {
        &self.destination
    }

    /// The draft the write carries, as submitted.
    pub fn draft(&self) -> &Profile {
        &self.draft
    }

    /// The form instance the save was submitted from.
    pub fn form(&self) -> u64 {
        self.form
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SaveResultKind {
    Saved,
    Failed,
    Gone,
}

/// How a save's write ended.
#[derive(Debug, PartialEq, Eq)]
pub struct SaveResult {
    kind: SaveResultKind,
    failure: Option<WriteFailure>,
}

#[allow(non_upper_case_globals)]
impl SaveResult {
    /// The profile is durable.
    pub const Saved: Self = Self {
        kind: SaveResultKind::Saved,
        failure: None,
    };

    /// The commit succeeded, but the profile is no longer in the file: a
    /// later write removed it.
    pub const Gone: Self = Self {
        kind: SaveResultKind::Gone,
        failure: None,
    };

    pub(crate) fn failed(failure: WriteFailure) -> Self {
        Self {
            kind: SaveResultKind::Failed,
            failure: Some(failure),
        }
    }

    /// Whether the profile is durable.
    pub fn is_saved(&self) -> bool {
        self.kind == SaveResultKind::Saved
    }

    /// Whether a later write removed the profile.
    pub fn is_gone(&self) -> bool {
        self.kind == SaveResultKind::Gone
    }

    /// The write failure, when this result failed.
    pub fn failure(&self) -> Option<&WriteFailure> {
        self.failure.as_ref()
    }
}

/// A settled [`SaveRecord`].
#[derive(Debug, PartialEq)]
pub struct SaveSettled {
    record: SaveRecord,
    result: SaveResult,
}

impl SaveSettled {
    pub(crate) fn new(record: SaveRecord, result: SaveResult) -> Self {
        Self { record, result }
    }

    /// The save that settled.
    pub fn record(&self) -> &SaveRecord {
        &self.record
    }

    /// How the write ended.
    pub fn result(&self) -> &SaveResult {
        &self.result
    }

    /// Consume the settled save.
    pub(crate) fn into_parts(self) -> (SaveRecord, SaveResult) {
        (self.record, self.result)
    }
}

/// The session's records of saves in flight and the results not yet taken,
/// each in the order it happened. The operation owns the record: a form only
/// asks the session about it.
#[derive(Default)]
pub(crate) struct SaveBook {
    records: Vec<SaveRecord>,
    settled: Vec<SaveSettled>,
}

impl SaveBook {
    pub(crate) fn track(&mut self, record: SaveRecord) {
        self.records.push(record);
    }

    pub(crate) fn tracks(&self, op: OperationId) -> bool {
        self.records.iter().any(|r| r.op == op)
    }

    /// The saves whose writes have not settled, oldest first.
    pub(crate) fn in_flight(&self) -> &[SaveRecord] {
        &self.records
    }

    /// `op`'s write ended; a record tracked under it settles.
    pub(crate) fn settle(&mut self, op: OperationId, result: SaveResult) {
        if let Some(i) = self.records.iter().position(|r| r.op == op) {
            let record = self.records.remove(i);
            self.settled.push(SaveSettled::new(record, result));
        }
    }

    pub(crate) fn take_settled(&mut self) -> Vec<SaveSettled> {
        std::mem::take(&mut self.settled)
    }
}
