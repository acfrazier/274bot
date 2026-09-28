//! Save feedback for a profile edit form, shared by the panel's Profiles
//! form and the TUI settings popup.
//!
//! A save refused before anything was written shows in the form, with
//! [`NOTHING_SAVED`] under its reason. `Saved <name>.` shows once the write
//! is durable, and only on the form instance it was submitted from: when
//! that form was switched or closed first, it is skipped. A write that fails
//! after it was accepted is not routed here at all: like every profile
//! write, it reaches the banner or status line and the log through
//! [`OperatorSession::take_write_failures`], so it can never show in another
//! profile's form.

use crate::operations::{OperationId, Outcome};
use crate::session::OperatorSession;

/// The line under every refused save's reason.
pub const NOTHING_SAVED: &str = "nothing was saved.";

/// What a profile form shows under its fields. The text is built once, when
/// the notice is set, so drawing it only borrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormNotice {
    /// Refused before anything was written: the reason, shown with
    /// [`NOTHING_SAVED`].
    Refused(String),
    /// The form's save is durable: `Saved <name>.`
    Saved(String),
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

/// An accepted save whose write has not settled, and the form instance it
/// was submitted from.
#[derive(Debug)]
struct Submitted {
    op: OperationId,
    name: String,
    form: u64,
}

/// One profile edit form's save feedback: which form instance is showing,
/// the save still being written, and the form's notice.
#[derive(Debug, Default)]
pub struct ProfileFormSave {
    form: u64,
    submitted: Option<Submitted>,
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

    /// The save was accepted and queued as `op`, writing profile `name` (a
    /// rename's new name). It replaces any earlier save still in flight.
    pub fn submitted(&mut self, op: OperationId, name: impl Into<String>) {
        self.notice = None;
        self.submitted = Some(Submitted {
            op,
            name: name.into(),
            form: self.form,
        });
    }

    /// Whether this form instance's own save is still being written.
    pub fn saving(&self) -> bool {
        self.submitted.as_ref().is_some_and(|s| s.form == self.form)
    }

    /// The profile a save still in flight writes, whichever form it came
    /// from.
    pub fn in_flight(&self) -> Option<&str> {
        self.submitted.as_ref().map(|s| s.name.as_str())
    }

    pub fn notice(&self) -> Option<&FormNotice> {
        self.notice.as_ref()
    }

    /// Settle the save in flight from its operation. `Some` once the write
    /// is durable. A failed or superseded write is dropped here without a
    /// notice: its failure is reported through `take_write_failures`.
    pub fn settle<Io>(&mut self, core: &OperatorSession<Io>) -> Option<SavedProfile> {
        let submitted = self.submitted.as_ref()?;
        let durable = match core
            .operation(submitted.op)
            .and_then(|report| report.outcome(&submitted.name))
        {
            Some(Outcome::Pending) => return None,
            Some(Outcome::Completed) => true,
            // Failed, superseded, or already evicted from settled history.
            _ => false,
        };
        let Submitted { name, form, .. } = self.submitted.take()?;
        if !durable {
            return None;
        }
        let same_form = form == self.form;
        if same_form {
            self.notice = Some(FormNotice::Saved(format!("Saved {name}.")));
        }
        Some(SavedProfile { name, same_form })
    }
}
