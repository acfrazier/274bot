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
use crate::profile_saves::{SaveRecord, SaveResult};
use crate::session::OperatorSession;

/// The line under every refused or failed save's reason.
pub const NOTHING_SAVED: &str = "nothing was saved.";

/// What a profile form shows under its fields. The text is built once, when
/// the notice is set, so drawing it only borrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormNotice {
    /// Refused before anything was written: the reason, shown with
    /// [`NOTHING_SAVED`].
    Refused(String),
    /// The write failed after it was accepted: `label: error`, shown with
    /// [`NOTHING_SAVED`]. The durable profile is unchanged.
    Failed(String),
    /// The form's save is durable: `Saved <name>.`
    Saved(String),
}

impl FormNotice {
    /// The reason a refused or failed save shows above [`NOTHING_SAVED`].
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Refused(reason) | Self::Failed(reason) => Some(reason),
            Self::Saved(_) => None,
        }
    }

    /// The notice's own line: the reason, or `Saved <name>.`.
    pub fn text(&self) -> &str {
        match self {
            Self::Refused(text) | Self::Failed(text) | Self::Saved(text) => text,
        }
    }
}

/// A durable save seen by [`ProfileFormSave::settle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProfile {
    /// The profile written (a rename's new name).
    pub name: String,
    /// Whether the form it was submitted from is still showing. That form
    /// now shows `Saved <name>.`.
    pub same_form: bool,
}

/// A save whose write failed, seen by [`ProfileFormSave::settle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedSave {
    /// The profile the save would have written.
    pub name: String,
    /// Whether the form it was submitted from is still showing. That form
    /// now shows the failure, and still holds the draft.
    pub same_form: bool,
}

/// One settled save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormSettled {
    Saved(SavedProfile),
    Failed(FailedSave),
}

/// An accepted save whose write has not settled, and the form instance it
/// was submitted from.
#[derive(Debug)]
struct Submitted {
    op: OperationId,
    name: String,
    form: u64,
}

/// One profile edit form's save feedback: which form instance is showing,
/// the saves still being written, and the form's notice.
#[derive(Debug, Default)]
pub struct ProfileFormSave {
    form: u64,
    submitted: Vec<Submitted>,
    notice: Option<FormNotice>,
}

impl ProfileFormSave {
    /// The form was opened on a target, switched to another one, or closed.
    /// Its notice goes, and a save still in flight from before never shows
    /// one.
    pub fn form_changed(&mut self) {
        self.form = self.form.wrapping_add(1);
        self.notice = None;
    }

    /// The operator edited the form: a notice about its last save no longer
    /// describes it.
    pub fn edited(&mut self) {
        self.notice = None;
    }

    /// The save was refused before anything was written.
    pub fn refused(&mut self, reason: impl Into<String>) {
        self.notice = Some(FormNotice::Refused(reason.into()));
    }

    /// The save was accepted and queued as `op`, writing profile
    /// `destination` (a rename's or a new profile's name) from a form that
    /// was editing `source` (`None` for a new profile). The record of it,
    /// with the draft the write carries (the staged row), is registered on
    /// `core` under `op`.
    pub fn submitted<Io>(
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
        core.track_save(SaveRecord {
            op,
            source: source.map(str::to_string),
            destination: destination.clone(),
            draft,
            form: self.form,
        });
        self.submitted.push(Submitted {
            op,
            name: destination,
            form: self.form,
        });
    }

    /// Whether this form instance's own save is still being written: the
    /// form holds a draft whose write can still fail, so leaving it is not
    /// free.
    pub fn saving(&self) -> bool {
        self.submitted.iter().any(|s| s.form == self.form)
    }

    /// The profile the newest save still in flight writes, whichever form it
    /// came from.
    pub fn in_flight(&self) -> Option<&str> {
        self.submitted.last().map(|s| s.name.as_str())
    }

    pub fn notice(&self) -> Option<&FormNotice> {
        self.notice.as_ref()
    }

    /// Settle the saves in flight from their operations' records. Each
    /// durable save is [`FormSettled::Saved`], each failed write
    /// [`FormSettled::Failed`]; the form they came from, if still showing,
    /// gets the notice. A save whose profile a later write removed settles
    /// silently.
    pub fn settle<Io>(&mut self, core: &mut OperatorSession<Io>) -> Vec<FormSettled> {
        let mut settled = Vec::new();
        let mut i = 0;
        while i < self.submitted.len() {
            let Some(done) = core.take_save(self.submitted[i].op) else {
                i += 1;
                continue;
            };
            let Submitted { name, form, .. } = self.submitted.remove(i);
            let same_form = form == self.form;
            match done.result {
                SaveResult::Saved => {
                    if same_form {
                        self.notice = Some(FormNotice::Saved(format!("Saved {name}.")));
                    }
                    settled.push(FormSettled::Saved(SavedProfile { name, same_form }));
                }
                SaveResult::Failed(failure) => {
                    if same_form {
                        self.notice = Some(FormNotice::Failed(failure.to_string()));
                    }
                    settled.push(FormSettled::Failed(FailedSave { name, same_form }));
                }
                SaveResult::Gone => {}
            }
        }
        settled
    }
}
