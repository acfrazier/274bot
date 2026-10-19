//! Save feedback for a profile edit form, shared by the panel's Profiles
//! form and the TUI settings popup.
//!
//! A save refused before anything was written shows in the form, with
//! [`NOTHING_SAVED`] under its reason. A save that was accepted is a
//! [`SaveRecord`] owned by its write's operation: `Saved <name>.` shows once
//! the write is durable, and a write that fails after it was queued shows its
//! error, with [`NOTHING_SAVED`], in the form the save was submitted from.
//! Both show only on that form instance: when it was switched or closed
//! first, the outcome is skipped here (a failure still reaches the banner or
//! status line through [`OperatorSession::take_write_failures`]), so it never
//! shows in another profile's form. While [`ProfileFormSave::saving`] the
//! form's save has not settled, and a front end asks before it leaves the
//! form.

use crate::operations::OperationId;
use crate::profile_saves::SaveRecord;
use crate::session::OperatorSession;

/// The line under every refused or failed save's reason.
pub const NOTHING_SAVED: &str = "nothing was saved.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormNoticeKind {
    Refused,
    Failed,
    Saved,
}

/// What a profile form shows under its fields. The text is built once, when
/// the notice is set, so drawing it only borrows.
#[derive(Debug, PartialEq, Eq)]
pub struct FormNotice {
    kind: FormNoticeKind,
    text: String,
}

impl FormNotice {
    /// Refused before anything was written.
    pub fn refused(reason: impl Into<String>) -> Self {
        Self {
            kind: FormNoticeKind::Refused,
            text: reason.into(),
        }
    }

    /// The write failed after it was accepted.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            kind: FormNoticeKind::Failed,
            text: reason.into(),
        }
    }

    /// The form's save is durable.
    pub fn saved(name: impl Into<String>) -> Self {
        Self {
            kind: FormNoticeKind::Saved,
            text: name.into(),
        }
    }

    /// Whether this notice is a durable save.
    pub fn is_saved(&self) -> bool {
        self.kind == FormNoticeKind::Saved
    }

    /// Whether this notice is a refused save.
    pub fn is_refused(&self) -> bool {
        self.kind == FormNoticeKind::Refused
    }

    /// Whether this notice is a failed write.
    pub fn is_failed(&self) -> bool {
        self.kind == FormNoticeKind::Failed
    }

    /// The reason a refused or failed save shows above [`NOTHING_SAVED`].
    pub fn error(&self) -> Option<&str> {
        match self.kind {
            FormNoticeKind::Refused | FormNoticeKind::Failed => Some(self.text.as_str()),
            FormNoticeKind::Saved => None,
        }
    }

    /// The notice's own line: the reason, or `Saved <name>.`.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A durable save seen by [`ProfileFormSave::settle`].
#[derive(Debug, PartialEq)]
pub struct SavedProfile {
    /// The operation's record of the save: the profile written
    /// (`destination`, a rename's new name) and the draft it carried.
    record: SaveRecord,
    /// Whether the form it was submitted from is still showing. That form
    /// now shows `Saved <name>.`.
    same_form: bool,
}

impl SavedProfile {
    pub(crate) fn new(record: SaveRecord, same_form: bool) -> Self {
        Self { record, same_form }
    }

    /// The operation's record of the save.
    pub fn record(&self) -> &SaveRecord {
        &self.record
    }

    /// Whether the form it was submitted from is still showing.
    pub fn same_form(&self) -> bool {
        self.same_form
    }

    /// Take the written profile name.
    pub fn into_destination(self) -> String {
        self.record.into_destination()
    }
}

/// A save whose write failed, seen by [`ProfileFormSave::settle`].
#[derive(Debug, PartialEq)]
pub struct FailedSave {
    /// The operation's record of the save: the profile it would have
    /// written and the draft that did not land.
    record: SaveRecord,
    /// Whether the form it was submitted from is still showing. That form
    /// now shows the failure, and still holds the draft.
    same_form: bool,
}

impl FailedSave {
    pub(crate) fn new(record: SaveRecord, same_form: bool) -> Self {
        Self { record, same_form }
    }

    /// The operation's record of the save.
    pub fn record(&self) -> &SaveRecord {
        &self.record
    }

    /// Whether the form it was submitted from is still showing.
    pub fn same_form(&self) -> bool {
        self.same_form
    }
}

#[derive(Debug, PartialEq)]
enum FormSettledKind {
    Saved(SavedProfile),
    Failed(FailedSave),
}

/// One settled save.
#[derive(Debug, PartialEq)]
pub struct FormSettled(FormSettledKind);

