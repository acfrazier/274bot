//! Setting-local feedback for the saved operator settings: which setting
//! has an accepted write still pending, and which one holds a retained
//! failure the front end shows beside it.
//!
//! The book is core state, so closing and reopening a settings window or a
//! popup reconstructs its pending and failed state: dropping the surface
//! cannot swallow a late failure. A failure leaves by the rule of every UI
//! error message (operator 2026-10-23): a later action on the same setting
//! clears it (a new accepted write, or a durable one), and otherwise it
//! expires by itself after [`crate::ERROR_MESSAGE_TIMEOUT`].
//!
//! The book never decides an effect. A setting's effect follows the latest
//! durable snapshot ([`crate::OperatorSession::poll_preferences`]); this
//! book only reports whether a write for it is pending or failed.

use std::time::Instant;

use crate::command::DrawSwitch;
use crate::error_expiry::ERROR_MESSAGE_TIMEOUT;
use crate::nav_prefs::NavPreference;
use crate::session::Notice;

/// A saved operator setting a front end changes through a command or a
/// core-owned quiet write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// A saved draw switch (capture input, sidecar 50 fps, focused 50 fps).
    Draw(DrawSwitch),
    /// One shared navigation preference (including its acknowledgements).
    Nav(NavPreference),
    /// The danger-routing level (the compound grant and routing members).
    DangerRouting,
    /// The session log file switch (and the sink it opens).
    SessionLog,
    /// The remembered map-bake warning choice.
    MapBake,
    /// The Quester Paths source folder and its switch.
    QuesterPaths,
    /// A one-time notice's "don't show again".
    Notice(Notice),
    /// The saved Load order, written by a reorder or quietly by a reconcile.
    ProfileOrder,
}

impl Setting {
    /// Every setting identity, for bounded failure expiry reporting.
    pub(crate) const ALL: [Self; 20] = [
        Self::Draw(DrawSwitch::CaptureInput),
        Self::Draw(DrawSwitch::SidecarFullRate),
        Self::Draw(DrawSwitch::FocusedFullRate),
        Self::Nav(NavPreference::ShowSpecialAreas),
        Self::Nav(NavPreference::PauseScriptOnManualWalkAbort),
        Self::Nav(NavPreference::AllowTeleports),
        Self::Nav(NavPreference::AllowWilderness),
        Self::Nav(NavPreference::AllowBankFetch),
        Self::Nav(NavPreference::AllowDangerZones),
        Self::Nav(NavPreference::SurvivableRouting),
        Self::Nav(NavPreference::ScriptScopeNoticeAck),
        Self::Nav(NavPreference::SurvivableRoutingNoticeAck),
        Self::DangerRouting,
        Self::SessionLog,
        Self::MapBake,
        Self::QuesterPaths,
        Self::Notice(Notice::BackgroundBots),
        Self::Notice(Notice::RenderAllWarning),
        Self::Notice(Notice::QuesterPaths),
        Self::ProfileOrder,
    ];

    /// Stable identity for a setting-event record.
    pub(crate) const fn identity(self) -> &'static str {
        match self {
            Self::Draw(DrawSwitch::CaptureInput) => "capture",
            Self::Draw(DrawSwitch::SidecarFullRate) => "sidecar_50",
            Self::Draw(DrawSwitch::FocusedFullRate) => "focused_50",
            Self::Draw(DrawSwitch::GameRenderer) => "game renderer",
            Self::Draw(DrawSwitch::LiveFullRate) => "live full rate",
            Self::Nav(preference) => preference.setting_identity(),
            Self::DangerRouting => "danger routing",
            Self::SessionLog => "session file setting",
            Self::MapBake => "map bake choice",
            Self::QuesterPaths => "Quest Paths settings",
            Self::Notice(Notice::BackgroundBots) => "background bots notice",
            Self::Notice(Notice::RenderAllWarning) => "render all warning",
            Self::Notice(Notice::QuesterPaths) => "Quest Paths notice",
            Self::ProfileOrder => "profile order",
        }
    }
}

/// What a front end shows beside a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingStatus<'a> {
    /// No write is pending and no failure is retained.
    Saved,
    /// An accepted write has not reached its terminal result.
    Pending,
    /// The retained failure text (a write that was refused or failed, or a
    /// runtime effect of a saved choice that could not start).
    Failed(&'a str),
}

#[derive(Debug)]
struct Failure {
    text: Box<str>,
    since: Instant,
}

#[derive(Debug)]
struct Entry {
    setting: Setting,
    pending: u32,
    failure: Option<Failure>,
}

/// Pending counts and retained failures of every setting. At most one entry
/// per [`Setting`] value, so the book is bounded by the enum.
#[derive(Debug, Default)]
pub(crate) struct SettingBook {
    entries: Vec<Entry>,
}

