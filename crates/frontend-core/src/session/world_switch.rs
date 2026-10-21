//! Switch world (`OperatorCommand::SwitchWorld`): the owner of each
//! member from its world save to its relog.
//!
//! A member first saves the profile's world (the profile form's world
//! field, [`OperatorSession::set_world_preference`]). Only when that save
//! is durable does the slot get the `SwitchWorld` intent, whose turnover
//! logs the bot out and back in through the login queue on the new world
//! (`api::lifecycle`, `Intent::SwitchWorld`). A failed or replaced save
//! never logs the bot out. The member then settles from the relog itself:
//! done once a new session is in game on the requested world, failed when
//! the login fails or lands elsewhere, cancelled when a later Log in, Log
//! out or Remove takes the slot. A running script keeps its run across the
//! turnover and an operator Pause stays paused (the host's session
//! boundary, not this owner, decides that).

use api::identity::{Cancellation, RequestId, SessionKey, Settlement};
use api::lifecycle::{Intent, ParkReason, WorldChoice};
use host_play::LoginLatchReason;

use super::commands::Applied;
use crate::command::{CommandFailure, Completion, FailureKind, Rejection, SkipReason, Subject};
use crate::operations::OperationId;
use crate::views::{FleetRow, Phase};
use crate::OperatorSession;

/// One member of a Switch world command that has not settled yet.
pub(super) struct WorldSwitch {
    request: RequestId,
    subject: Subject,
    name: String,
    world: u16,
    stage: SwitchStage,
}

enum SwitchStage {
    /// The world save is being written.
    Saving(OperationId),
    /// The intent is posted; the relog has not finished. The session the
    /// bot was in when it was posted.
    Relogging(Option<SessionKey>),
}

/// Why `row` cannot move to `world` now, if it cannot: the typed skip and
/// the reason the plan prompt and the result name. `None` admits it.
pub(super) fn refusal(row: Option<&FleetRow>, world: u16) -> Option<(SkipReason, &'static str)> {
    let Some(row) = row.filter(|row| row.loaded()) else {
        return Some((SkipReason::NotLoaded, "not loaded"));
    };
    match row.phase() {
        Phase::Ready if row.world() == Some(world) => {
            Some((SkipReason::AlreadyOnWorld, "already on this world"))
        }
        Phase::Ready => None,
        Phase::Waiting | Phase::Queued => Some((SkipReason::AlreadyLoggingIn, "queued for login")),
        Phase::Preparing | Phase::Connecting | Phase::Loading => {
            Some((SkipReason::AlreadyLoggingIn, "logging in"))
        }
        _ => Some((SkipReason::NotLoggedIn, "not logged in")),
    }
}

