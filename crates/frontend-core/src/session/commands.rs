//! The operator command seam (T3.3; F0 item 5, `command.rs`):
//! [`OperatorSession::submit`] is the one entry point for an operator
//! mutation. The panel, the TUI and [`crate::HeadlessDriver`] all call it,
//! so a headed click and a headless command take the same path.
//!
//! `submit` mints the command identity from the process work service (the
//! composition's one host `RequestIdMinter`), opens the command's report
//! with every member in the [`CommandBook`] under the latched process
//! conditions, dispatches each admitted member to the existing core path,
//! and settles the member as its row says ([`OperatorCommand::settles`]):
//!
//! - `AtDispatch`: at once, from the dispatch result.
//! - `IntentApplied`: the member's slot mailbox receipt is recorded
//!   ([`CommandBook::posted`]) and the slot's `IntentApplied` event settles
//!   it ([`CommandBook::observe`]).
//! - `Owner`: from the owner's report. For the rows that report through the
//!   operation book today (load, remove, script runs, profile writes) that
//!   is the member's operation outcome, read by [`OperatorSession::poll`].
//!
//! Operation reports ([`crate::OperationReport`]) stay what the surfaces
//! show; the command report is the typed, request-correlated record over
//! them.

use std::sync::Arc;
use std::time::Instant;

use api::identity::{Cancellation, RequestId, Scope, Settlement, SlotKey};
use api::lifecycle::{Intent, PostReceipt};
use host_play::SlotArm;
use script::work_admission::{WorkService, WorkStartError};

use super::{OperatorSession, Removal, SAVING};
use crate::command::{
    CommandBook, CommandError, CommandFailure, Completion, FailureKind, OperatorCommand, Rejection,
    Settles, SkipReason, Subject, SubjectKind,
};
use crate::operations::{OperationId, Outcome};
use crate::surface::SlotSurface;

