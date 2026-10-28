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
    /// The waiting fleet plan's frozen targets
    /// ([`crate::OperatorSession::fleet_plan`]), with its card. The command
    /// must be the plan's [`crate::FleetPlan::command`]; submitting it
    /// takes the plan.
    Plan,
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
    /// [`OperatorSession::submit_detached`] was asked for a row that spawns
    /// or removes a slot.
    #[error("{0:?} needs the surface's slot devices")]
    NeedsSurface(OperatorCommand),
}

/// What a row's dispatch did with one member: the member's terminal, or
/// who settles it later. Every arm of the dispatch returns one; nothing
/// settles a member by default.
#[derive(Clone, Debug)]
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
    bulk: Option<crate::bulk::BulkReport>,
    marked: Option<crate::marked::MarkedCommandReport>,
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
    pub fn bulk(&self) -> Option<&crate::bulk::BulkReport> {
        self.bulk.as_ref()
    }

    /// Take the bulk report.
    pub fn take_bulk(&mut self) -> Option<crate::bulk::BulkReport> {
        self.bulk.take()
    }

    pub(crate) fn set_bulk(&mut self, bulk: crate::bulk::BulkReport) {
        self.bulk = Some(bulk);
    }
    /// The exact marked command's per-profile admission report.
    pub fn marked(&self) -> Option<&crate::marked::MarkedCommandReport> {
        self.marked.as_ref()
    }

    /// Take the marked command's per-profile admission report.
    pub fn take_marked(&mut self) -> Option<crate::marked::MarkedCommandReport> {
        self.marked.take()
    }

    pub(crate) fn set_marked(&mut self, marked: crate::marked::MarkedCommandReport) {
        self.marked = Some(marked);
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

/// A survival edit staged for [`OperatorCommand::SetSurvival`]: the
/// profile's Cure poison setting.
#[derive(Debug, PartialEq, Eq)]
pub struct SurvivalEdit {
    cure_poison: crate::profile_draft::CurePoison,
}

impl SurvivalEdit {
    /// The Cure poison setting the write stores.
    pub fn cure_poison(policy: crate::profile_draft::CurePoison) -> Self {
        Self {
            cure_poison: policy,
        }
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

/// Reusable member-name storage: a name list is emptied into the pool and
/// refilled from it, so the per-submit target and admitted-member lists keep
/// their `String` buffers and allocate nothing once warmed up.
#[derive(Default)]
pub(super) struct NamePool {
    /// Emptied name buffers, ready for the next [`Self::push`].
    strings: Vec<String>,
    /// Emptied lists, ready for the next [`Self::take_list`].
    lists: Vec<Vec<String>>,
}

impl NamePool {
    /// Empty `list`, keeping its name buffers for later pushes.
    fn clear(&mut self, list: &mut Vec<String>) {
        self.strings.append(list);
    }

    /// Append a copy of `name` to `list` in a reused buffer.
    fn push(&mut self, list: &mut Vec<String>, name: &str) {
        let mut slot = self.strings.pop().unwrap_or_default();
        slot.clear();
        slot.push_str(name);
        list.push(slot);
    }

    /// An empty list with retained capacity (or a new one while cold).
    fn take_list(&mut self) -> Vec<String> {
        self.lists.pop().unwrap_or_default()
    }

    /// Take `list` back, keeping its name buffers and its own capacity.
    fn recycle(&mut self, mut list: Vec<String>) {
        self.strings.append(&mut list);
        self.lists.push(list);
    }
}

/// The session's command state: the book, the members waiting on an owner
/// report, and the arms whose events settle posted members.
#[derive(Default)]
pub(super) struct CommandDesk {
    pub(super) book: CommandBook,
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
    /// The survival edit the next SetSurvival commits.
    survival: Option<SurvivalEdit>,
    /// Member names of the command being opened (reused buffer).
    names: Vec<String>,
    /// Name buffers behind `names` and the admitted-member lists.
    pool: NamePool,
    /// Members of the command being opened (reused buffer).
    targets: Vec<(Subject, Scope)>,
    /// The notice the next AcknowledgeNotice answers.
    notice: Option<super::prefs::Notice>,
    /// The path and text the next SaveLog writes.
    log_save: Option<(std::path::PathBuf, String)>,
    /// SaveLog commands whose write is still running.
    log_saves: Vec<(RequestId, crate::log::SaveTicket)>,
    /// Preference commands whose write is still queued on the
    /// local-state writer thread.
    pref_writes: Vec<super::prefs::PrefWriteWatch>,
    /// Saved-setting transitions recorded for a front end that asked to
    /// watch them ([`OperatorSession::watch_setting_events`]). Empty, and
    /// never pushed to, while no front end watches.
    setting_events: Vec<super::prefs::SettingEvent>,
    /// Set when the bounded event queue drops at least one transition.
    setting_events_overflowed: bool,
    /// Whether a front end drains `setting_events`.
    watch_setting_events: bool,
    /// The walk view a queued Nav or danger write adopted when it
    /// settled, for a front end that does not re-read it each frame (the
    /// headless driver) to take into its lent model.
    settled_walk_globals: Option<crate::walk_permissions::WalkGlobalsView>,
    /// The Quester Paths settings (and whether to write them) the next
    /// SetQuesterPaths applies.
    quester_paths: Option<(crate::quester_paths::QuesterPathsView, bool)>,
    /// The bound profile's game data the next Quester Paths row reloads
    /// with.
    quester_game_data: Option<Arc<api::game_data::SelectedGameData>>,
    /// The rows the next MarkAll marks (reused buffer).
    mark_all: Vec<crate::selection::ProfileIdentity>,
    /// The reorder the next ReorderProfiles applies.
    reorder: Option<crate::profile_order::ReorderOp>,
    /// What the next vault row commits.
    vault: Option<super::vault_rows::StagedVault>,
    /// The next Select also brings the member's worker up (the panel's
    /// chooser pick).
    select_opens_slot: bool,
    /// The fleet plan waiting for Confirm or Cancel.
    pub(super) fleet_plan: Option<crate::fleet_plan::FleetPlan>,
    /// The plan of the planned command being dispatched.
    pub(super) planned: Option<crate::fleet_plan::FleetPlan>,
    /// The latest fleet action's result view.
    pub(super) fleet_result: Option<crate::fleet_plan::FleetResult>,
    /// The card the next PlanFleetAction uses instead of the fleet card
    /// (the Scripts window's copy shortcut).
    pub(super) plan_card: Option<crate::scripts::ScriptChoice>,
    /// The one profile the next PlanFleetAction acts on instead of the
    /// marks (a world bubble, the TUI's cursor row).
    pub(super) plan_target: Option<crate::selection::ProfileIdentity>,
    /// Switch world members not settled yet ([`super::world_switch`]).
    pub(super) world_switches: Vec<super::world_switch::WorldSwitch>,
    /// Commands refused before they became operations ([`OperatorSession::refusals`]).
    pub(super) refusals: u64,
}

impl CommandDesk {
    pub(super) fn book(&self) -> &CommandBook {
        &self.book
    }

    /// Feed `arm`'s lifecycle events to the book until its posted members
    /// settle.
    pub(super) fn watch_arm(&mut self, arm: Arc<SlotArm>) {
        if !self.watched.iter().any(|w| Arc::ptr_eq(w, &arm)) {
            self.watched.push(arm);
        }
    }

    pub(super) fn stage_vault(&mut self, staged: super::vault_rows::StagedVault) {
        self.vault = Some(staged);
    }

    pub(super) fn take_vault(&mut self) -> Option<super::vault_rows::StagedVault> {
        self.vault.take()
    }

    pub(super) fn mark_all_staging(&mut self) -> &mut Vec<crate::selection::ProfileIdentity> {
        &mut self.mark_all
    }

    pub(super) fn stage_reorder(&mut self, op: crate::profile_order::ReorderOp) {
        self.reorder = Some(op);
    }

    pub(super) fn take_reorder(&mut self) -> Option<crate::profile_order::ReorderOp> {
        self.reorder.take()
    }

    pub(super) fn stage_notice(&mut self, notice: super::prefs::Notice) {
        self.notice = Some(notice);
    }

    pub(super) fn take_notice(&mut self) -> Option<super::prefs::Notice> {
        self.notice.take()
    }

    pub(super) fn stage_log_save(&mut self, path: std::path::PathBuf, text: String) {
        self.log_save = Some((path, text));
    }

    pub(super) fn take_log_save(&mut self) -> Option<(std::path::PathBuf, String)> {
        self.log_save.take()
    }

    pub(super) fn watch_log_save(&mut self, request: RequestId, ticket: crate::log::SaveTicket) {
        self.log_saves.push((request, ticket));
    }

    pub(super) fn watch_pref_write(&mut self, watch: super::prefs::PrefWriteWatch) {
        self.pref_writes.push(watch);
    }

    pub(super) fn pref_writes(&mut self) -> &mut Vec<super::prefs::PrefWriteWatch> {
        &mut self.pref_writes
    }

    pub(super) fn preferences_pending(&self) -> bool {
        !self.pref_writes.is_empty()
    }

    pub(super) fn set_watch_setting_events(&mut self, watch: bool) {
        self.watch_setting_events = watch;
        if !watch {
            self.setting_events.clear();
            self.setting_events_overflowed = false;
        }
    }

    pub(super) fn watching_setting_events(&self) -> bool {
        self.watch_setting_events
    }

    /// Record one saved-setting transition for a watching front end. With
    /// no watcher this is one bool check: nothing is built or kept. When
    /// the bounded queue fills, the returned batch marks itself incomplete.
    pub(super) fn note_setting_event(
        &mut self,
        setting: &str,
        state: super::prefs::SettingEventState,
        error: Option<String>,
    ) {
        if !self.watch_setting_events {
            return;
        }
        if self.setting_events.len() >= super::prefs::SETTING_EVENTS_CAP {
            self.setting_events_overflowed = true;
            return;
        }
        self.setting_events.push(super::prefs::SettingEvent::new(
            setting.to_owned(),
            state,
            error,
        ));
    }

    pub(super) fn take_setting_events(&mut self) -> (Vec<super::prefs::SettingEvent>, bool) {
        (
            std::mem::take(&mut self.setting_events),
            std::mem::take(&mut self.setting_events_overflowed),
        )
    }

    pub(super) fn settle_walk_globals(&mut self, view: crate::walk_permissions::WalkGlobalsView) {
        self.settled_walk_globals = Some(view);
    }

    pub(super) fn take_settled_walk_globals(
        &mut self,
    ) -> Option<crate::walk_permissions::WalkGlobalsView> {
        self.settled_walk_globals.take()
    }

    pub(super) fn stage_quester_paths(
        &mut self,
        after: crate::quester_paths::QuesterPathsView,
        persist: bool,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        self.quester_paths = Some((after, persist));
        self.quester_game_data = selected;
    }

    pub(super) fn stage_quester_game_data(
        &mut self,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        self.quester_game_data = selected;
    }

    pub(super) fn take_quester_game_data(
        &mut self,
    ) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.quester_game_data.take()
    }

    pub(super) fn take_quester_paths(
        &mut self,
    ) -> Option<(crate::quester_paths::QuesterPathsView, bool)> {
        self.quester_paths.take()
    }

    /// Settle every SaveLog whose write finished.
    fn settle_log_saves(&mut self) {
        let book = &mut self.book;
        self.log_saves.retain(|(request, ticket)| {
            let Some(outcome) = ticket.poll() else {
                return true;
            };
            let (terminal, detail) = match outcome {
                Ok(_) => (Settlement::Completed(Completion::Done), None),
                Err(reason) => (
                    Settlement::Failed(CommandFailure::Failed(FailureKind::WriteFailed)),
                    Some(reason),
                ),
            };
            let _ = book.settle(*request, Subject::Application, terminal, detail.as_deref());
            false
        });
    }

    /// A handle on the running save of `request` (SaveLog), for the
    /// front end's status line.
    pub(super) fn log_save_ticket(&self, request: RequestId) -> Option<crate::log::SaveTicket> {
        self.log_saves
            .iter()
            .find(|(r, _)| *r == request)
            .map(|(_, ticket)| ticket.share())
    }
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

    /// Stage a new profile with `username`, `password`, a fresh uid and
    /// default settings for the next [`OperatorCommand::CreateProfile`].
    pub fn stage_new_profile(&mut self, username: &str, password: &str) {
        let uid = self.fresh_uid();
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
    }

    /// Stage `profile` as the write the next [`OperatorCommand::SaveProfile`]
    /// commits, `mirror` applied to a running slot when it is durable
    /// (tests: the profile form stages this for the panel).
    #[cfg(any(test, feature = "test-support"))]
    pub fn stage_profile_save_for_test(
        &mut self,
        profile: vault::Profile,
        mirror: super::ArmMirror,
        label: &'static str,
    ) {
        self.stage_profile_write(StagedProfileWrite::new(profile, None, false, mirror, label));
    }

    /// Stage the guardian edit the next [`OperatorCommand::SetGuardian`]
    /// commits.
    pub fn stage_guardian(&mut self, edit: GuardianEdit) {
        self.commands.guardian = Some(edit);
    }

    /// Stage the survival edit the next [`OperatorCommand::SetSurvival`]
    /// commits.
    pub fn stage_survival(&mut self, edit: SurvivalEdit) {
        self.commands.survival = Some(edit);
    }

    /// Stage that the next [`OperatorCommand::Select`] also brings the
    /// member's worker up without changing its login intent (the panel's
    /// chooser pick; a TUI focus change only selects). The submit needs
    /// the surface's slot devices.
    pub fn stage_select_opens_slot(&mut self) {
        self.commands.select_opens_slot = true;
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
    /// window ([`super::SLOT_REMOVE_TIMEOUT`]). This is the command
    /// boundary: a command refused before it becomes an operation logs one
    /// `info` line here, for every front end alike.
    pub fn submit_at<S: SlotSurface<Io = Io>>(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
        now: Instant,
        surface: &mut S,
    ) -> Result<Submitted, SubmitError> {
        // Target names are scratch for one resolution; none left over from
        // an earlier command may name this refusal's bots.
        let mut names = std::mem::take(&mut self.commands.names);
        self.commands.pool.clear(&mut names);
        self.commands.names = names;
        let submitted = self.submit_unlogged(command, targets, now, surface);
        if let Err(error) = &submitted {
            let bots = self.refused_bots(targets);
            self.record_refusal(command, &bots, &error.to_string());
        }
        submitted
    }

    /// How many commands this session has refused before they became
    /// operations. A front end's error line uses it to tell a repeated
    /// refusal (same text, new refusal) from the message already shown.
    pub fn refusals(&self) -> u64 {
        self.commands.refusals
    }

    /// One info line for the members of `request` that were refused, naming
    /// the command, the bots and the reason. A refusal settles a member as
    /// [`CommandFailure::Rejected`] before any operation exists for it. One
    /// line per command, however many bots it refused.
    fn log_refusals(&mut self, request: RequestId, command: OperatorCommand) {
        let Some(report) = self.commands.book.report(request) else {
            return;
        };
        let mut refused: Vec<(String, String)> = Vec::new();
        for member in report.members() {
            let Some(Settlement::Failed(CommandFailure::Rejected(rejection))) = member.terminal()
            else {
                continue;
            };
            let name = match member.subject() {
                Subject::Profile(identity) => self.name_for_identity(identity),
                Subject::Application => "application".to_owned(),
            };
            let reason = match member.detail() {
                Some(detail) => format!("{rejection:?}: {detail}"),
                None => format!("{rejection:?}"),
            };
            refused.push((name, reason));
        }
        let bots = refused
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let reason = match refused.as_slice() {
            [] => return,
            [(_, reason)] => reason.clone(),
            many => many
                .iter()
                .map(|(name, reason)| format!("{name}: {reason}"))
                .collect::<Vec<_>>()
                .join("; "),
        };
        self.record_refusal(command, &bots, &reason);
    }

    /// The bots a refused submission named, for its log line: the names the
    /// caller gave, or the ones the targets resolved to before the refusal.
    fn refused_bots(&self, targets: Targets<'_>) -> String {
        const NO_BOTS: &str = "no bots";
        let joined = |names: &[String]| {
            if names.is_empty() {
                NO_BOTS.to_owned()
            } else {
                names.join(", ")
            }
        };
        match targets {
            Targets::Application => "application".to_owned(),
            Targets::Profile(name) => name.to_owned(),
            Targets::Profiles(names) => joined(names),
            Targets::Selected => self.selected.clone().unwrap_or_else(|| NO_BOTS.to_owned()),
            Targets::Everyone | Targets::Marked | Targets::Plan => joined(&self.commands.names),
        }
    }

    /// Log and count one refused command: the one place a refusal is
    /// written, for a refused submission, for refused members and for a front
    /// end's refusal before it can submit ([`Self::refuse_unsubmitted`]). Never
    /// for `GameInput`, which is staged every frame.
    fn record_refusal(&mut self, command: OperatorCommand, bots: &str, reason: &str) {
        if command == OperatorCommand::GameInput {
            return;
        }
        self.commands.refusals += 1;
        api::host_log!(
            api::hostlog::Category::Lifecycle,
            api::hostlog::Level::Info,
            "command refused: {command:?} on {bots}: {reason}"
        );
    }

    /// A front end refused `command` before it could submit it (a missing
    /// focus, a bot with no card). The refusal logs and counts as every other
    /// refusal does, with the bots it named (`&[]` for none) and the reason
    /// the operator sees.
    pub fn refuse_unsubmitted(&mut self, command: OperatorCommand, bots: &[&str], reason: &str) {
        let bots = if bots.is_empty() {
            "no bots".to_owned()
        } else {
            bots.join(", ")
        };
        self.record_refusal(command, &bots, reason);
    }

    fn submit_unlogged<S: SlotSurface<Io = Io>>(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
        now: Instant,
        surface: &mut S,
    ) -> Result<Submitted, SubmitError> {
        let command = self.route_paint_control(command, targets, surface);
        // A planned command runs over the plan's frozen marks, never the
        // lent ones.
        let plan = if matches!(targets, Targets::Plan) {
            Some(self.take_plan_for(command)?)
        } else {
            None
        };
        let targets = if plan.is_some() {
            Targets::Marked
        } else {
            targets
        };
        let resolved = match plan.as_ref() {
            Some(plan) => self.resolve_targets(command, targets, Some(&plan.marks)),
            None => {
                let models = surface.models();
                self.resolve_targets(command, targets, models.marks())
            }
        };
        if let Err(error) = resolved {
            return match plan {
                // Every frozen target is gone: the plan ends here, its
                // result naming each one skipped.
                Some(plan) if matches!(error, SubmitError::NoTarget(_)) => {
                    self.end_emptied_plan(plan, surface)
                }
                // Nothing ran: the plan stays for another try or a Cancel.
                Some(plan) => {
                    self.commands.fleet_plan = Some(plan);
                    Err(error)
                }
                None => Err(error),
            };
        }
        let request = match self.open_command(command) {
            Ok(request) => request,
            Err(error) => {
                if let Some(plan) = plan {
                    self.commands.fleet_plan = Some(plan);
                }
                return Err(error);
            }
        };
        self.commands.planned = plan;
        let mut submitted = Submitted {
            request,
            operation: None,
            removal: None,
            added: 0,
            bulk: None,
            marked: None,
        };
        self.commands.posts.clear();
        self.commands.recording = command.settles() == Settles::IntentApplied;
        let planned = self.commands.planned.is_some();
        if planned {
            let root = self
                .commands
                .planned
                .as_ref()
                .and_then(|plan| plan.catalog_root.clone());
            if let Some(scripts) = surface.models().scripts.as_deref_mut() {
                scripts.set_planned_run(true);
                if root.is_some() {
                    scripts.stage_start_catalog_root(root);
                }
            }
        }
        self.dispatch(command, targets, now, surface, &mut submitted);
        if planned {
            if let Some(scripts) = surface.models().scripts.as_deref_mut() {
                scripts.set_planned_run(false);
            }
        }
        self.commands.recording = false;
        self.record_posts(request, command);
        let ran_plan = self.commands.planned.take();
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
        self.log_refusals(request, command);
        if let Some(plan) = ran_plan {
            // A planned Log in can load the first members: with nothing
            // selected, focus the remembered (else first) one, as Load all
            // does. An existing selection is never taken.
            if plan.action() == crate::FleetAction::Login && self.selected.is_none() {
                let hint = self.focus_hint.clone();
                if let Some(name) = super::pick_focus(self.members(), hint.as_deref()) {
                    self.select(&name);
                }
            }
            self.open_fleet_result(Some(request), plan);
        }
        Ok(submitted)
    }

    /// A staged host paint control never dispatches as `ScriptPaintInput`.
    /// When its binding still holds for the one target
    /// ([`OperatorSession::paint_control_refusal`]), the submission becomes
    /// the control's own Pause, Resume or Stop row, with that row's
    /// settlement. A refused binding stays a `ScriptPaintInput`, which
    /// reports why and controls nothing.
    fn route_paint_control<S: SlotSurface<Io = Io>>(
        &self,
        command: OperatorCommand,
        targets: Targets<'_>,
        surface: &mut S,
    ) -> OperatorCommand {
        if command != OperatorCommand::ScriptPaintInput {
            return command;
        }
        let mut models = surface.models();
        let Some(scripts) = models.scripts.as_deref_mut() else {
            return command;
        };
        let Some(activation) = scripts.staged_paint_control() else {
            return command;
        };
        let name = match targets {
            Targets::Profile(name) => Some(name),
            Targets::Selected => self.selected(),
            _ => None,
        };
        let Some(name) = name else {
            return command;
        };
        match self.paint_control_refusal(name, &activation) {
            None => {
                scripts.take_staged_paint_input();
                activation.control.command()
            }
            Some(reason) => {
                scripts.refuse_staged_paint_control(reason);
                command
            }
        }
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
        self.commands.pool.clear(&mut names);
        let application_target = matches!(targets, Targets::Application);
        match (command.subject(), targets) {
            (SubjectKind::Application, Targets::Application) => {}
            (SubjectKind::Profile, Targets::Application)
                if command.accepts_subject(SubjectKind::Application) => {}
            (SubjectKind::Application, _) | (SubjectKind::Profile, Targets::Application) => {
                self.commands.names = names;
                return Err(SubmitError::WrongTargets(command));
            }
            (SubjectKind::Profile, Targets::Profile(name)) => {
                self.commands.pool.push(&mut names, name);
            }
            (SubjectKind::Profile, Targets::Profiles(list)) => {
                for name in list {
                    if !names.contains(name) {
                        self.commands.pool.push(&mut names, name);
                    }
                }
            }
            (SubjectKind::Profile, Targets::Selected) => {
                if let Some(name) = self.selected.as_deref() {
                    self.commands.pool.push(&mut names, name);
                }
            }
            (SubjectKind::Profile, Targets::Everyone) => self.everyone(command, &mut names),
            (SubjectKind::Profile, Targets::Marked | Targets::Plan) => {
                if let Some(marks) = marks {
                    for (name, identity) in crate::selection::profile_rows(self) {
                        if marks.contains(identity) && !names.contains(&name) {
                            names.push(name);
                        }
                    }
                    // The saved Load order, as Load all admits them; a
                    // name the order lacks follows by name.
                    names.sort_by(|a, b| {
                        let rank = |name: &str| self.order_rank(name).unwrap_or(usize::MAX);
                        rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
                    });
                }
            }
        }
        let mut targets = std::mem::take(&mut self.commands.targets);
        targets.clear();
        if command.subject() == SubjectKind::Application || application_target {
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

    /// A plan none of whose frozen targets remains: nothing runs, a copy
    /// gives back its prepared scope and start hold, and the result is
    /// terminal at once with every target a named skip. The request is
    /// minted for the caller but opens no command record (a record needs
    /// a member).
    fn end_emptied_plan<S: SlotSurface<Io = Io>>(
        &mut self,
        plan: crate::FleetPlan,
        surface: &mut S,
    ) -> Result<Submitted, SubmitError> {
        let request = WorkService::shared()?
            .lock()
            .mint_identity(Scope::Process, None)
            .map_err(|_| SubmitError::IdentityExhausted)?
            .job();
        if plan.action() == crate::FleetAction::CopySettings {
            if let Some(scripts) = surface.models().scripts.as_deref_mut() {
                scripts.cancel_settings_sync();
            }
        }
        self.open_fleet_result(None, plan);
        Ok(Submitted {
            request,
            operation: None,
            removal: None,
            added: 0,
            bulk: None,
            marked: None,
        })
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

    /// The admitted members of the command being dispatched, in a pooled
    /// list: hand it back with [`Self::release_names`] when the arm is done.
    pub(super) fn admitted_names(&mut self, request: RequestId) -> Vec<String> {
        let mut admitted = self.commands.pool.take_list();
        let names = std::mem::take(&mut self.commands.names);
        for name in &names {
            if self.admitted(request, name) {
                self.commands.pool.push(&mut admitted, name);
            }
        }
        self.commands.names = names;
        admitted
    }

    /// Return a list from [`Self::admitted_names`] to the pool.
    pub(super) fn release_names(&mut self, names: Vec<String>) {
        self.commands.pool.recycle(names);
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
                self.release_names(names);
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
                self.release_names(names);
            }
            OperatorCommand::Login if matches!(targets, Targets::Marked) => {
                self.login_marked(request, surface, submitted);
            }
            OperatorCommand::Login => {
                let names = self.admitted_names(request);
                if !names.is_empty() {
                    submitted.operation = Some(self.login_members(&names, surface));
                }
                self.release_names(names);
            }
            OperatorCommand::Logout if matches!(targets, Targets::Marked) => {
                self.logout_marked(request, surface, submitted);
            }
            OperatorCommand::Logout => {
                let names = self.admitted_names(request);
                if !names.is_empty() {
                    submitted.operation = Some(self.logout_members(&names));
                }
                self.release_names(names);
            }
            OperatorCommand::RelogNow => {
                let names = self.admitted_names(request);
                for name in &names {
                    let op = self.request_memory_relog(name, surface);
                    submitted.operation = Some(op);
                }
                self.release_names(names);
            }
            OperatorCommand::SetAutoLogin(on) => {
                let names = self.admitted_names(request);
                for name in &names {
                    let result = self.set_auto_login(name, on);
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    self.owner_result(request, command, name, result);
                }
                self.release_names(names);
            }
            OperatorCommand::SetMemoryMode(mode) => {
                let lowmem = mode == api::lifecycle::MemoryMode::Low;
                let mut models = surface.models();
                let names = self.admitted_names(request);
                for name in &names {
                    let result = self.set_memory_mode(name, lowmem);
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    if let Some(form_save) = models.form_save.as_deref_mut() {
                        match &result {
                            Ok(op) => form_save.submitted(self, *op, Some(name), name),
                            Err(reason) => form_save.refused(reason.clone()),
                        }
                    }
                    self.owner_result(request, command, name, result);
                }
                self.release_names(names);
            }
            OperatorCommand::Select => {
                let open = std::mem::take(&mut self.commands.select_opens_slot);
                let names = self.admitted_names(request);
                for name in &names {
                    if open {
                        if let Err(reason) = self.open_slot(name, surface) {
                            let applied = Applied::Rejected(rejection(&reason), Some(reason));
                            self.settle_member(request, command, name, applied, submitted);
                            continue;
                        }
                    }
                    self.select(name);
                    self.settle_member(request, command, name, Applied::Done, submitted);
                }
                self.release_names(names);
            }
            OperatorCommand::PauseScript | OperatorCommand::ResumeScript => {
                let names = self.admitted_names(request);
                for name in &names {
                    // Today's toggle: Resume when Paused, otherwise Pause
                    // (the run owner settles a Pause with no running script
                    // as skipped).
                    let paused = self
                        .play
                        .as_ref()
                        .is_some_and(|play| play.script_state(name) == script::RunState::Paused);
                    let applies = (command == OperatorCommand::ResumeScript) == paused;
                    if applies {
                        let op = self.toggle_pause(name);
                        self.own_member(request, command, name, op);
                    } else {
                        let reason = if command == OperatorCommand::PauseScript {
                            SkipReason::NoRunningScript
                        } else {
                            SkipReason::NoScript
                        };
                        self.settle_name(
                            request,
                            name,
                            Settlement::Completed(Completion::Skipped(reason)),
                            None,
                        );
                    }
                }
                self.release_names(names);
            }
            OperatorCommand::StopScript => {
                let mut names = self.admitted_names(request);
                if !matches!(targets, Targets::Everyone) {
                    self.cancel_waiting_starts(request, command, &mut names, surface, submitted);
                }
                match (targets, names.as_slice()) {
                    // One member: today's single Stop, which settles when the
                    // run owner reports the slot idle.
                    (Targets::Profile(_) | Targets::Selected, [name]) => {
                        submitted.operation = Some(self.stop_script(name));
                    }
                    (Targets::Everyone, _) => {
                        let operation = {
                            let mut models = surface.models();
                            if let Some(scripts) = models.scripts.as_deref_mut() {
                                scripts.stop_all_with_receipt(self).1
                            } else {
                                Some(self.stop_scripts(&names).0)
                            }
                        };
                        if let Some(operation) = operation {
                            submitted.operation = Some(operation);
                        } else {
                            for name in &names {
                                self.settle_member(
                                    request,
                                    command,
                                    name,
                                    Applied::Skipped(SkipReason::NotReady),
                                    submitted,
                                );
                            }
                        }
                    }
                    (_, []) => {}
                    _ => submitted.operation = Some(self.stop_scripts(&names).0),
                }
                self.release_names(names);
            }
            OperatorCommand::SaveProfile | OperatorCommand::CreateProfile => {
                let names = if command == OperatorCommand::SaveProfile {
                    self.admitted_names(request)
                } else {
                    Vec::new()
                };
                let admitted = if command == OperatorCommand::CreateProfile {
                    application_admitted
                } else {
                    !names.is_empty()
                };
                if admitted {
                    let mut models = surface.models();
                    #[cfg(feature = "js")]
                    {
                        let imported = match models.import.as_deref_mut() {
                            Some(import) => match import.take_staged_account(command) {
                                Ok(imported) => imported,
                                Err(error) => {
                                    let applied = Applied::Rejected(
                                        Rejection::NotStaged,
                                        Some(error.to_string()),
                                    );
                                    if command == OperatorCommand::CreateProfile {
                                        self.settle_app(request, command, applied, submitted);
                                    } else {
                                        for name in &names {
                                            self.settle_member(
                                                request,
                                                command,
                                                name,
                                                Applied::Rejected(
                                                    Rejection::NotStaged,
                                                    Some(error.to_string()),
                                                ),
                                                submitted,
                                            );
                                        }
                                    }
                                    return;
                                }
                            },
                            None => None,
                        };
                        if let Some(imported) = imported {
                            let crate::rs2b0t_import::StagedImportAccount {
                                index,
                                username,
                                write,
                            } = imported;
                            let matches_target = if command == OperatorCommand::CreateProfile {
                                write.profile.username.as_str() == username
                            } else {
                                names.len() == 1 && names[0].as_str() == username
                            };
                            if !matches_target {
                                let reason = format!(
                                "profile import account {username} does not match the writer target"
                            );
                                if let Some(import) = models.import.as_deref_mut() {
                                    import.record_write_result(index, Err(&reason));
                                }
                                if command == OperatorCommand::CreateProfile {
                                    self.settle_app(
                                        request,
                                        command,
                                        Applied::Rejected(Rejection::NotStaged, Some(reason)),
                                        submitted,
                                    );
                                } else {
                                    for name in &names {
                                        self.settle_member(
                                            request,
                                            command,
                                            name,
                                            Applied::Rejected(
                                                Rejection::NotStaged,
                                                Some(reason.clone()),
                                            ),
                                            submitted,
                                        );
                                    }
                                }
                                return;
                            }

                            self.stage_profile_write(write);
                            let write = self
                                .commands
                                .profile_write
                                .take()
                                .expect("the imported account was just staged");
                            let result = match write.rename_from.as_deref() {
                                Some(old) => {
                                    let old = old.to_string();
                                    self.rename_profile(
                                        &old,
                                        write.profile,
                                        write.mirror,
                                        write.label,
                                    )
                                }
                                None if write.creating => {
                                    self.create_profile(write.profile, write.mirror, write.label)
                                }
                                None => self.save_profile(write.profile, write.mirror, write.label),
                            };
                            if let Some(import) = models.import.as_deref_mut() {
                                import.record_write_result(
                                    index,
                                    match &result {
                                        Ok(op) => Ok(*op),
                                        Err(reason) => Err(reason),
                                    },
                                );
                            }
                            let applied = match &result {
                                Ok(op) => Applied::Operation(*op),
                                Err(reason) => {
                                    Applied::Rejected(rejection(reason), Some(reason.clone()))
                                }
                            };
                            if command == OperatorCommand::CreateProfile {
                                self.settle_app(request, command, applied, submitted);
                            } else {
                                self.settle_member(request, command, &username, applied, submitted);
                            }
                            return;
                        }
                    }

                    let editor_submission = if models.editor.is_some() {
                        let Some(scripts) = models.scripts.as_deref() else {
                            let applied = Applied::Rejected(
                                Rejection::NoModel,
                                Some("profile form save requires the script model".into()),
                            );
                            if command == OperatorCommand::CreateProfile {
                                self.settle_app(request, command, applied, submitted);
                            } else {
                                for name in &names {
                                    self.settle_member(
                                        request,
                                        command,
                                        name,
                                        Applied::Rejected(
                                            Rejection::NoModel,
                                            Some(
                                                "profile form save requires the script model"
                                                    .into(),
                                            ),
                                        ),
                                        submitted,
                                    );
                                }
                            }
                            return;
                        };
                        let editor = models.editor.as_deref_mut().expect("checked above");
                        match editor.save(self, scripts, command) {
                            Ok(submission) => Some(submission),
                            Err(error) => {
                                let (reason, detail) = match error {
                                    crate::SaveRefused::StillSaving => {
                                        (Rejection::ProfileSaving, error.to_string())
                                    }
                                    crate::SaveRefused::Refused => (
                                        Rejection::VaultRefused,
                                        editor
                                            .save_state()
                                            .notice()
                                            .and_then(|notice| notice.error())
                                            .unwrap_or("profile form save was refused")
                                            .to_owned(),
                                    ),
                                };
                                if command == OperatorCommand::CreateProfile {
                                    self.settle_app(
                                        request,
                                        command,
                                        Applied::Rejected(reason, Some(detail)),
                                        submitted,
                                    );
                                } else {
                                    for name in &names {
                                        self.settle_member(
                                            request,
                                            command,
                                            name,
                                            Applied::Rejected(reason, Some(detail.clone())),
                                            submitted,
                                        );
                                    }
                                }
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    let staged = self.commands.profile_write.take();
                    match staged {
                        Some(write) => {
                            let (source, destination) = editor_submission.unwrap_or_else(|| {
                                (write.rename_from.clone(), write.profile.username.clone())
                            });
                            let result = match write.rename_from.as_deref() {
                                Some(old) => {
                                    let old = old.to_string();
                                    self.rename_profile(
                                        &old,
                                        write.profile,
                                        write.mirror,
                                        write.label,
                                    )
                                }
                                None if write.creating => {
                                    self.create_profile(write.profile, write.mirror, write.label)
                                }
                                None => self.save_profile(write.profile, write.mirror, write.label),
                            };
                            match &result {
                                Ok(op) => {
                                    if let Some(editor) = models.editor.as_deref_mut() {
                                        editor.submitted(
                                            self,
                                            *op,
                                            source.as_deref(),
                                            &destination,
                                        );
                                    } else if let Some(form_save) = models.form_save.as_deref_mut()
                                    {
                                        form_save.submitted(
                                            self,
                                            *op,
                                            source.as_deref(),
                                            &destination,
                                        );
                                    }
                                }
                                Err(reason) => {
                                    if let Some(editor) = models.editor.as_deref_mut() {
                                        editor.refused(reason.clone());
                                    } else if let Some(form_save) = models.form_save.as_deref_mut()
                                    {
                                        form_save.refused(reason.clone());
                                    }
                                }
                            }
                            self.settle_form_write(request, command, result, submitted);
                        }
                        None => {
                            let applied = Applied::Rejected(
                                Rejection::NotStaged,
                                Some("nothing staged to save".into()),
                            );
                            if command == OperatorCommand::CreateProfile {
                                self.settle_app(request, command, applied, submitted);
                            } else {
                                for name in &names {
                                    self.settle_member(
                                        request,
                                        command,
                                        name,
                                        Applied::Rejected(
                                            Rejection::NotStaged,
                                            Some("nothing staged to save".into()),
                                        ),
                                        submitted,
                                    );
                                }
                            }
                        }
                    }
                }
                self.release_names(names);
            }
            OperatorCommand::DeleteProfile => {
                let mut models = surface.models();
                let names = self.admitted_names(request);
                for name in &names {
                    if let Some(editor) = models.editor.as_deref_mut() {
                        if !editor.before_delete(self, name) {
                            self.settle_member(
                                request,
                                command,
                                name,
                                Applied::Skipped(SkipReason::DecisionStaged),
                                submitted,
                            );
                            continue;
                        }
                    }
                    let identity = self.profile_identity_or_synthetic(name);
                    match self.vault_remove(name) {
                        Ok(Some(op)) => {
                            if let Some(fleet) = models.fleet.as_deref_mut() {
                                fleet.remove_mark(identity);
                            }
                            submitted.operation = Some(op);
                            self.own_member_as(request, command, name, identity, op);
                        }
                        Ok(None) => self.settle_name(
                            request,
                            name,
                            Settlement::Completed(Completion::Skipped(
                                SkipReason::ProfileUnavailable,
                            )),
                            None,
                        ),
                        Err(reason) => self.owner_result(request, command, name, Err(reason)),
                    }
                }
                self.release_names(names);
            }
            OperatorCommand::SetGuardian => {
                let edit = self.commands.guardian.take();
                let mut models = surface.models();
                let names = self.admitted_names(request);
                for name in &names {
                    let result = match edit.as_ref().map(|edit| &edit.0) {
                        Some(GuardianEditKind::Fields {
                            random_events,
                            lamp_skill,
                            lamp_auto,
                        }) => {
                            self.set_random_settings(name, *random_events, lamp_skill, *lamp_auto)
                        }
                        Some(GuardianEditKind::Policy { event, policy }) => {
                            self.set_random_event_policy(name, event, *policy)
                        }
                        None => Err("random: nothing staged".to_string()),
                    };
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    if let Some(form_save) = models.form_save.as_deref_mut() {
                        match &result {
                            Ok(op) => form_save.submitted(self, *op, Some(name), name),
                            Err(reason) => form_save.refused(format!("settings: {reason}")),
                        }
                    }
                    self.owner_result(request, command, name, result);
                }
                self.release_names(names);
            }
            OperatorCommand::SetSurvival => {
                let edit = self.commands.survival.take();
                let mut models = surface.models();
                let names = self.admitted_names(request);
                for name in &names {
                    let result = match edit.as_ref() {
                        Some(edit) => self.set_survival_settings(name, edit.cure_poison),
                        None => Err("survival: nothing staged".to_string()),
                    };
                    submitted.operation = result.as_ref().ok().copied().or(submitted.operation);
                    if let Some(form_save) = models.form_save.as_deref_mut() {
                        match &result {
                            Ok(op) => form_save.submitted(self, *op, Some(name), name),
                            Err(reason) => form_save.refused(format!("settings: {reason}")),
                        }
                    }
                    self.owner_result(request, command, name, result);
                }
                self.release_names(names);
            }
            OperatorCommand::SetRaster(raster) => {
                let names = self.admitted_names(request);
                for name in &names {
                    let result = self.set_profile_raster(name, raster);
                    self.optional_write(request, command, name, result, submitted);
                }
                self.release_names(names);
            }
            OperatorCommand::MarkTutorialSkipped => {
                let names = self.admitted_names(request);
                for name in &names {
                    let result = self.record_tutorial_state(name, true);
                    self.optional_write(request, command, name, result, submitted);
                }
                self.release_names(names);
            }
            OperatorCommand::SetWorld(world) => {
                let names = self.admitted_names(request);
                for name in &names {
                    let applied = self.set_world_preference(name, world);
                    self.settle_member(request, command, name, applied, submitted);
                }
                self.release_names(names);
            }
            OperatorCommand::Quit => {
                let names = self.admitted_names(request);
                self.quit(request, &names);
                self.release_names(names);
            }
            OperatorCommand::SwitchWorld(world) => {
                let names = self.admitted_names(request);
                for name in &names {
                    let applied = self.begin_world_switch(request, name, world);
                    self.settle_member(request, command, name, applied, submitted);
                }
                self.release_names(names);
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
            OperatorCommand::UnlockVault => self.vault_row(
                super::vault_rows::VaultRow::Unlock,
                request,
                application_admitted,
                surface,
                submitted,
            ),
            OperatorCommand::CreateVault => self.vault_row(
                super::vault_rows::VaultRow::Create,
                request,
                application_admitted,
                surface,
                submitted,
            ),
            OperatorCommand::ResetVault => self.vault_row(
                super::vault_rows::VaultRow::Reset,
                request,
                application_admitted,
                surface,
                submitted,
            ),
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
            OperatorCommand::ReorderProfiles => {
                let row = super::fleet::FleetRow::Reorder;
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::SetFleetCard => {
                let row = super::fleet::FleetRow::Card;
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::MarkFleet(op) => {
                let row = super::fleet::FleetRow::MarkOp(op);
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::PlanFleetAction(action) => {
                let row = super::fleet::FleetRow::Plan(action);
                self.fleet_row(row, request, application_admitted, surface, submitted)
            }
            OperatorCommand::CancelFleetPlan => {
                let row = super::fleet::FleetRow::CancelPlan;
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
            OperatorCommand::WalkTo => self.map_row(
                super::map::MapRow::WalkTo,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::GroupWalk => self.map_row(
                super::map::MapRow::GroupWalk,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::Teleport => self.map_row(
                super::map::MapRow::Teleport,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::DebugCommand => self.map_row(
                super::map::MapRow::DebugCommand,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::GameInput => self.map_row(
                super::map::MapRow::GameInput,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::OpenMapDemand => self.map_row(
                super::map::MapRow::OpenDemand,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::ReleaseMapDemand => self.map_row(
                super::map::MapRow::ReleaseDemand,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::BakeMapNow => self.map_row(
                super::map::MapRow::BakeNow,
                targets,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::DeclineMapBake => self.map_row(
                super::map::MapRow::DeclineBake,
                targets,
                request,
                surface,
                submitted,
            ),
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
            OperatorCommand::EditProfileDraft(field) => self.form_row(
                super::form::FormRow::Edit(field),
                request,
                surface,
                submitted,
            ),
            OperatorCommand::DiscardProfileEdits => {
                self.form_row(super::form::FormRow::Discard, request, surface, submitted)
            }
            OperatorCommand::KeepEditing => self.form_row(
                super::form::FormRow::KeepEditing,
                request,
                surface,
                submitted,
            ),
            OperatorCommand::CloseProfileForm(close) => self.form_row(
                super::form::FormRow::Close(close),
                request,
                surface,
                submitted,
            ),
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
            Applied::Rejected(rejection, detail) => (
                Settlement::Failed(CommandFailure::Rejected(rejection)),
                detail,
            ),
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

    /// Stop's half for a member whose Start still waits for its permit or
    /// its preparation: the coordinator cancels that Start (it never runs
    /// later, and its own command settles cancelled), and the member
    /// settles done here. A member with a live script stays in `names`
    /// for the host stop.
    fn cancel_waiting_starts<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        names: &mut Vec<String>,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let mut models = surface.models();
        let Some(scripts) = models.scripts.as_deref_mut() else {
            return;
        };
        let mut cancelled_any = false;
        let mut index = 0;
        while index < names.len() {
            let name = &names[index];
            let idle = self.play.as_ref().is_none_or(|play| {
                matches!(
                    play.script_state(name),
                    script::RunState::Idle | script::RunState::Error
                )
            });
            if scripts.cancel_queued_as(name, "cancelled") {
                cancelled_any = true;
                if idle {
                    let name = names.remove(index);
                    self.settle_member(request, command, &name, Applied::Done, submitted);
                    continue;
                }
            }
            index += 1;
        }
        if cancelled_any {
            scripts.publish_start_places(self);
        }
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
            Applied::Rejected(rejection, detail) => (
                Settlement::Failed(CommandFailure::Rejected(rejection)),
                detail,
            ),
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
        let _ = self
            .commands
            .book
            .settle(request, subject, terminal, detail);
    }

    /// The marks whose profile is gone, as the bulk report's skipped rows.
    fn gone_marks(&self, marks: Option<&crate::MarkedSelection>) -> Vec<crate::BulkRow> {
        let Some(marks) = marks else {
            return Vec::new();
        };
        let rows = crate::selection::profile_rows(self);
        marks
            .iter()
            .filter(|identity| !rows.iter().any(|(_, id)| id == identity))
            .map(|identity| {
                crate::BulkRow::new(
                    format!("profile#{}", identity.raw()),
                    crate::BulkOutcome::skipped("profile unavailable"),
                )
            })
            .collect()
    }

    /// Log in the marked profiles (the Fleet window's *Log in marked*):
    /// one already logged in or logging in is skipped with its reason; one
    /// not loaded yet is loaded first. The per-row report goes to
    /// [`Submitted::bulk`].
    fn login_marked<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        use crate::views::Phase;
        let planned = self.planned_marks();
        let mut rows = match planned.as_ref() {
            Some(marks) => self.gone_marks(Some(marks)),
            None => self.gone_marks(surface.models().marks()),
        };
        let mut eligible = Vec::new();
        for name in self.admitted_names(request) {
            let phase = self.fleet_view().row(&name).map(|row| row.phase());
            let member = self.members().contains(&name);
            let skip = match phase.filter(|_| member) {
                Some(Phase::Ready) => Some((SkipReason::AlreadyLoggedIn, "already logged in")),
                Some(
                    Phase::Preparing
                    | Phase::Waiting
                    | Phase::Queued
                    | Phase::Connecting
                    | Phase::Loading,
                ) => Some((SkipReason::AlreadyLoggingIn, "already logging in")),
                _ => None,
            };
            match skip {
                Some((reason, text)) => {
                    rows.push(crate::BulkRow::new(
                        &name,
                        crate::BulkOutcome::skipped(text),
                    ));
                    let command = OperatorCommand::Login;
                    self.settle_member(
                        request,
                        command,
                        &name,
                        Applied::Skipped(reason),
                        submitted,
                    );
                }
                None => {
                    if !member {
                        let op = self.operations.open(crate::operations::ActionKind::Load);
                        self.load_member(op, &name, surface);
                    }
                    eligible.push(name);
                }
            }
        }
        if !eligible.is_empty() {
            submitted.operation = Some(self.login_members(&eligible, surface));
            rows.extend(
                eligible
                    .into_iter()
                    .map(|name| crate::BulkRow::new(name, crate::BulkOutcome::Done)),
            );
        }
        submitted.set_bulk(crate::bulk::BulkReport::new(
            "Log in marked",
            "logging in",
            rows,
            None,
        ));
    }

    /// Log out the marked profiles that are logged in or logging in (the
    /// Fleet window's *Log out marked*); a queued Start of each is dropped
    /// with the logout. The per-row report goes to [`Submitted::bulk`].
    fn logout_marked<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        use crate::bot_projection::LogoutEligibility;
        let planned = self.planned_marks();
        let mut rows = match planned.as_ref() {
            Some(marks) => self.gone_marks(Some(marks)),
            None => self.gone_marks(surface.models().marks()),
        };
        let mut eligible = Vec::new();
        for name in self.admitted_names(request) {
            let member = self.members().contains(&name);
            let logout = self
                .fleet_view()
                .row(&name)
                .filter(|_| member)
                .map(|row| row.logout_eligibility())
                .unwrap_or(LogoutEligibility::NotLoggedIn);
            if logout.eligible() {
                eligible.push(name);
            } else {
                rows.push(crate::BulkRow::new(
                    &name,
                    crate::BulkOutcome::skipped(LogoutEligibility::REFUSAL),
                ));
                let skipped = Applied::Skipped(SkipReason::NotLoggedIn);
                self.settle_member(request, OperatorCommand::Logout, &name, skipped, submitted);
            }
        }
        if !eligible.is_empty() {
            if let Some(scripts) = surface.models().scripts.as_deref_mut() {
                for name in &eligible {
                    scripts.cancel_queued_as(name, "logged out");
                }
                scripts.publish_start_places(self);
            }
            submitted.operation = Some(self.logout_members(&eligible));
            rows.extend(
                eligible
                    .into_iter()
                    .map(|name| crate::BulkRow::new(name, crate::BulkOutcome::Done)),
            );
        }
        submitted.set_bulk(crate::bulk::BulkReport::new(
            "Log out marked",
            "logging out",
            rows,
            None,
        ));
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
                None => {
                    self.settle_name(request, name, Settlement::Completed(Completion::Done), None)
                }
            }
        }
        self.close_play();
        for slot in slots {
            self.commands.book.slot_retired(slot);
        }
    }

    /// Persist `name`'s world preference for its next handshake (the
    /// profile form's world field), mirrored onto a running slot's arm.
    pub(super) fn set_world_preference(
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
        let identity = self.profile_identity_or_synthetic(name);
        self.own_member_as(request, command, name, identity, op);
    }

    /// [`Self::own_member`] for a member whose identity was read before
    /// its dispatch changed the vault (a deleted profile no longer
    /// resolves its name to its identity).
    fn own_member_as(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        name: &str,
        identity: crate::selection::ProfileIdentity,
        op: OperationId,
    ) {
        let subject = Subject::Profile(identity);
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
                        self.commands.watch_arm(arm);
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
    /// slot's current arm retired its slot. The list is filtered in place
    /// and the slot rows are read in place: a poll with nothing new
    /// allocates nothing, however many relogs wait.
    fn drain_watched(&mut self) {
        if self.commands.watched.is_empty() {
            return;
        }
        let mut watched = std::mem::take(&mut self.commands.watched);
        watched.retain(|arm| {
            loop {
                let (event, _) = arm.take_lifecycle_event();
                let Some(event) = event else {
                    break;
                };
                self.commands.book.observe(&event);
            }
            let slot = arm.slot();
            let current = self.play.as_ref().is_some_and(|play| play.holds_arm(arm));
            if !current {
                self.commands.book.slot_retired(slot);
            }
            self.commands.book.reports().any(|report| {
                report.members().iter().any(|member| {
                    member.scope() == Scope::Slot(slot)
                        && matches!(
                            member.pending(),
                            Some(crate::command::Pending::Posted { .. })
                        )
                })
            })
        });
        debug_assert!(self.commands.watched.is_empty());
        self.commands.watched = watched;
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
        self.advance_world_switches();
        self.commands.settle_log_saves();
        self.settle_pref_writes();
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
            | OperatorCommand::UnlockVault
            | OperatorCommand::CreateVault
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
pub(super) fn terminal_of(
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
#[cfg(test)]
mod setting_event_tests {
    use super::CommandDesk;
    use crate::session::prefs::{SettingEventState, SETTING_EVENTS_CAP};

    #[test]
    fn queue_overflow_is_reported_with_the_drained_batch() {
        let mut commands = CommandDesk::default();
        commands.set_watch_setting_events(true);
        for _ in 0..SETTING_EVENTS_CAP {
            commands.note_setting_event("capture", SettingEventState::Pending, None);
        }
        commands.note_setting_event(
            "capture",
            SettingEventState::Failed,
            Some("disk full".into()),
        );

        let (events, incomplete) = commands.take_setting_events();
        assert_eq!(events.len(), SETTING_EVENTS_CAP);
        assert!(
            incomplete,
            "a dropped terminal failure marks the batch incomplete"
        );
        assert!(
            events
                .iter()
                .all(|event| event.state() == SettingEventState::Pending),
            "the full queue retains its original bounded contents"
        );
        assert_eq!(commands.take_setting_events(), (Vec::new(), false));
    }

    #[test]
    fn explicit_owner_clear_state_survives_the_event_queue() {
        let mut commands = CommandDesk::default();
        commands.set_watch_setting_events(true);
        commands.note_setting_event(
            "capture",
            SettingEventState::Failed,
            Some("disk full".into()),
        );
        commands.note_setting_event("capture", SettingEventState::Cleared, None);

        let (events, incomplete) = commands.take_setting_events();
        assert!(!incomplete);
        assert_eq!(
            events.iter().map(|event| event.state()).collect::<Vec<_>>(),
            [SettingEventState::Failed, SettingEventState::Cleared]
        );
    }
}