impl<Io> OperatorSession<Io> {
    /// The public worlds a bot can switch to, from the bound server's
    /// world list (read once at launch; nothing is fetched). `None` on a
    /// server with one world (local): it offers no world switch.
    pub fn switch_world_choices(&self) -> Option<impl ExactSizeIterator<Item = u16> + '_> {
        let worlds = self.public_worlds.as_ref()?;
        Some(worlds.worlds.iter().map(|world| world.number))
    }

    /// Why no bot can switch to `world` on this server, if none can.
    pub(super) fn switch_world_unavailable(&self, world: u16) -> Option<String> {
        let mut worlds = match self.switch_world_choices() {
            None => return Some("this server has one world".into()),
            Some(worlds) => worlds,
        };
        (!worlds.any(|number| number == world))
            .then(|| format!("world {world} is not on this server's world list"))
    }

    /// Dispatch one admitted member: save its world, then (from
    /// [`Self::advance_world_switches`]) relog it.
    pub(super) fn begin_world_switch(
        &mut self,
        request: RequestId,
        name: &str,
        world: u16,
    ) -> Applied {
        if let Some(why) = self.switch_world_unavailable(world) {
            return Applied::Rejected(Rejection::NotApplicable, Some(why));
        }
        let subject = Subject::Profile(self.profile_identity_or_synthetic(name));
        let row = self
            .fleet_view()
            .rows()
            .iter()
            .find(|row| row.name() == name);
        let refused = match refusal(row, world) {
            Some(refused) => Some(refused),
            None if self.profile_write_pending(name) => {
                Some((SkipReason::ProfileSaving, "profile is being saved"))
            }
            None => None,
        };
        if let Some((reason, why)) = refused {
            let _ = self.commands.book.settle(
                request,
                subject,
                Settlement::Completed(Completion::Skipped(reason)),
                Some(why),
            );
            return Applied::Later;
        }
        let stage = match self.set_world_preference(name, WorldChoice::World(world)) {
            Applied::Operation(op) => SwitchStage::Saving(op),
            // The saved world already is `world` (the bot is elsewhere
            // after a world error or an automatic choice): relog at once.
            Applied::Done => SwitchStage::Relogging(None),
            refused => return refused,
        };
        let mut switch = WorldSwitch {
            request,
            subject,
            name: name.to_owned(),
            world,
            stage,
        };
        let unsettled = match switch.stage {
            SwitchStage::Saving(_) => true,
            SwitchStage::Relogging(_) => self.post_world_switch(&mut switch),
        };
        if unsettled {
            self.commands.world_switches.push(switch);
        }
        Applied::Later
    }

    /// Move every unsettled member on: post the relog of a member whose
    /// save is durable, settle a failed save without logging out, and
    /// settle a relog from the slot. Called by every poll.
    pub(super) fn advance_world_switches(&mut self) {
        if self.commands.world_switches.is_empty() {
            return;
        }
        let mut switches = std::mem::take(&mut self.commands.world_switches);
        switches.retain_mut(|switch| self.advance_world_switch(switch));
        debug_assert!(self.commands.world_switches.is_empty());
        self.commands.world_switches = switches;
    }

    /// One member's step: `false` once it has settled.
    fn advance_world_switch(&mut self, switch: &mut WorldSwitch) -> bool {
        let pending = self
            .commands
            .book
            .report(switch.request)
            .and_then(|report| report.member(switch.subject))
            .is_some_and(|member| member.pending().is_some());
        // A later Log in, Log out or Remove settled it (superseded or
        // retired): nothing more is posted.
        if !pending {
            return false;
        }
        match switch.stage {
            SwitchStage::Saving(op) => {
                let Some(outcome) = self
                    .operations
                    .get(op)
                    .and_then(|report| report.outcome(&switch.name))
                else {
                    self.settle_switch(
                        switch,
                        Settlement::Cancelled(Cancellation::ScopeInvalidated),
                        Some("the world save's report is gone"),
                    );
                    return false;
                };
                if outcome.is_pending() {
                    return true;
                }
                if !outcome.is_completed() {
                    let terminal = super::commands::terminal_of(
                        crate::OperatorCommand::SetWorld(WorldChoice::World(switch.world)),
                        outcome,
                    )
                    .unwrap_or(Settlement::Failed(CommandFailure::Failed(
                        FailureKind::WriteFailed,
                    )));
                    let detail = match outcome.reason() {
                        Some(reason) => format!("world not saved: {reason}"),
                        None => "world not saved".to_owned(),
                    };
                    self.settle_switch(switch, terminal, Some(&detail));
                    return false;
                }
                if !self.members().contains(&switch.name) {
                    self.settle_switch(
                        switch,
                        Settlement::Cancelled(Cancellation::ScopeInvalidated),
                        Some("the bot was removed"),
                    );
                    return false;
                }
                // The bot may have left the game while the save was
                // written: its next login uses the saved world.
                let row = self
                    .fleet_view()
                    .rows()
                    .iter()
                    .find(|row| row.name() == switch.name);
                if let Some((reason, why)) = refusal(row, switch.world) {
                    let detail = format!("world saved; {why}");
                    self.settle_switch(
                        switch,
                        Settlement::Completed(Completion::Skipped(reason)),
                        Some(&detail),
                    );
                    return false;
                }
                self.post_world_switch(switch)
            }
            SwitchStage::Relogging(previous) => self.settle_relog(switch, previous),
        }
    }

    /// Post the member's `SwitchWorld` row: `false` when it settled
    /// instead (no slot, a closed lifetime, a slot the command was not
    /// admitted on). The book is asked first, so a refused member never
    /// reaches the slot's mailbox.
    fn post_world_switch(&mut self, switch: &mut WorldSwitch) -> bool {
        let Some(arm) = self.play.as_ref().and_then(|play| play.arm(&switch.name)) else {
            self.settle_switch(
                switch,
                Settlement::Cancelled(Cancellation::ScopeInvalidated),
                Some("the bot was removed"),
            );
            return false;
        };
        if self
            .commands
            .book
            .may_post(switch.request, switch.subject, arm.slot())
            .is_err()
        {
            // The slot was replaced since admission: this lifetime is not
            // the one the command was for.
            self.settle_switch(
                switch,
                Settlement::Cancelled(Cancellation::ScopeInvalidated),
                Some("the bot's slot restarted"),
            );
            return false;
        }
        let previous = arm.session_key();
        let Some(receipt) = arm.post_intent(Intent::SwitchWorld(switch.world)) else {
            self.settle_switch(
                switch,
                Settlement::Cancelled(Cancellation::ScopeInvalidated),
                Some("the bot was removed"),
            );
            return false;
        };
        // Supersedes the member's older pending Log in / Log out / Relog.
        // The book allowed this post just above, so it records it.
        let recorded =
            self.commands
                .book
                .posted(switch.request, switch.subject, arm.slot(), receipt);
        debug_assert!(recorded.is_ok(), "{recorded:?}");
        self.commands.watch_arm(arm);
        if let Some(play) = self.play.as_ref() {
            play.wake(&switch.name);
        }
        switch.stage = SwitchStage::Relogging(previous);
        true
    }

    /// Settle a posted member from its slot: `false` once settled. Success
    /// and a wrong world are read from the slot's ready session, whose key,
    /// readiness and world the slot reports together; the status row only
    /// carries the failures (a refused world, a startup error, an ended
    /// worker), which belong to no session.
    fn settle_relog(&mut self, switch: &WorldSwitch, previous: Option<SessionKey>) -> bool {
        let Some(arm) = self.play.as_ref().and_then(|play| play.arm(&switch.name)) else {
            self.settle_switch(
                switch,
                Settlement::Cancelled(Cancellation::ScopeInvalidated),
                Some("the bot was removed"),
            );
            return false;
        };
        if let Some((_, world)) = arm
            .ready_session()
            .filter(|(key, _)| Some(*key) != previous)
        {
            let (terminal, detail) = match world {
                Some(world) if world == switch.world => {
                    (Settlement::Completed(Completion::Done), None)
                }
                other => (
                    Settlement::Failed(CommandFailure::Failed(FailureKind::WorldRejected)),
                    Some(match other {
                        Some(world) => format!("logged in on world {world} instead"),
                        None => "logged in without a public world".to_owned(),
                    }),
                ),
            };
            self.settle_switch(switch, terminal, detail.as_deref());
            return false;
        }
        let Some(status) = self.statuses.iter().find(|s| s.username == switch.name) else {
            return true;
        };
        let (terminal, detail) = if arm.login_held_by_error() {
            (
                Settlement::Failed(CommandFailure::Failed(FailureKind::WorldRejected)),
                Some(
                    status
                        .error
                        .clone()
                        .unwrap_or_else(|| "the world was refused".to_owned()),
                ),
            )
        } else if let Some(error) = status.terminal_startup_error() {
            (
                Settlement::Failed(CommandFailure::Failed(FailureKind::LoginFailed)),
                Some(error.to_owned()),
            )
        } else if status.worker_terminal.is_some() {
            (
                Settlement::Failed(CommandFailure::Failed(FailureKind::WorkerEnded)),
                Some(
                    status
                        .error
                        .clone()
                        .unwrap_or_else(|| "worker ended".to_owned()),
                ),
            )
        } else if !arm.login_wanted() && arm.session_key().is_none_or(|key| Some(key) == previous) {
            // No login is owed and no new session began. A Log out this
            // core posted settled the member when it was posted, so what
            // withdrew the login here is the slot's own latch: name it, as
            // the book does (`CommandBook::withdrawn_words`), or say only
            // "cancelled" when the slot names no cause. (A new session that
            // is not ready yet owes no login either; it settles above once
            // ready.)
            let latch = arm.login_latch_reason().map(|reason| match reason {
                LoginLatchReason::OperatorLogout => ParkReason::OperatorLogout,
                LoginLatchReason::PersistedLogout => ParkReason::PersistedLogout,
                LoginLatchReason::RepeatedUnexpectedLogouts { .. } => ParkReason::RepeatGuard,
            });
            (
                Settlement::Cancelled(Cancellation::Superseded),
                crate::command::CommandBook::withdrawn_words(latch).map(str::to_owned),
            )
        } else {
            return true;
        };
        self.settle_switch(switch, terminal, detail.as_deref());
        false
    }

    fn settle_switch(
        &mut self,
        switch: &WorldSwitch,
        terminal: Settlement<Completion, CommandFailure>,
        detail: Option<&str>,
    ) {
        let _ = self
            .commands
            .book
            .settle(switch.request, switch.subject, terminal, detail);
    }
}

#[cfg(test)]
mod tests {
    /// One record per switching bot, only while its switch runs: a second
    /// per-bot copy (a status row, a profile) would not fit.
    #[test]
    fn a_switch_record_stays_small() {
        let size = std::mem::size_of::<super::WorldSwitch>();
        assert!(size <= 96, "{size} bytes per switching bot");
    }
}