/// Which members a command is for.
#[derive(Debug, Clone, Copy)]
pub enum Targets<'a> {
    /// The application's shared state (an application row).
    Application,
    /// One profile, by name.
    Profile(&'a str),
    /// These profiles, by name, as one command.
    Profiles(&'a [String]),
    /// The selected member.
    Selected,
    /// The row's "all": Load all loads every profile not yet a member, Log
    /// in all every fleet member, Log out all every member and every other
    /// running slot, Stop all every member.
    Everyone,
    /// The profiles the front end's lent fleet marks name
    /// ([`crate::MarkedSelection`]), by name; a mark whose profile is gone
    /// names no member.
    Marked,
}

/// Why [`OperatorSession::submit`] opened no command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubmitError {
    /// The command book refused the command (no members, wrong subject).
    #[error(transparent)]
    Command(#[from] CommandError),
    /// The process work service, which mints command identities, could not
    /// start.
    #[error(transparent)]
    WorkService(#[from] WorkStartError),
    /// Host request identities are exhausted.
    #[error("host request identities are exhausted")]
    IdentityExhausted,
    /// [`Targets::Selected`] with nothing selected, or a target that names
    /// no member for this row.
    #[error("{0:?} names no member")]
    NoTarget(OperatorCommand),
    /// The targets do not fit the row's subject kind.
    #[error("{0:?} cannot take these targets")]
    WrongTargets(OperatorCommand),
    /// A normative row with no current entry point (`SwitchWorld`, per the
    /// F0 census).
    #[error("{0:?} has no current entry point")]
    NoEntryPoint(OperatorCommand),
    /// [`OperatorSession::submit_detached`] was asked for a row that spawns
    /// or removes a slot.
    #[error("{0:?} needs the surface's slot devices")]
    NeedsSurface(OperatorCommand),
}

/// What a row's dispatch did with one member: the member's terminal, or
/// who settles it later. Every arm of the dispatch returns one; nothing
/// settles a member by default.
#[derive(Debug)]
pub(crate) enum Applied {
    /// The effect holds.
    Done,
    /// Nothing was done, for this typed reason.
    Skipped(SkipReason),
    /// The owner refused it at dispatch.
    Rejected(Rejection, Option<String>),
    /// The owner failed it.
    Failed(FailureKind, Option<String>),
    /// The operation book reports the outcome of `0`.
    Operation(OperationId),
    /// The model that took the row settles it from its own result (it
    /// keeps the request id; [`OperatorSession::settle_later`]).
    Later,
    /// A latched process condition already refused the member at
    /// admission: it is settled and is not dispatched.
    NotAdmitted,
}

/// What [`OperatorSession::submit`] did.
#[derive(Debug)]
pub struct Submitted {
    request: RequestId,
    operation: Option<OperationId>,
    removal: Option<Removal>,
    added: usize,
    bulk: Option<crate::BulkReport>,
}

impl Submitted {
    /// The command identity: its report is [`OperatorSession::commands`]`.report(request)`.
    pub fn request(&self) -> RequestId {
        self.request
    }

    /// The operation the surfaces show for this command, when the row
    /// reports through the operation book.
    pub fn operation(&self) -> Option<OperationId> {
        self.operation
    }

    /// The removal's selection effect (Remove only).
    pub fn removal(&self) -> Option<&Removal> {
        self.removal.as_ref()
    }

    /// How many members a Load newly added to the fleet.
    pub fn added(&self) -> usize {
        self.added
    }

    /// The per-row report of a bulk or marked command (Start all, the
    /// Fleet window's marked actions), for the front end's report line.
    pub fn bulk(&self) -> Option<&crate::BulkReport> {
        self.bulk.as_ref()
    }

    /// Take the bulk report.
    pub fn take_bulk(&mut self) -> Option<crate::BulkReport> {
        self.bulk.take()
    }

    pub(crate) fn set_bulk(&mut self, bulk: crate::BulkReport) {
        self.bulk = Some(bulk);
    }
}

/// A profile form's Save, staged for [`OperatorCommand::SaveProfile`] /
/// [`OperatorCommand::CreateProfile`] (`ProfileEditor::save`).
pub(crate) struct StagedProfileWrite {
    profile: vault::Profile,
    rename_from: Option<String>,
    creating: bool,
    mirror: super::ArmMirror,
    label: &'static str,
}

impl StagedProfileWrite {
    pub(crate) fn new(
        profile: vault::Profile,
        rename_from: Option<String>,
        creating: bool,
        mirror: super::ArmMirror,
        label: &'static str,
    ) -> Self {
        Self {
            profile,
            rename_from,
            creating,
            mirror,
            label,
        }
    }
}

/// A guardian edit staged for [`OperatorCommand::SetGuardian`]: the
/// random-event fields, or one hostile random's response.
#[derive(Debug, PartialEq, Eq)]
pub struct GuardianEdit(GuardianEditKind);

#[derive(Debug, PartialEq, Eq)]
enum GuardianEditKind {
    Fields {
        random_events: bool,
        lamp_skill: String,
        lamp_auto: bool,
    },
    Policy {
        event: String,
        policy: crate::profile_draft::GuardianPolicy,
    },
}

impl GuardianEdit {
    /// Incoming random events, the lamp skill and lamp auto-use.
    pub fn fields(random_events: bool, lamp_skill: &str, lamp_auto: bool) -> Self {
        Self(GuardianEditKind::Fields {
            random_events,
            lamp_skill: lamp_skill.to_string(),
            lamp_auto,
        })
    }

    /// One hostile random's response (`event` a [`crate::GUARDIAN_EVENTS`]
    /// key).
    pub fn policy(event: &str, policy: crate::profile_draft::GuardianPolicy) -> Self {
        Self(GuardianEditKind::Policy {
            event: event.to_string(),
            policy,
        })
    }
}

/// A member settled by its operation outcome.
struct OwnedMember {
    request: RequestId,
    command: OperatorCommand,
    subject: Subject,
    name: String,
    op: OperationId,
}

/// One mailbox post made while a command dispatched.
struct Post {
    name: String,
    slot: SlotKey,
    receipt: Option<PostReceipt>,
}

/// The session's command state: the book, the members waiting on an owner
/// report, and the arms whose events settle posted members.
#[derive(Default)]
pub(super) struct CommandDesk {
    book: CommandBook,
    owned: Vec<OwnedMember>,
    /// Application members settled by an operation (the op's one member).
    app_owned: Vec<(RequestId, OperatorCommand, OperationId)>,
    /// Arms with a posted, still pending member; drained for events.
    watched: Vec<Arc<SlotArm>>,
    /// Posts recorded while `recording` is set (reused buffer).
    posts: Vec<Post>,
    recording: bool,
    /// The profile write the next SaveProfile/CreateProfile commits.
    profile_write: Option<StagedProfileWrite>,
    /// The guardian edit the next SetGuardian commits.
    guardian: Option<GuardianEdit>,
    /// Member names of the command being opened (reused buffer).
    names: Vec<String>,
    /// Members of the command being opened (reused buffer).
    targets: Vec<(Subject, Scope)>,
}

impl<Io> OperatorSession<Io> {
    /// The command book: every open command's report and the bounded
    /// settled history.
    pub fn commands(&self) -> &CommandBook {
        &self.commands.book
    }

    /// The owner's refusal or failure text of the first failed member of
    /// `request`, for a surface's error banner.
    pub fn command_failure(&self, request: RequestId) -> Option<&str> {
        self.commands
            .book
            .report(request)?
            .members()
            .iter()
            .find(|member| matches!(member.terminal(), Some(Settlement::Failed(_))))
            .and_then(|member| member.detail())
    }

    /// Stage the profile write the next SaveProfile / CreateProfile
    /// commits (a profile form's Save).
    pub(crate) fn stage_profile_write(&mut self, write: StagedProfileWrite) {
        self.commands.profile_write = Some(write);
    }

    /// Create the profile `username` with `password`, a fresh uid and
    /// default settings, through [`OperatorCommand::CreateProfile`] (the
    /// TUI's first-run `--user` profile).
    pub fn create_profile_named(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<Submitted, SubmitError> {
        let uid = self.vault.as_ref().map_or(274_000_001, crate::fresh_uid);
        let profile = vault::Profile {
            username: username.to_string(),
            password: password.into(),
            uid,
            settings: vault::ProfileSettings::default(),
        };
        self.stage_profile_write(StagedProfileWrite::new(
            profile,
            None,
            true,
            super::ArmMirror::none(),
            "profile",
        ));
        self.submit_detached(OperatorCommand::CreateProfile, Targets::Application)
    }

    /// Stage the guardian edit the next [`OperatorCommand::SetGuardian`]
    /// commits.
    pub fn stage_guardian(&mut self, edit: GuardianEdit) {
        self.commands.guardian = Some(edit);
    }

    /// The name Load all prefers to select when nothing is selected (the
    /// surface's remembered last focus). A staged input of `Load`
    /// [`Targets::Everyone`].
    pub fn set_focus_hint(&mut self, last: Option<&str>) {
        self.focus_hint = last.map(str::to_string);
    }

    /// Issue `command` for `targets`: the one entry point for an operator
    /// mutation. See the module documentation for how members settle.
    pub fn submit<S: SlotSurface<Io = Io>>(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
        surface: &mut S,
    ) -> Result<Submitted, SubmitError> {
        self.submit_at(command, targets, Instant::now(), surface)
    }

    /// [`Self::submit`] at `now`: the start of a Remove's clean-logout
    /// window ([`super::SLOT_REMOVE_TIMEOUT`]).
    pub fn submit_at<S: SlotSurface<Io = Io>>(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
        now: Instant,
        surface: &mut S,
    ) -> Result<Submitted, SubmitError> {
        if matches!(command, OperatorCommand::SwitchWorld(_)) {
            return Err(SubmitError::NoEntryPoint(command));
        }
        {
            let models = surface.models();
            self.resolve_targets(command, targets, models.marks.as_deref())?;
        }
        let request = self.open_command(command)?;
        let mut submitted = Submitted {
            request,
            operation: None,
            removal: None,
            added: 0,
            bulk: None,
        };
        self.commands.posts.clear();
        self.commands.recording = command.settles() == Settles::IntentApplied;
        self.dispatch(command, targets, now, surface, &mut submitted);
        self.commands.recording = false;
        self.record_posts(request, command);
        if let Some(op) = submitted.operation {
            if command.settles() == Settles::Owner {
                self.own_members(request, command, op);
            }
        }
        debug_assert!(
            command.settles() != Settles::AtDispatch
                || self
                    .commands
                    .book
                    .report(request)
                    .is_none_or(|report| report.members().iter().all(|m| m.pending().is_none())),
            "{command:?} settles at dispatch, and its dispatch left a member pending"
        );
        self.drain_watched();
        self.settle_owned();
        Ok(submitted)
    }

    /// [`Self::submit`] for a row that never attaches or releases a slot
    /// device (Log out, Stop, Pause/Resume, Select, profile writes, the
    /// application rows), from code that holds no surface. A row that
    /// spawns or removes a slot is refused with
    /// [`SubmitError::NeedsSurface`].
    pub fn submit_detached(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<Submitted, SubmitError> {
        if needs_surface(command) {
            return Err(SubmitError::NeedsSurface(command));
        }
        self.submit_at(
            command,
            targets,
            Instant::now(),
            &mut Detached(std::marker::PhantomData),
        )
    }

    /// Fill `commands.names` and `commands.targets` for `command`.
    fn resolve_targets(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
        marks: Option<&crate::MarkedSelection>,
    ) -> Result<(), SubmitError> {
        let mut names = std::mem::take(&mut self.commands.names);
        names.clear();
        match (command.subject(), targets) {
            (SubjectKind::Application, Targets::Application) => {}
            (SubjectKind::Application, _) | (SubjectKind::Profile, Targets::Application) => {
                self.commands.names = names;
                return Err(SubmitError::WrongTargets(command));
            }
            (SubjectKind::Profile, Targets::Profile(name)) => names.push(name.to_string()),
            (SubjectKind::Profile, Targets::Profiles(list)) => {
                for name in list {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
            }
            (SubjectKind::Profile, Targets::Selected) => {
                if let Some(name) = self.selected.clone() {
                    names.push(name);
                }
            }
            (SubjectKind::Profile, Targets::Everyone) => self.everyone(command, &mut names),
            (SubjectKind::Profile, Targets::Marked) => {
                if let Some(marks) = marks {
                    for (name, identity) in crate::selection::profile_rows(self) {
                        if marks.contains(identity) && !names.contains(&name) {
                            names.push(name);
                        }
                    }
                    names.sort();
                }
            }
        }
        let mut targets = std::mem::take(&mut self.commands.targets);
        targets.clear();
        if command.subject() == SubjectKind::Application {
            targets.push((Subject::Application, Scope::Process));
        } else {
            for name in &names {
                let subject = Subject::Profile(self.profile_identity_or_synthetic(name));
                let scope = self
                    .play
                    .as_ref()
                    .and_then(|play| play.arm(name))
                    .map_or(Scope::Process, |arm| Scope::Slot(arm.slot()));
                targets.push((subject, scope));
            }
        }
        let empty = targets.is_empty();
        self.commands.names = names;
        self.commands.targets = targets;
        if empty {
            return Err(SubmitError::NoTarget(command));
        }
        Ok(())
    }

    /// The members of a row's "all".
    fn everyone(&self, command: OperatorCommand, names: &mut Vec<String>) {
        match command {
            OperatorCommand::Load => {
                for name in self.profile_names() {
                    if !self.fleet.members().contains(&name) && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            OperatorCommand::Logout | OperatorCommand::Quit => {
                names.extend(self.fleet.members().iter().cloned());
                if let Some(play) = self.play.as_ref() {
                    for status in play.statuses() {
                        if !names.contains(&status.username) {
                            names.push(status.username);
                        }
                    }
                }
            }
            _ => names.extend(self.fleet.members().iter().cloned()),
        }
    }

    /// Mint the identity and open the report with the resolved members.
    fn open_command(&mut self, command: OperatorCommand) -> Result<RequestId, SubmitError> {
        let shared = WorkService::shared()?;
        let mut service = shared.lock();
        let request = service
            .mint_identity(Scope::Process, None)
            .map_err(|_| SubmitError::IdentityExhausted)?
            .job();
        let conditions = service.conditions();
        self.commands
            .book
            .open(request, command, &self.commands.targets, &conditions)?;
        Ok(request)
    }

    /// Whether the book admitted the application member of `request` as
    /// Pending (a latched condition did not refuse it at admission).
    pub(super) fn app_admitted(&self, request: RequestId) -> bool {
        self.commands
            .book
            .report(request)
            .and_then(|report| report.member(Subject::Application))
            .is_some_and(|member| member.pending().is_some())
    }

    /// Whether the book admitted `name`'s member as Pending (a member a
    /// latched condition refused is already settled and is not dispatched).
    fn admitted(&self, request: RequestId, name: &str) -> bool {
        let subject = Subject::Profile(self.profile_identity_or_synthetic(name));
        self.commands
            .book
            .report(request)
            .and_then(|report| report.member(subject))
            .is_some_and(|member| member.pending().is_some())
    }

    /// The admitted members of the command being dispatched.
    pub(super) fn admitted_names(&self, request: RequestId) -> Vec<String> {
        self.commands
            .names
            .iter()
            .filter(|name| self.admitted(request, name))
            .cloned()
            .collect()
    }

    fn dispatch<S: SlotSurface<Io = Io>>(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
        now: Instant,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let request = submitted.request;
        let application_admitted = command.subject() == SubjectKind::Application
            && self
                .commands
                .book
                .report(request)
                .and_then(|report| report.member(Subject::Application))
                .is_some_and(|member| member.pending().is_some());
        match command {
            OperatorCommand::Load => {
                let names = self.admitted_names(request);
                let op = self.operations.open(crate::operations::ActionKind::Load);
                for name in &names {
                    if self.load_member(op, name, surface) {
                        submitted.added += 1;
                    }
                }
                if matches!(targets, Targets::Everyone) && self.selected.is_none() {
                    let hint = self.focus_hint.clone();
                    if let Some(name) = super::pick_focus(self.members(), hint.as_deref()) {
                        self.select(&name);
                    }
                }
                submitted.operation = Some(op);
            }
            OperatorCommand::Remove => {
                let names = self.admitted_names(request);
                for name in &names {
                    let removal = self.remove(name, now, surface);
                    self.own_member(request, command, name, removal.op);
                    submitted.removal = Some(removal);
                }
            }
            OperatorCommand::Login => {
                let names = self.admitted_names(request);
                if !names.is_empty() {
                    submitted.operation = Some(self.login_members(&names, surface));
                }
            }
            OperatorCommand::Logout => {
                let names = self.admitted_names(request);
                if !names.is_empty() {
                    submitted.operation = Some(self.logout_members(&names));
                }
            }
            OperatorCommand::RelogNow => {
                for name in self.admitted_names(request) {
                    let op = self.request_memory_relog(&name, surface);
                    submitted.operation = Some(op);
                }
            }
            OperatorCommand::SetAutoLogin(on) => {
                for name in self.admitted_names(request) {
                    let result = self.set_auto_login(&name, on);
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    self.owner_result(request, command, &name, result);
                }
            }
            OperatorCommand::SetMemoryMode(mode) => {
                let lowmem = mode == api::lifecycle::MemoryMode::Low;
                for name in self.admitted_names(request) {
                    let result = self.set_memory_mode(&name, lowmem);
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    self.owner_result(request, command, &name, result);
                }
            }
            OperatorCommand::Select => {
                for name in self.admitted_names(request) {
                    self.select(&name);
                    self.settle_member(request, command, &name, Applied::Done, submitted);
                }
            }
            OperatorCommand::PauseScript | OperatorCommand::ResumeScript => {
                for name in self.admitted_names(request) {
                    // Today's toggle: Resume when Paused, otherwise Pause
                    // (the run owner settles a Pause with no running script
                    // as skipped).
                    let paused = self
                        .play
                        .as_ref()
                        .is_some_and(|play| play.script_state(&name) == script::RunState::Paused);
                    let applies = (command == OperatorCommand::ResumeScript) == paused;
                    if applies {
                        let op = self.toggle_pause(&name);
                        self.own_member(request, command, &name, op);
                    } else {
                        let reason = if command == OperatorCommand::PauseScript {
                            SkipReason::NoRunningScript
                        } else {
                            SkipReason::NoScript
                        };
                        self.settle_name(
                            request,
                            &name,
                            Settlement::Completed(Completion::Skipped(reason)),
                            None,
                        );
                    }
                }
            }
            OperatorCommand::StopScript => {
                let names = self.admitted_names(request);
                match (targets, names.as_slice()) {
                    // One member: today's single Stop, which settles when the
                    // run owner reports the slot idle.
                    (Targets::Profile(_) | Targets::Selected, [name]) => {
                        submitted.operation = Some(self.stop_script(name));
                    }
                    (_, []) => {}
                    _ => submitted.operation = Some(self.stop_scripts(&names).0),
                }
            }
            OperatorCommand::SaveProfile | OperatorCommand::CreateProfile => {
                let admitted = if command == OperatorCommand::CreateProfile {
                    application_admitted
                } else {
                    !self.admitted_names(request).is_empty()
                };
                let staged = self.commands.profile_write.take();
                match (admitted, staged) {
                    (true, Some(write)) => {
                        let result = match write.rename_from.as_deref() {
                            Some(old) => {
                                let old = old.to_string();
                                self.rename_profile(&old, write.profile, write.mirror, write.label)
                            }
                            None if write.creating => {
                                self.create_profile(write.profile, write.mirror, write.label)
                            }
                            None => self.save_profile(write.profile, write.mirror, write.label),
                        };
                        self.settle_form_write(request, command, result, submitted);
                    }
                    (true, None) => self.settle_form_write(
                        request,
                        command,
                        Err("nothing staged to save".to_string()),
                        submitted,
                    ),
                    (false, _) => {}
                }
            }
            OperatorCommand::DeleteProfile => {
                for name in self.admitted_names(request) {
                    match self.vault_remove(&name) {
                        Ok(Some(op)) => {
                            submitted.operation = Some(op);
                            self.own_member(request, command, &name, op);
                        }
                        Ok(None) => self.settle_name(
                            request,
                            &name,
                            Settlement::Completed(Completion::Skipped(
                                SkipReason::ProfileUnavailable,
                            )),
                            None,
                        ),
                        Err(reason) => self.owner_result(request, command, &name, Err(reason)),
                    }
                }
            }
            OperatorCommand::SetGuardian => {
                let edit = self.commands.guardian.take();
                for name in self.admitted_names(request) {
                    let result = match edit.as_ref().map(|edit| &edit.0) {
                        Some(GuardianEditKind::Fields {
                            random_events,
                            lamp_skill,
                            lamp_auto,
                        }) => {
                            self.set_random_settings(&name, *random_events, lamp_skill, *lamp_auto)
                        }
                        Some(GuardianEditKind::Policy { event, policy }) => {
                            self.set_random_event_policy(&name, event, *policy)
                        }
                        None => Err("random: nothing staged".to_string()),
                    };
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    self.owner_result(request, command, &name, result);
                }
            }
            OperatorCommand::SetRaster(raster) => {
                for name in self.admitted_names(request) {
                    let result = self.set_profile_raster(&name, raster);
                    self.optional_write(request, command, &name, result, submitted);
                }
            }
            OperatorCommand::MarkTutorialSkipped => {
                for name in self.admitted_names(request) {
                    let result = self.record_tutorial_state(&name, true);
                    self.optional_write(request, command, &name, result, submitted);
                }
            }
            OperatorCommand::SetWorld(world) => {
                for name in self.admitted_names(request) {
                    let applied = self.set_world_preference(&name, world);
                    self.settle_member(request, command, &name, applied, submitted);
                }
            }
            OperatorCommand::Quit => {
                let names = self.admitted_names(request);
                self.quit(request, &names);
            }
            OperatorCommand::SwitchWorld(_) => {
                unreachable!("SwitchWorld is refused with NoEntryPoint before a command opens")
            }
            OperatorCommand::SetRenderAll(all) => {
                let applied = if application_admitted {
                    self.set_capacity_render(if all {
                        crate::capacity::RenderPolicyLabel::RenderAll
                    } else {
                        crate::capacity::RenderPolicyLabel::OnlySelected
                    });
                    Applied::Done
                } else {
                    Applied::NotAdmitted
                };
                self.settle_app(request, command, applied, submitted);
            }
            // --- Vault ---
            OperatorCommand::UnlockVault => {
                self.vault_row(super::vault_rows::VaultRow::Unlock, request, application_admitted, submitted)
            }
            OperatorCommand::CreateVault => {
                self.vault_row(super::vault_rows::VaultRow::Create, request, application_admitted, submitted)
            }
            OperatorCommand::ResetVault => {
                self.vault_row(super::vault_rows::VaultRow::Reset, request, application_admitted, submitted)
            }
            // --- Fleet view, marks, MultiBox ---
            OperatorCommand::SetFleetFilter => {
                let row = super::fleet::FleetRow::Filter;
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetFleetSort(order) => {
                let row = super::fleet::FleetRow::Sort(order);
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetMark(on) => {
                let row = super::fleet::FleetRow::Mark(on);
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::MarkAll => {
                let row = super::fleet::FleetRow::MarkAll;
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::ClearMarks => {
                let row = super::fleet::FleetRow::ClearMarks;
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetMultiBox(on) => {
                let row = super::fleet::FleetRow::MultiBox(on);
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            // --- Preferences and notices ---
            OperatorCommand::SetDraw(switch, on) => {
                let row = super::prefs::PrefRow::Draw(switch, on);
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetNavPreference(pref, on) => {
                let row = super::prefs::PrefRow::Nav(pref, on);
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetDangerLevel(level) => {
                let row = super::prefs::PrefRow::Danger(level);
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetSessionLog(on) => {
                let row = super::prefs::PrefRow::SessionLog(on);
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetMapBakeChoice(choice) => {
                let row = super::prefs::PrefRow::MapBake(choice);
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetQuesterPaths => {
                let row = super::prefs::PrefRow::QuesterPaths;
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::ReloadQuesterPaths => {
                let row = super::prefs::PrefRow::ReloadQuesterPaths;
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::AcknowledgeNotice => {
                let row = super::prefs::PrefRow::AcknowledgeNotice;
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SaveLog => {
                let row = super::prefs::PrefRow::SaveLog;
                self.pref_row(row, request, application_admitted, surface, submitted)
            }
            // --- Map, movement and game input ---
            OperatorCommand::WalkTo => {
                self.map_row(super::map::MapRow::WalkTo, request, surface, submitted)
            }
            OperatorCommand::GroupWalk => {
                self.map_row(super::map::MapRow::GroupWalk, request, surface, submitted)
            }
            OperatorCommand::Teleport => {
                self.map_row(super::map::MapRow::Teleport, request, surface, submitted)
            }
            OperatorCommand::DebugCommand => {
                self.map_row(super::map::MapRow::DebugCommand, request, surface, submitted)
            }
            OperatorCommand::GameInput => {
                self.map_row(super::map::MapRow::GameInput, request, surface, submitted)
            }
            OperatorCommand::OpenMapDemand => {
                self.map_row(super::map::MapRow::OpenDemand, request, surface, submitted)
            }
            OperatorCommand::ReleaseMapDemand => {
                self.map_row(super::map::MapRow::ReleaseDemand, request, surface, submitted)
            }
            OperatorCommand::BakeMapNow => {
                self.map_row(super::map::MapRow::BakeNow, request, surface, submitted)
            }
            OperatorCommand::DeclineMapBake => {
                self.map_row(super::map::MapRow::DeclineBake, request, surface, submitted)
            }
            // --- Script runs and the script coordinator ---
            OperatorCommand::StartScript => {
                let row = super::scripts::ScriptRow::Start;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::CancelQueuedStart => {
                let row = super::scripts::ScriptRow::CancelQueuedStart;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::PickBrowseCard => {
                let row = super::scripts::ScriptRow::PickBrowseCard;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::SelectScriptCard => {
                let row = super::scripts::ScriptRow::SelectScriptCard;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::PrepareScriptCard => {
                let row = super::scripts::ScriptRow::PrepareScriptCard;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::AssignScript => {
                let row = super::scripts::ScriptRow::Assign;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::AssignAndRestart => {
                let row = super::scripts::ScriptRow::AssignAndRestart;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::SaveScriptSettings(kind) => {
                let row = super::scripts::ScriptRow::SaveSettings(kind);
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::SaveLegacyScriptSettings => {
                let row = super::scripts::ScriptRow::SaveLegacySettings;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::EditParameterText => {
                let row = super::scripts::ScriptRow::EditParameterText;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::DiscardParameterText => {
                let row = super::scripts::ScriptRow::DiscardParameterText;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::ApplySettingsToMarked(kind) => {
                let row = super::scripts::ScriptRow::ApplySettingsToMarked(kind);
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::CancelSettingsSync => {
                let row = super::scripts::ScriptRow::CancelSettingsSync;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::RestartForSettings => {
                let row = super::scripts::ScriptRow::RestartForSettings;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::DeferSettingsRestart => {
                let row = super::scripts::ScriptRow::DeferSettingsRestart;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::LoadScriptFile => {
                let row = super::scripts::ScriptRow::LoadScriptFile;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::ReloadScripts => {
                let row = super::scripts::ScriptRow::Reload;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::ConfirmReload => {
                let row = super::scripts::ScriptRow::ConfirmReload;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::CancelReload => {
                let row = super::scripts::ScriptRow::CancelReload;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::ImportCatalog => {
                let row = super::scripts::ScriptRow::ImportCatalog;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::TranspileAll => {
                let row = super::scripts::ScriptRow::TranspileAll;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::DeferCatalogImport => {
                let row = super::scripts::ScriptRow::DeferCatalogImport;
                self.script_row(row, targets, request, surface, submitted)
            }
            OperatorCommand::ScriptPaintInput => {
                let row = super::scripts::ScriptRow::PaintInput;
                self.script_row(row, targets, request, surface, submitted)
            }
            // --- The profile form and the rs2b0t profile import ---
            OperatorCommand::OpenProfileForm => {
                self.form_row(super::form::FormRow::Open, request, surface, submitted)
            }
            OperatorCommand::OpenNewProfileForm => {
                self.form_row(super::form::FormRow::OpenNew, request, surface, submitted)
            }
            OperatorCommand::EditProfileDraft(field) => {
                self.form_row(super::form::FormRow::Edit(field), request, surface, submitted)
            }
            OperatorCommand::DiscardProfileEdits => {
                self.form_row(super::form::FormRow::Discard, request, surface, submitted)
            }
            OperatorCommand::KeepEditing => {
                self.form_row(super::form::FormRow::KeepEditing, request, surface, submitted)
            }
            OperatorCommand::CloseProfileForm(close) => {
                self.form_row(super::form::FormRow::Close(close), request, surface, submitted)
            }
            OperatorCommand::EditProfileImportPath => {
                let row = super::form::FormRow::ImportPath;
                self.form_row(row, request, surface, submitted)
            }
            OperatorCommand::PreviewProfileImport => {
                let row = super::form::FormRow::ImportPreview;
                self.form_row(row, request, surface, submitted)
            }
            OperatorCommand::SetProfileImportPolicy(policy) => {
                let row = super::form::FormRow::ImportPolicy(policy);
                self.form_row(row, request, surface, submitted)
            }
            OperatorCommand::SetProfileImportIncludeUnusable(on) => {
                let row = super::form::FormRow::ImportIncludeUnusable(on);
                self.form_row(row, request, surface, submitted)
            }
            OperatorCommand::ReturnToProfileImportPath => {
                let row = super::form::FormRow::ImportBack;
                self.form_row(row, request, surface, submitted)
            }
            OperatorCommand::SetProfileImportDeletePrompt(on) => {
                let row = super::form::FormRow::ImportDeletePrompt(on);
                self.form_row(row, request, surface, submitted)
            }
            OperatorCommand::DeleteProfileImportExport => {
                let row = super::form::FormRow::ImportDeleteExport;
                self.form_row(row, request, surface, submitted)
            }
            // --- The loadout form ---
            OperatorCommand::SelectLoadout => {
                let row = super::loadouts::LoadoutRow::Select;
                self.loadout_row(row, request, surface, submitted)
            }
            OperatorCommand::EditLoadoutDraft(field) => {
                let row = super::loadouts::LoadoutRow::Edit(field);
                self.loadout_row(row, request, surface, submitted)
            }
            OperatorCommand::CopyEquipmentToLoadout => {
                let row = super::loadouts::LoadoutRow::CopyEquipment;
                self.loadout_row(row, request, surface, submitted)
            }
            OperatorCommand::CreateLoadout => {
                let row = super::loadouts::LoadoutRow::Create;
                self.loadout_row(row, request, surface, submitted)
            }
            OperatorCommand::DuplicateLoadout => {
                let row = super::loadouts::LoadoutRow::Duplicate;
                self.loadout_row(row, request, surface, submitted)
            }
            OperatorCommand::SaveLoadout => {
                let row = super::loadouts::LoadoutRow::Save;
                self.loadout_row(row, request, surface, submitted)
            }
            OperatorCommand::DeleteLoadout => {
                let row = super::loadouts::LoadoutRow::Delete;
                self.loadout_row(row, request, surface, submitted)
            }
        }
    }

    /// Settle the application member of `request` as `applied` says.
    pub(super) fn settle_app(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        applied: Applied,
        submitted: &mut Submitted,
    ) {
        let terminal = match applied {
            Applied::NotAdmitted => return,
            Applied::Operation(op) => {
                submitted.operation = Some(op);
                self.commands.app_owned.push((request, command, op));
                return;
            }
            Applied::Later => return,
            Applied::Done => (Settlement::Completed(Completion::Done), None),
            Applied::Skipped(reason) => (Settlement::Completed(Completion::Skipped(reason)), None),
            Applied::Rejected(rejection, detail) => {
                (Settlement::Failed(CommandFailure::Rejected(rejection)), detail)
            }
            Applied::Failed(kind, detail) => {
                (Settlement::Failed(CommandFailure::Failed(kind)), detail)
            }
        };
        let _ = self.commands.book.settle(
            request,
            Subject::Application,
            terminal.0,
            terminal.1.as_deref(),
        );
    }

    /// Settle `name`'s member of `request` as `applied` says.
    pub(super) fn settle_member(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        name: &str,
        applied: Applied,
        submitted: &mut Submitted,
    ) {
        let (terminal, detail) = match applied {
            Applied::NotAdmitted | Applied::Later => return,
            Applied::Operation(op) => {
                submitted.operation = Some(op);
                self.own_member(request, command, name, op);
                return;
            }
            Applied::Done => (Settlement::Completed(Completion::Done), None),
            Applied::Skipped(reason) => (Settlement::Completed(Completion::Skipped(reason)), None),
            Applied::Rejected(rejection, detail) => {
                (Settlement::Failed(CommandFailure::Rejected(rejection)), detail)
            }
            Applied::Failed(kind, detail) => {
                (Settlement::Failed(CommandFailure::Failed(kind)), detail)
            }
        };
        self.settle_name(request, name, terminal, detail.as_deref());
    }

    /// Settle a member the owner's result reports later (a model that
    /// holds the request id: a catalog import, a map demand, a reload).
    pub(crate) fn settle_later(
        &mut self,
        request: RequestId,
        subject: Subject,
        terminal: Settlement<Completion, CommandFailure>,
        detail: Option<&str>,
    ) {
        let _ = self.commands.book.settle(request, subject, terminal, detail);
    }

    /// Quit: shut every member's slot down and close the play session.
    /// Each member completes when its slot retires (`slot_retired`); a
    /// member with no slot left already has nothing to retire.
    fn quit(&mut self, request: RequestId, names: &[String]) {
        let mut slots = Vec::with_capacity(names.len());
        for name in names {
            match self.play.as_ref().and_then(|play| play.arm(name)) {
                Some(arm) => {
                    self.post(name, &arm, Intent::ProcessShutdown);
                    slots.push(arm.slot());
                }
                None => self.settle_name(
                    request,
                    name,
                    Settlement::Completed(Completion::Done),
                    None,
                ),
            }
        }
        self.close_play();
        for slot in slots {
            self.commands.book.slot_retired(slot);
        }
    }

    /// Persist `name`'s world preference for its next handshake (the
    /// profile form's world field), mirrored onto a running slot's arm.
    fn set_world_preference(
        &mut self,
        name: &str,
        world: api::lifecycle::WorldChoice,
    ) -> Applied {
        let Some(vault) = self.vault.as_ref() else {
            return Applied::Rejected(Rejection::NoHost, Some("world: vault locked".into()));
        };
        let Some(mut profile) = vault.get(name).cloned() else {
            return Applied::Rejected(
                Rejection::UnknownProfile,
                Some(format!("world: no profile {name}")),
            );
        };
        let world = match world {
            api::lifecycle::WorldChoice::Automatic => None,
            api::lifecycle::WorldChoice::World(number) => Some(number),
        };
        if profile.settings.world == world {
            return Applied::Done;
        }
        profile.settings.world = world;
        match self.save_profile(profile, super::ArmMirror::remember(None), "world") {
            Ok(op) => Applied::Operation(op),
            Err(reason) => Applied::Rejected(rejection(&reason), Some(reason)),
        }
    }

    /// Settle a profile-form write: its operation's outcome, or its refusal.
    fn settle_form_write(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        result: Result<OperationId, String>,
        submitted: &mut Submitted,
    ) {
        if command.subject() == SubjectKind::Application {
            match result {
                Ok(op) => {
                    submitted.operation = Some(op);
                    self.commands.app_owned.push((request, command, op));
                }
                Err(reason) => {
                    let terminal = Settlement::Failed(CommandFailure::Rejected(rejection(&reason)));
                    let _ = self.commands.book.settle(
                        request,
                        Subject::Application,
                        terminal,
                        Some(&reason),
                    );
                }
            }
            return;
        }
        submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
        let names = std::mem::take(&mut self.commands.names);
        if let Some(name) = names.first() {
            self.owner_result(request, command, name, result);
        }
        self.commands.names = names;
    }

    /// A write that may have nothing to do (an unknown profile, an
    /// unchanged value): `Ok(None)` completes the member at once.
    fn optional_write(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        name: &str,
        result: Result<Option<OperationId>, String>,
        submitted: &mut Submitted,
    ) {
        match result {
            Ok(Some(op)) => {
                submitted.operation = Some(op);
                self.own_member(request, command, name, op);
            }
            Ok(None) => {
                self.settle_name(request, name, Settlement::Completed(Completion::Done), None)
            }
            Err(reason) => self.owner_result(request, command, name, Err(reason)),
        }
    }

    /// Track every admitted member of `request` under `op`.
    fn own_members(&mut self, request: RequestId, command: OperatorCommand, op: OperationId) {
        let names = std::mem::take(&mut self.commands.names);
        for name in &names {
            self.own_member(request, command, name, op);
        }
        self.commands.names = names;
    }

    fn own_member(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        name: &str,
        op: OperationId,
    ) {
        let subject = Subject::Profile(self.profile_identity_or_synthetic(name));
        let pending = self
            .commands
            .book
            .report(request)
            .and_then(|report| report.member(subject))
            .is_some_and(|member| member.pending().is_some());
        let tracked = self
            .commands
            .owned
            .iter()
            .any(|owned| owned.request == request && owned.subject == subject);
        if pending && !tracked {
            self.commands.owned.push(OwnedMember {
                request,
                command,
                subject,
                name: name.to_string(),
                op,
            });
        }
    }

    /// An owner path that returns its operation or a dispatch refusal.
    fn owner_result(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        name: &str,
        result: Result<OperationId, String>,
    ) {
        match result {
            Ok(op) => self.own_member(request, command, name, op),
            Err(reason) => {
                let terminal = Settlement::Failed(CommandFailure::Rejected(rejection(&reason)));
                self.settle_name(request, name, terminal, Some(&reason));
            }
        }
    }

    fn settle_name(
        &mut self,
        request: RequestId,
        name: &str,
        terminal: Settlement<Completion, CommandFailure>,
        detail: Option<&str>,
    ) {
        let subject = Subject::Profile(self.profile_identity_or_synthetic(name));
        // A member already settled by a condition or a slot event keeps
        // its one terminal.
        let _ = self
            .commands
            .book
            .settle(request, subject, terminal, detail);
    }

    /// Record the dispatch's mailbox receipts. A member admitted on its
    /// slot is posted and settles on that slot's `IntentApplied`; a member
    /// whose slot did not exist at admission (the dispatch spawned it)
    /// settles at once, because the post applies synchronously. A member
    /// with no post was not dispatched to a slot: it settles from its
    /// operation outcome instead.
    fn record_posts(&mut self, request: RequestId, command: OperatorCommand) {
        if command.settles() != Settles::IntentApplied {
            return;
        }
        let posts = std::mem::take(&mut self.commands.posts);
        for post in &posts {
            let subject = Subject::Profile(self.profile_identity_or_synthetic(&post.name));
            let Some(receipt) = post.receipt else {
                continue;
            };
            let admitted_on_slot = self
                .commands
                .book
                .report(request)
                .and_then(|report| report.member(subject))
                .is_some_and(|member| member.scope() == Scope::Slot(post.slot));
            if admitted_on_slot {
                if self
                    .commands
                    .book
                    .posted(request, subject, post.slot, receipt)
                    .is_ok()
                {
                    if let Some(arm) = self.play.as_ref().and_then(|play| play.arm(&post.name)) {
                        if !self.commands.watched.iter().any(|w| Arc::ptr_eq(w, &arm)) {
                            self.commands.watched.push(arm);
                        }
                    }
                }
            } else {
                let _ = self.commands.book.settle(
                    request,
                    subject,
                    Settlement::Completed(Completion::Done),
                    None,
                );
            }
        }
        self.commands.posts = posts;
        // Members with no post settle from their operation outcome.
        let names = std::mem::take(&mut self.commands.names);
        let op = self.operations_last_for(command);
        for name in &names {
            let posted = self.commands.posts.iter().any(|post| &post.name == name);
            if !posted {
                if let Some(op) = op {
                    self.own_member(request, command, name, op);
                }
            }
        }
        self.commands.names = names;
    }

    /// The operation the last dispatch of a mailbox row opened.
    fn operations_last_for(&self, command: OperatorCommand) -> Option<OperationId> {
        let action = command.action_kind()?;
        self.operations
            .last()
            .filter(|report| report.action() == action)
            .map(|report| report.id())
    }

    /// Record a mailbox post made by a dispatch (see [`Self::submit`]).
    pub(super) fn post(&mut self, name: &str, arm: &SlotArm, row: Intent) {
        let receipt = arm.post_intent(row);
        if self.commands.recording {
            self.commands.posts.push(Post {
                name: name.to_string(),
                slot: arm.slot(),
                receipt,
            });
        }
    }

    /// Feed the watched arms' lifecycle events to the book and drop arms
    /// with nothing left to settle. A watched arm that is no longer its
    /// slot's current arm retired its slot.
    fn drain_watched(&mut self) {
        if self.commands.watched.is_empty() {
            return;
        }
        let watched = std::mem::take(&mut self.commands.watched);
        let mut kept = Vec::with_capacity(watched.len());
        for arm in watched {
            loop {
                let (event, _) = arm.take_lifecycle_event();
                let Some(event) = event else {
                    break;
                };
                self.commands.book.observe(&event);
            }
            let slot = arm.slot();
            let current = self.play.as_ref().is_some_and(|play| {
                play.statuses()
                    .iter()
                    .filter_map(|status| play.arm(&status.username))
                    .any(|live| Arc::ptr_eq(&live, &arm))
            });
            if !current {
                self.commands.book.slot_retired(slot);
            }
            let waiting = self.commands.book.reports().any(|report| {
                report.members().iter().any(|member| {
                    member.scope() == Scope::Slot(slot)
                        && matches!(
                            member.pending(),
                            Some(crate::command::Pending::Posted { .. })
                        )
                })
            });
            if waiting {
                kept.push(arm);
            }
        }
        self.commands.watched = kept;
    }

    /// Settle owner-reported members from their operation outcomes.
    fn settle_owned(&mut self) {
        self.settle_app_owned();
        if self.commands.owned.is_empty() {
            return;
        }
        let mut index = 0;
        while index < self.commands.owned.len() {
            let owned = &self.commands.owned[index];
            let still_pending = self
                .commands
                .book
                .report(owned.request)
                .and_then(|report| report.member(owned.subject))
                .is_some_and(|member| member.pending().is_some());
            let settled = match self.operations.get(owned.op) {
                // The operation book keeps a report with a pending member;
                // one gone before this read settled and was evicted.
                None => Some((
                    Settlement::Cancelled(Cancellation::ScopeInvalidated),
                    Some("operation report evicted".to_string()),
                )),
                Some(report) => report.outcome(&owned.name).and_then(|outcome| {
                    terminal_of(owned.command, outcome)
                        .map(|terminal| (terminal, outcome.reason().map(str::to_string)))
                }),
            };
            match (still_pending, settled) {
                (false, _) => {
                    self.commands.owned.swap_remove(index);
                }
                (true, Some((terminal, detail))) => {
                    let owned = self.commands.owned.swap_remove(index);
                    let _ = self.commands.book.settle(
                        owned.request,
                        owned.subject,
                        terminal,
                        detail.as_deref(),
                    );
                }
                (true, None) => index += 1,
            }
        }
    }

    /// Settle application members from their operation's (single) outcome.
    fn settle_app_owned(&mut self) {
        let mut index = 0;
        while index < self.commands.app_owned.len() {
            let (request, command, op) = self.commands.app_owned[index];
            let settled = match self.operations.get(op) {
                None => Some((
                    Settlement::Cancelled(Cancellation::ScopeInvalidated),
                    Some("operation report evicted".to_string()),
                )),
                Some(report) => report.members().first().and_then(|member| {
                    terminal_of(command, member.outcome())
                        .map(|terminal| (terminal, member.outcome().reason().map(str::to_string)))
                }),
            };
            match settled {
                Some((terminal, detail)) => {
                    self.commands.app_owned.swap_remove(index);
                    let _ = self.commands.book.settle(
                        request,
                        Subject::Application,
                        terminal,
                        detail.as_deref(),
                    );
                }
                None => index += 1,
            }
        }
    }

    /// The poll-time half: slot events, owner outcomes and process
    /// conditions. Costs nothing while no command is pending.
    pub(super) fn settle_commands(&mut self) {
        self.drain_watched();
        self.settle_owned();
        if !self.commands.book.has_pending() {
            return;
        }
        if let Ok(shared) = WorkService::shared() {
            let service = shared.lock();
            let conditions = service.conditions();
            self.commands.book.fail_unavailable(&conditions);
        }
    }
}

/// Rows whose dispatch attaches, resets or releases a slot device.
const fn needs_surface(command: OperatorCommand) -> bool {
    matches!(
        command,
        OperatorCommand::Load
            | OperatorCommand::Login
            | OperatorCommand::RelogNow
            | OperatorCommand::Remove
            | OperatorCommand::SetDraw(..)
    )
}

/// The surface of [`OperatorSession::submit_detached`]: the rows it admits
/// never reach it.
struct Detached<Io>(std::marker::PhantomData<Io>);

impl<Io> SlotSurface for Detached<Io> {
    type Io = Io;

    fn attach(
        &mut self,
        _name: &str,
        _profile: &mut crate::surface::SpawnProfile<'_>,
        _retained: Option<Io>,
    ) -> crate::surface::SlotAttach<Io> {
        unreachable!("submit_detached refuses every row that attaches a slot")
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {
        unreachable!("submit_detached refuses every row that resets a slot lifetime")
    }

    fn released(&mut self, _name: &str) {
        unreachable!("submit_detached refuses every row that releases a slot")
    }

    /// Detached code lends no models: a row that needs one is refused
    /// with [`Rejection::NoModel`].
    fn models(&mut self) -> crate::models::Models<'_> {
        crate::models::Models::none()
    }

    fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {
        unreachable!("submit_detached refuses SetDraw")
    }
}

/// A dispatch refusal's typed reason, from the core's refusal text.
fn rejection(reason: &str) -> Rejection {
    if reason.contains(SAVING) {
        Rejection::ProfileSaving
    } else if reason.contains("vault locked") {
        Rejection::NoHost
    } else {
        Rejection::UnknownProfile
    }
}

/// The member terminal an operation outcome stands for; `None` while it is
/// pending.
fn terminal_of(
    command: OperatorCommand,
    outcome: &Outcome,
) -> Option<Settlement<Completion, CommandFailure>> {
    if outcome.is_pending() {
        return None;
    }
    if outcome.is_completed() {
        return Some(Settlement::Completed(Completion::Done));
    }
    if outcome.is_cancelled() {
        return Some(Settlement::Cancelled(Cancellation::Superseded));
    }
    let reason = outcome.reason().unwrap_or_default();
    if outcome.is_skipped() {
        let skip = if reason.contains(SAVING) {
            SkipReason::ProfileSaving
        } else if reason.contains("not running") || reason.contains("no running") {
            SkipReason::NoRunningScript
        } else if reason.contains("no script") {
            SkipReason::NoScript
        } else if reason.contains("stopping") {
            SkipReason::AlreadyStopping
        } else {
            SkipReason::NotReady
        };
        return Some(Settlement::Completed(Completion::Skipped(skip)));
    }
    if reason.contains(SAVING) {
        return Some(Settlement::Failed(CommandFailure::Rejected(
            Rejection::ProfileSaving,
        )));
    }
    if reason.contains("vault locked") || reason == "no play" {
        return Some(Settlement::Failed(CommandFailure::Rejected(
            Rejection::NoHost,
        )));
    }
    if reason.starts_with("no profile") {
        return Some(Settlement::Failed(CommandFailure::Rejected(
            Rejection::UnknownProfile,
        )));
    }
    let kind = match command {
        OperatorCommand::Login | OperatorCommand::RelogNow => FailureKind::LoginFailed,
        OperatorCommand::Load | OperatorCommand::Remove => FailureKind::WorkerEnded,
        OperatorCommand::StartScript => FailureKind::StartFailed,
        _ => FailureKind::WriteFailed,
    };
    Some(Settlement::Failed(CommandFailure::Failed(kind)))
}