impl SettingBook {
    fn entry(&mut self, setting: Setting) -> &mut Entry {
        match self.entries.iter().position(|e| e.setting == setting) {
            Some(index) => &mut self.entries[index],
            None => {
                self.entries.push(Entry {
                    setting,
                    pending: 0,
                    failure: None,
                });
                self.entries.last_mut().expect("an entry was just pushed")
            }
        }
    }

    fn find(&self, setting: Setting) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.setting == setting)
    }

    /// A write for `setting` was accepted: it is pending, and the newer
    /// action clears the failure it resolves.
    pub(crate) fn begin(&mut self, setting: Setting) {
        let entry = self.entry(setting);
        entry.pending = entry.pending.saturating_add(1);
        entry.failure = None;
    }

    /// An accepted write for `setting` became durable.
    pub(crate) fn succeed(&mut self, setting: Setting) {
        let entry = self.entry(setting);
        entry.pending = entry.pending.saturating_sub(1);
        entry.failure = None;
    }

    /// An accepted write for `setting` failed or was cancelled.
    pub(crate) fn fail(&mut self, setting: Setting, text: String, now: Instant) {
        let entry = self.entry(setting);
        entry.pending = entry.pending.saturating_sub(1);
        entry.failure = Some(Failure {
            text: text.into(),
            since: now,
        });
    }

    /// A write for `setting` was refused before it was accepted, or its
    /// saved choice could not take effect: no pending count changes.
    pub(crate) fn fail_now(&mut self, setting: Setting, text: String, now: Instant) {
        self.entry(setting).failure = Some(Failure {
            text: text.into(),
            since: now,
        });
    }

    /// Clear a retained failure: the action that resolves it happened.
    pub(crate) fn clear_failure(&mut self, setting: Setting) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.setting == setting) {
            entry.failure = None;
        }
    }

    /// Drop every failure that has stood for the error timeout.
    pub(crate) fn expire(&mut self, now: Instant) {
        for entry in &mut self.entries {
            if entry.failure.as_ref().is_some_and(|failure| {
                now.saturating_duration_since(failure.since) >= ERROR_MESSAGE_TIMEOUT
            }) {
                entry.failure = None;
            }
        }
    }

    pub(crate) fn status(&self, setting: Setting) -> SettingStatus<'_> {
        match self.find(setting) {
            Some(Entry { pending, .. }) if *pending > 0 => SettingStatus::Pending,
            Some(Entry {
                failure: Some(failure),
                ..
            }) => SettingStatus::Failed(&failure.text),
            _ => SettingStatus::Saved,
        }
    }

    /// The retained failure text of `setting`, even while a newer write is
    /// pending (never both: a new accepted write clears the failure).
    pub(crate) fn failure(&self, setting: Setting) -> Option<&str> {
        self.find(setting)
            .and_then(|entry| entry.failure.as_ref())
            .map(|failure| &*failure.text)
    }

    /// When the earliest retained failure expires, for a front end that
    /// sleeps between frames.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.entries
            .iter()
            .filter_map(|entry| entry.failure.as_ref())
            .map(|failure| failure.since + ERROR_MESSAGE_TIMEOUT)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_failure_is_cleared_by_a_new_write_a_success_or_the_timeout() {
        let mut book = SettingBook::default();
        let now = Instant::now();
        let setting = Setting::SessionLog;
        assert_eq!(book.status(setting), SettingStatus::Saved);
        book.begin(setting);
        assert_eq!(book.status(setting), SettingStatus::Pending);
        book.fail(setting, "Not saved: disk".into(), now);
        assert_eq!(
            book.status(setting),
            SettingStatus::Failed("Not saved: disk")
        );
        // The retry is the action that resolves it.
        book.begin(setting);
        assert_eq!(book.status(setting), SettingStatus::Pending);
        assert_eq!(book.failure(setting), None);
        book.succeed(setting);
        assert_eq!(book.status(setting), SettingStatus::Saved);
        // Left alone, the failure expires by itself.
        book.begin(setting);
        book.fail(setting, "Not saved: disk".into(), now);
        book.expire(now + ERROR_MESSAGE_TIMEOUT - Duration::from_millis(1));
        assert!(matches!(book.status(setting), SettingStatus::Failed(_)));
        assert_eq!(
            book.next_expiry(),
            Some(now + ERROR_MESSAGE_TIMEOUT),
            "the front end can sleep until the failure expires"
        );
        book.expire(now + ERROR_MESSAGE_TIMEOUT);
        assert_eq!(book.status(setting), SettingStatus::Saved);
        assert_eq!(book.next_expiry(), None);
    }

    #[test]
    fn the_book_holds_one_entry_per_setting() {
        let mut book = SettingBook::default();
        for _ in 0..100 {
            book.begin(Setting::MapBake);
            book.succeed(Setting::MapBake);
            book.fail_now(Setting::MapBake, "x".into(), Instant::now());
        }
        assert_eq!(book.entries.len(), 1);
        assert!(std::mem::size_of::<Entry>() <= 64, "an entry stays small");
    }
}