impl FormSettled {
    pub(crate) fn saved(profile: SavedProfile) -> Self {
        Self(FormSettledKind::Saved(profile))
    }

    pub(crate) fn failed(save: FailedSave) -> Self {
        Self(FormSettledKind::Failed(save))
    }

    /// A durable save.
    pub fn as_saved(&self) -> Option<&SavedProfile> {
        match &self.0 {
            FormSettledKind::Saved(profile) => Some(profile),
            FormSettledKind::Failed(_) => None,
        }
    }

    /// Consume a durable save.
    pub fn into_saved(self) -> Option<SavedProfile> {
        match self.0 {
            FormSettledKind::Saved(profile) => Some(profile),
            FormSettledKind::Failed(_) => None,
        }
    }

    /// A failed write.
    pub fn as_failed(&self) -> Option<&FailedSave> {
        match &self.0 {
            FormSettledKind::Failed(save) => Some(save),
            FormSettledKind::Saved(_) => None,
        }
    }
}

/// One profile edit form's save feedback: which form instance is showing
/// and the form's notice. The saves themselves are the session's
/// [`SaveRecord`]s, owned by their write operations; the form asks the
/// session which of them are its own (a session has one such form).
#[derive(Debug, Default)]
pub struct ProfileFormSave {
    form: u64,
    notice: Option<FormNotice>,
}

impl ProfileFormSave {
    /// The form was opened on a target, switched to another one, or closed.
    /// Its notice goes, and a save still in flight from before never shows
    /// one.
    pub(crate) fn form_changed(&mut self) {
        self.form = self.form.wrapping_add(1);
        self.notice = None;
    }

    /// The operator edited the form: a notice about its last save no longer
    /// describes it.
    pub(crate) fn edited(&mut self) {
        self.notice = None;
    }

    /// The save was refused before anything was written.
    pub(crate) fn refused(&mut self, reason: impl Into<String>) {
        self.notice = Some(FormNotice::refused(reason));
    }

    /// The save was accepted and queued as `op`, writing profile
    /// `destination` (a rename's or a new profile's name) from a form that
    /// was editing `source` (`None` for a new profile). The record of it,
    /// with the draft the write carries (the staged row), is registered on
    /// `core` under `op`; it is the only state of the save.
    pub(crate) fn submitted<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        op: OperationId,
        source: Option<&str>,
        destination: impl Into<String>,
    ) {
        let destination = destination.into();
        self.notice = None;
        // An accepted write has staged its row.
        let Some(draft) = core.vault().and_then(|v| v.get(&destination)).cloned() else {
            return;
        };
        core.track_save(SaveRecord::new(
            op,
            source.map(str::to_string),
            destination,
            draft,
            self.form,
        ));
    }

    /// This form instance's own saves that `core` has not settled: the form
    /// holds a draft whose write can still fail.
    fn own<'a, Io>(&self, core: &'a OperatorSession<Io>) -> impl Iterator<Item = &'a SaveRecord> {
        let form = self.form;
        core.saves_in_flight()
            .iter()
            .filter(move |r| r.form() == form)
    }

    /// Whether this form instance's own save is still being written:
    /// leaving the form is not free, since the write can still fail.
    pub fn saving<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.own(core).next().is_some()
    }

    /// The profile the newest of this form instance's saves still in flight
    /// writes.
    pub fn saving_name<'a, Io>(&self, core: &'a OperatorSession<Io>) -> Option<&'a str> {
        self.own(core).last().map(|r| r.destination())
    }

    pub fn notice(&self) -> Option<&FormNotice> {
        self.notice.as_ref()
    }

    /// Settle the saves the session has finished writing, from their
    /// records. Each durable save is [`FormSettled::Saved`], each failed
    /// write [`FormSettled::Failed`]; the form they came from, if still
    /// showing, gets the notice. A save whose profile a later write removed
    /// settles silently.
    pub fn settle<Io>(&mut self, core: &mut OperatorSession<Io>) -> Vec<FormSettled> {
        let mut settled = Vec::new();
        for done in core.take_settled_saves() {
            let same_form = done.record().form() == self.form;
            let destination = done.record().destination().to_owned();
            let (record, result) = done.into_parts();
            if result.is_saved() {
                if same_form {
                    self.notice = Some(FormNotice::saved(format!("Saved {destination}.")));
                }
                settled.push(FormSettled::saved(SavedProfile::new(record, same_form)));
            } else if let Some(failure) = result.failure() {
                if same_form {
                    self.notice = Some(FormNotice::failed(failure.to_string()));
                }
                settled.push(FormSettled::failed(FailedSave::new(record, same_form)));
            }
        }
        settled
    }
}
