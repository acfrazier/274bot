//! The global PvP setting of the shared preferences document: `pvp.start_fights`
//! (PVP-DESIGN-1026 §2.7). It lets any script start a fight with a player
//! where the game allows player combat. Fighting back is never affected.
//!
//! The process service projects the member for the host (`pvp_globals`); the
//! `SetPvpStartFights` command writes it. Both front ends show the same row.

use api::identity::{RequestId, Settlement};
use host_play::local_state::Update;

use crate::prefs::{PreferenceError, PreferencePatch, UiPreferences};
use crate::OperatorSession;

/// The row label both front ends show.
pub const START_FIGHTS_LABEL: &str = "Start fights with players";

/// What the row means, shown beside it. It is off by default.
pub const START_FIGHTS_NOTE: &str = "Lets any script start a fight with a player in the Wilderness or a duel pen. Bots always fight back when a player attacks them.";

/// The saved value, read from the host's cached projection. `Err` while the
/// store is cold or refused, or when the member is malformed: show it as off.
pub fn pvp_start_fights(preferences: &UiPreferences) -> Result<bool, PreferenceError> {
    preferences
        .inner()
        .pvp_globals()
        .map(|globals| globals.start_fights)
}

/// Submit the member without waiting. The service merges only
/// `pvp.start_fights` into its latest durable document, and logs one audit
/// line for the durable write (profile, action, operation id, `old -> new`).
pub(crate) fn write_pvp_start_fights(
    preferences: &UiPreferences,
    enabled: bool,
) -> Result<Update, PreferenceError> {
    preferences.begin_patch(PreferencePatch::member(
        &["pvp", "start_fights"],
        Some(serde_json::Value::Bool(enabled)),
    )?)
}

/// The prefix of every error line this setting's save leaves.
const START_FIGHTS_ERROR: &str = "start fights with players";

/// How a click's command settled, as [`PvpSettingView::follow`] reports it once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvpWrite {
    /// The save failed or was refused; the row shows the saved value again and
    /// [`PvpSettingView::failure`] gives the command owner's reason.
    Failed,
    /// The save settled `Done`: a failure line it left can go.
    Saved,
}

/// The reason shown when a refused save carries no text of its own.
const REFUSED_TEXT: &str = "start fights with players: the save was refused";

/// Whether `message` is the error line a failed save of this setting left.
pub fn is_start_fights_error(message: &str) -> bool {
    message.contains(START_FIGHTS_ERROR)
}

/// The value a front end shows for the row. A click shows its new value at
/// once; the view keeps it until the saved value agrees or the click's own
/// command settles, so a slow write does not flip the row back and a failed
/// one never leaves the attempted value on screen. Panel and TUI use this one
/// rule: [`Self::begin`] at the click, [`Self::submitted`] once the command has
/// its request, [`Self::follow`] once per frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PvpSettingView {
    shown: bool,
    /// The last saved value this view adopted: what a failed click shows again.
    saved: bool,
    pending: Option<Pending>,
    /// The command of a click whose saved value was adopted before the command
    /// settled: [`Self::follow`] still reports its settlement, whichever of
    /// [`Self::adopt`] and `follow` a front end calls first.
    unreported: Option<RequestId>,
    /// A click whose saved value was adopted before its command was submitted
    /// (a peer wrote that value between the click and the submission):
    /// [`Self::submitted`] takes the command into `unreported`.
    unsubmitted: bool,
    /// The click whose command last settled failed ([`Self::failure`]).
    failed: Option<RequestId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    value: bool,
    /// The `SetPvpStartFights` command of the click; `None` between the click
    /// and its submission.
    request: Option<RequestId>,
}

impl PvpSettingView {
    /// A view that shows `shown` with no write waiting.
    pub const fn new(shown: bool) -> Self {
        Self {
            shown,
            saved: shown,
            pending: None,
            unreported: None,
            unsubmitted: false,
            failed: None,
        }
    }

    /// The value the row shows.
    pub const fn shown(&self) -> bool {
        self.shown
    }

    /// Whether a click is waiting for its saved value.
    pub const fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The operator set the row to `enabled`: show it now and wait for the save.
    pub fn begin(&mut self, enabled: bool) {
        self.shown = enabled;
        self.unreported = None;
        self.unsubmitted = false;
        self.pending = Some(Pending {
            value: enabled,
            request: None,
        });
    }

    /// The click's command was submitted as `request`: [`Self::follow`] ends
    /// the wait when that command settles, also when [`Self::adopt`] ended the
    /// wait before this call.
    pub fn submitted(&mut self, request: RequestId) {
        if let Some(pending) = self.pending.as_mut() {
            pending.request = Some(request);
        } else if std::mem::take(&mut self.unsubmitted) {
            self.unreported = Some(request);
        }
    }

    /// The operator's click never became a command: show the saved value
    /// again at once.
    pub fn refuse(&mut self) {
        self.pending = None;
        self.unreported = None;
        self.unsubmitted = false;
        self.shown = self.saved;
    }

    /// Adopt the saved value read now (`None` while the store is not ready).
    /// A click whose value the file now holds stops waiting, but its command
    /// stays watched: [`Self::follow`] still reports how it settled, so the
    /// error a failed attempt left goes when this save succeeds. This holds
    /// when the adoption comes before [`Self::submitted`]: that call then
    /// takes the command.
    pub fn adopt(&mut self, saved: Option<bool>) {
        let Some(saved) = saved else {
            return;
        };
        self.saved = saved;
        if let Some(pending) = self.pending.filter(|pending| pending.value == saved) {
            self.unreported = pending.request;
            self.unsubmitted = pending.request.is_none();
            self.pending = None;
        }
        if self.pending.is_none() {
            self.shown = saved;
        }
    }

    /// Once per frame: end the wait of a click whose command has settled, then
    /// adopt the saved value. A failed command shows the saved value again at
    /// once (the failed value is never kept); a settled save that the file no
    /// longer holds (a peer wrote after it) shows the file's value. A report the
    /// history no longer holds counts as settled.
    pub fn follow<Io>(&mut self, core: &OperatorSession<Io>) -> Option<PvpWrite> {
        let watched = match self.pending {
            Some(pending) => pending.request,
            None => self.unreported,
        };
        let outcome = watched.and_then(|request| match core.commands().report(request) {
            Some(report) if !report.is_settled() => None,
            Some(report) => {
                let failed = report
                    .members()
                    .iter()
                    .any(|member| matches!(member.terminal(), Some(Settlement::Failed(_))));
                Some(if failed {
                    self.failed = Some(request);
                    PvpWrite::Failed
                } else {
                    PvpWrite::Saved
                })
            }
            None => Some(PvpWrite::Saved),
        });
        if outcome.is_some() {
            self.pending = None;
            self.unreported = None;
        }
        self.adopt(core.pvp_start_fights());
        if outcome == Some(PvpWrite::Failed) {
            // A cold store gave nothing to adopt: the failed value still goes.
            self.shown = self.saved;
        }
        outcome
    }

    /// The reason the click that last failed gave (`start fights with players:
    /// …`), for the front end's error line.
    pub fn failure<'a, Io>(&self, core: &'a OperatorSession<Io>) -> &'a str {
        self.failed
            .and_then(|request| core.command_failure(request))
            .unwrap_or(REFUSED_TEXT)
    }
}

/// Serializes the tests that write the setting and count its audit lines in
/// the process-wide log: another test's write would otherwise add a line.
#[cfg(test)]
pub(crate) static TEST_AUDIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::settled_for_test;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(label: &str) -> (PathBuf, UiPreferences) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-frontend-pvp-prefs-{}-{label}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let preferences = UiPreferences::open_for_test(dir.clone());
        (dir, preferences)
    }

    #[test]
    fn the_setting_is_off_by_default_and_a_write_keeps_other_members() {
        let _audit = TEST_AUDIT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (dir, prefs) = scratch("roundtrip");
        vault::write_private_file(
            &prefs.path(),
            br#"{"last_focus":"alice","nav":{"allow_teleports":true}}"#,
        )
        .unwrap();
        prefs.reload().unwrap();
        assert_eq!(pvp_start_fights(&prefs).ok(), Some(false), "absent = off");

        settled_for_test(write_pvp_start_fights(&prefs, true)).unwrap();
        assert_eq!(pvp_start_fights(&prefs).ok(), Some(true));
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(saved["pvp"]["start_fights"], true);
        assert_eq!(saved["last_focus"], "alice");
        assert_eq!(saved["nav"]["allow_teleports"], true);

        settled_for_test(write_pvp_start_fights(&prefs, false)).unwrap();
        assert_eq!(pvp_start_fights(&prefs).ok(), Some(false));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_malformed_member_is_an_error_a_caller_reads_as_off() {
        let (dir, prefs) = scratch("malformed");
        vault::write_private_file(&prefs.path(), br#"{"pvp":{"start_fights":"yes"}}"#).unwrap();
        prefs.reload().unwrap();
        assert!(pvp_start_fights(&prefs).is_err());
        let (dir2, prefs2) = scratch("not-object");
        vault::write_private_file(&prefs2.path(), br#"{"pvp":true}"#).unwrap();
        prefs2.reload().unwrap();
        assert!(pvp_start_fights(&prefs2).is_err());
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(dir2).unwrap();
    }

    #[test]
    fn a_click_stays_shown_until_the_saved_value_agrees() {
        let mut view = PvpSettingView::new(false);
        view.adopt(Some(false));
        view.begin(true);
        assert!(view.shown() && view.pending());
        // A read of the old value while the write is queued does not flip it.
        view.adopt(Some(false));
        assert!(view.shown() && view.pending());
        view.adopt(Some(true));
        assert!(view.shown() && !view.pending());
        // A click that never became a command shows the saved value at once,
        // even while the store is cold (nothing to adopt).
        view.begin(false);
        assert!(!view.shown());
        view.refuse();
        assert!(view.shown() && !view.pending());
        view.adopt(Some(true));
        assert!(view.shown() && !view.pending());
        // A cold store keeps what is shown.
        view.adopt(None);
        assert!(view.shown());
    }
}
