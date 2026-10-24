use std::sync::Arc;

use api::identity::{RequestId, Settlement};
use nav_planner::map::identity::Digest;
use nav_planner::router::FindOptions;
use nav_planner::tile::Tile;
use nav_planner::world::NavWorld;
use nav_planner::WorldState;

use crate::command::{
    CommandFailure, Completion, FailureKind, OperatorCommand, Rejection, SkipReason, Subject,
};
use crate::launch::BoundProfile;
use crate::map_bake::MapBakeGate;
use crate::surface::{InputEv, InputSender};
use crate::walk_map::{
    observed_walk_services, RouteProjectionView, WalkActionError, WalkActionKind, WalkCatalogue,
    WalkExclude, WalkMap, WalkMapContext, WalkMapDemand, WalkMapDemandHandle, WalkMapJobState,
    WalkObservedService, WalkReadyCatalogue, WalkSlotStatus,
};
use crate::OperatorSession;

impl<Io> OperatorSession<Io> {
    /// Bind the map to a navigation identity, catalogue overlay and current
    /// focused slot. The frontend supplies only stable display identities;
    /// host focus tokens are captured here.
    pub fn walk_map_context(
        &self,
        name: Option<&str>,
        nav: Digest,
        overlay: Option<Digest>,
        generation: u64,
    ) -> WalkMapContext {
        let focus = name.and_then(|name| self.play.as_ref()?.map_focus(name));
        WalkMapContext::from_host(host_play::walk_map::MapContext {
            focus,
            nav,
            overlay,
            generation,
        })
    }

    /// One host-authored Walk-send eligibility result.
    pub fn walk_slot_status(&self, name: &str) -> WalkSlotStatus {
        self.play
            .as_ref()
            .map(|play| WalkSlotStatus::from_host(play.walk_eligibility(name)))
            .unwrap_or_else(|| WalkSlotStatus::excluded(WalkExclude::NotLoggedIn))
    }

    /// Whether the bound target allows Debug Teleport.
    pub fn walk_teleport_authorized(&self) -> bool {
        self.play
            .as_ref()
            .is_some_and(|play| play.map_teleport_authorized())
    }

    /// Read the focused map route through the host's live > script > manual
    /// precedence while exposing only the frontend-owned route projection.
    pub fn with_walk_route<R>(
        &self,
        name: &str,
        live: Option<RouteProjectionView<'_>>,
        read: impl FnOnce(Option<RouteProjectionView<'_>>) -> R,
    ) -> R {
        let Some(play) = self.play.as_ref() else {
            return read(live);
        };
        let manual = self.walk_arms().lock().unwrap().get(name).cloned();
        play.with_map_route(
            name,
            manual.as_deref(),
            live.map(RouteProjectionView::to_host),
            |route| read(route.map(RouteProjectionView::from_host)),
        )
    }

    /// Test whether the current focus can use a selected map action without
    /// consuming the pending selection.
    pub fn walk_availability(
        &self,
        map: &WalkMap,
        kind: WalkActionKind,
        context: WalkMapContext,
        name: Option<&str>,
    ) -> Result<Tile, WalkActionError> {
        let origin = name.and_then(|name| self.walk_origin(name));
        map.availability(kind, context, origin)
    }

    /// Share the focused manual walk route with a renderer without cloning
    /// its tile or leg buffers.
    pub fn manual_walk_route(&self, name: &str) -> Option<Arc<nav_planner::router::Route>> {
        let arm = self.walk_arms().lock().unwrap().get(name).cloned()?;
        let route = arm.lock().unwrap().route.clone();
        route
    }

    /// Observed NPC services for the current map binding. Missing snapshot or
    /// stale observations produce no rows, never facts about a replacement.
    pub fn observed_walk_services(
        &self,
        name: &str,
        context: WalkMapContext,
    ) -> Vec<WalkObservedService> {
        self.with_slot_snapshot(name, |snapshot| {
            observed_walk_services(snapshot, context).unwrap_or_default()
        })
        .unwrap_or_default()
    }

    /// Build a frontend-owned catalogue from the map worker's ready lease.
    pub fn walk_catalogue_from_ready(
        &self,
        profile: &BoundProfile,
        world: Arc<NavWorld>,
        ready: Arc<WalkReadyCatalogue>,
    ) -> Result<(WalkCatalogue, bool), crate::walk_map::WalkMapCacheError> {
        WalkCatalogue::from_ready(world, profile, ready)
    }

    pub(crate) fn walk_state(&self, name: Option<&str>) -> WorldState {
        name.and_then(|name| {
            self.with_slot_world_state(name, |_, state| {
                state.clone().with_map_members(self.map_members())
            })
        })
        .unwrap_or_else(|| WorldState::empty().with_map_members(self.map_members()))
    }

    pub(crate) fn walk_risk_input(&self, name: Option<&str>) -> host_play::admission::RiskInput {
        name.and_then(|name| {
            self.with_slot_snapshot(name, |snapshot| {
                host_play::admission::capture(
                    snapshot,
                    self.map_members(),
                    Default::default(),
                    false,
                )
            })
        })
        .unwrap_or_else(|| host_play::admission::unavailable(self.map_members()))
    }

    pub(crate) fn walk_origin(&self, name: &str) -> Option<Tile> {
        self.walk_slot_status(name).origin()
    }
}

/// One map, movement or game-input row of the command seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapRow {
    WalkTo,
    GroupWalk,
    Teleport,
    DebugCommand,
    GameInput,
    OpenDemand,
    ReleaseDemand,
    BakeNow,
    DeclineBake,
}

impl MapRow {
    fn command(self) -> OperatorCommand {
        use OperatorCommand as C;
        match self {
            Self::WalkTo => C::WalkTo,
            Self::GroupWalk => C::GroupWalk,
            Self::Teleport => C::Teleport,
            Self::DebugCommand => C::DebugCommand,
            Self::GameInput => C::GameInput,
            Self::OpenDemand => C::OpenMapDemand,
            Self::ReleaseDemand => C::ReleaseMapDemand,
            Self::BakeNow => C::BakeMapNow,
            Self::DeclineBake => C::DeclineMapBake,
        }
    }
}

/// One already-resolved input for the focused game client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameInputEvent {
    /// A captured panel event sent to the focused slot.
    Captured(InputEv),
    /// A client wire command (TUI movement and dialogue).
    Wire(host_play::WireCmd),
}

impl GameInputEvent {
    /// Stage one client-pathfound walk step without exposing the host command type.
    pub fn walk(x: i32, z: i32, level: i32) -> Self {
        Self::Wire(host_play::WireCmd::Walk { x, z, level })
    }

    /// Stage a request to continue the focused client's dialogue.
    pub fn continue_dialogue() -> Self {
        Self::Wire(host_play::WireCmd::Continue)
    }

    /// Stage the selected dialogue answer for the focused client.
    pub fn answer(option: i32) -> Self {
        Self::Wire(host_play::WireCmd::Answer(option))
    }
}

const GAME_INPUT_CAPACITY: usize = 512;

struct StagedGameInput {
    event: GameInputEvent,
    sender: Option<InputSender>,
}

impl StagedGameInput {
    fn sender<'a>(&'a self, fallback: Option<&'a InputSender>) -> Option<&'a InputSender> {
        self.sender.as_ref().or(fallback)
    }
}

struct GameInputQueue {
    events: [Option<StagedGameInput>; GAME_INPUT_CAPACITY],
    len: usize,
    overflowed: bool,
}

impl Default for GameInputQueue {
    fn default() -> Self {
        Self {
            events: std::array::from_fn(|_| None),
            len: 0,
            overflowed: false,
        }
    }
}

impl GameInputQueue {
    fn clear(&mut self) {
        for event in &mut self.events[..self.len] {
            *event = None;
        }
        self.len = 0;
        self.overflowed = false;
    }

    fn push(&mut self, event: GameInputEvent) -> bool {
        self.push_with_sender(event, None)
    }

    fn push_to(&mut self, event: GameInputEvent, sender: &InputSender) -> bool {
        self.push_with_sender(event, Some(sender.duplicate()))
    }

    fn push_with_sender(&mut self, event: GameInputEvent, sender: Option<InputSender>) -> bool {
        let Some(slot) = self.events.get_mut(self.len) else {
            self.overflowed = true;
            return false;
        };
        *slot = Some(StagedGameInput { event, sender });
        self.len += 1;
        true
    }

    fn is_empty(&self) -> bool {
        self.len == 0 && !self.overflowed
    }

    fn is_overflowed(&self) -> bool {
        self.overflowed
    }

    fn iter(&self) -> impl Iterator<Item = &StagedGameInput> + '_ {
        self.events[..self.len].iter().flatten()
    }
}

struct StagedMapAction {
    context: WalkMapContext,
    command: Result<host_play::walk_map::MapCommand, WalkActionError>,
    destination: Option<Tile>,
    options: FindOptions,
}

struct StagedDebugCommand {
    command: String,
    snapshot: Option<Vec<(crate::selection::ProfileIdentity, String)>>,
    /// A command the front end's own controls issue (the rail cheat
    /// buttons), sent as the host's internal cheat rather than as typed
    /// operator text.
    internal: bool,
}

struct StagedMapDemand {
    profile: BoundProfile,
    demand: WalkMapDemand,
}

/// Map, demand and game-input custody lent to one command dispatch.
///
/// The UI may stage inputs through the public staging methods; only this
/// module commits them to the host.
#[derive(Default)]
pub struct MapDesk {
    walk_dest: Option<Tile>,
    map_demand: Option<WalkMapDemandHandle>,
    demand_kind: Option<WalkMapDemand>,
    bake_gate: MapBakeGate,
    walk: Option<StagedMapAction>,
    group_walk: Option<crate::group_walk::StagedMarkedWalk>,
    teleport: Option<StagedMapAction>,
    debug_command: Option<StagedDebugCommand>,
    staged_demand: Option<StagedMapDemand>,
    staged_bake_profile: Option<BoundProfile>,
    pending_demand: Vec<RequestId>,
    input_sender: Option<InputSender>,
    game_input: GameInputQueue,
}

impl MapDesk {
    /// An empty map desk with the remembered terrain-bake choice.
    pub fn new(choice: crate::MapBakeChoice) -> Self {
        Self {
            bake_gate: MapBakeGate::new(choice),
            ..Self::default()
        }
    }
    /// The accepted WalkTo destination, when a route is armed.
    pub fn walk_dest(&self) -> Option<Tile> {
        self.walk_dest
    }

    /// Seed the route projection in front-end tests without running a WalkTo command.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_walk_dest_for_test(&mut self, destination: Option<Tile>) {
        self.walk_dest = destination;
    }

    /// Clear the route projection when its owning slot lifetime ends.
    pub fn clear_walk_dest(&mut self) {
        self.walk_dest = None;
    }

    /// The held map-cache demand, if one is open.
    pub fn map_demand(&self) -> Option<&WalkMapDemandHandle> {
        self.map_demand.as_ref()
    }

    /// The kind of the held map-cache demand.
    pub fn demand_kind(&self) -> Option<WalkMapDemand> {
        self.demand_kind
    }

    /// Whether a map demand or its owner settlement still needs releasing.
    pub fn needs_release(&self) -> bool {
        self.map_demand.is_some() || !self.pending_demand.is_empty()
    }

    /// The per-process terrain-bake consent gate.
    pub fn bake_gate(&self) -> &MapBakeGate {
        &self.bake_gate
    }

    pub(crate) fn bake_gate_mut(&mut self) -> &mut MapBakeGate {
        &mut self.bake_gate
    }

    /// Note that an adopt-only image demand settled without terrain.
    pub fn note_ready_without_terrain(&mut self, demand: WalkMapDemand) {
        self.bake_gate.note_ready_without_terrain(demand);
    }

    /// Stage one focused routed walk. This consumes only the UI's pending
    /// selection; host routing remains inside the command dispatch.
    pub fn stage_walk_to(
        &mut self,
        map: &mut WalkMap,
        context: WalkMapContext,
        origin: Option<Tile>,
        options: FindOptions,
    ) {
        self.clear_map_confirmation();
        let destination = map
            .pending()
            .map(|selection| selection.reported_destination());
        self.walk = Some(StagedMapAction {
            context,
            command: map.confirm(WalkActionKind::Walk, context, origin, options),
            destination,
            options,
        });
    }

    /// Stage one marked routed walk without asking the host to route yet.
    pub fn stage_group_walk(
        &mut self,
        map: &mut WalkMap,
        context: WalkMapContext,
        options: FindOptions,
    ) {
        self.clear_map_confirmation();
        self.group_walk = Some(crate::group_walk::StagedMarkedWalk::new(
            map, context, options,
        ));
    }

    /// Stage one focused debug teleport.
    pub fn stage_teleport(
        &mut self,
        map: &mut WalkMap,
        context: WalkMapContext,
        origin: Option<Tile>,
        options: FindOptions,
    ) {
        self.clear_map_confirmation();
        let destination = map
            .pending()
            .map(|selection| selection.reported_destination());
        self.teleport = Some(StagedMapAction {
            context,
            command: map.confirm(WalkActionKind::Teleport, context, origin, options),
            destination,
            options,
        });
    }

    /// Stage the profile and demand kind for OpenMapDemand.
    pub fn stage_open_demand(&mut self, profile: BoundProfile, demand: WalkMapDemand) {
        self.staged_demand = Some(StagedMapDemand { profile, demand });
    }

    /// Stage the bound profile for BakeMapNow.
    pub fn stage_bake_now(&mut self, profile: BoundProfile) {
        self.staged_bake_profile = Some(profile);
    }

    /// Stage command text and, for marked sends, the frozen identity/name
    /// snapshot used to account for rows that disappear during admission.
    pub fn stage_debug_command(
        &mut self,
        command: &str,
        snapshot: Option<Vec<(crate::ProfileIdentity, String)>>,
    ) {
        self.debug_command = Some(StagedDebugCommand {
            command: command.to_owned(),
            snapshot,
            internal: false,
        });
    }

    /// Stage a command a front-end control issues (the panel's rail cheat
    /// buttons) for the next DebugCommand on one profile.
    pub fn stage_internal_debug_command(&mut self, command: &str) {
        self.debug_command = Some(StagedDebugCommand {
            command: command.to_owned(),
            snapshot: None,
            internal: true,
        });
    }

    /// Begin one panel capture frame; input is submitted once after capture.
    pub fn begin_game_input_frame(&mut self) {
        self.game_input.clear();
    }

    /// Stage one input event for dispatch in the current frame.
    pub fn stage_game_input(&mut self, event: GameInputEvent) -> bool {
        self.game_input.push(event)
    }

    /// Stage one captured event for the sender that received its press.
    ///
    /// The selected input sender can change before the frame is submitted.
    /// This method keeps the event bound to the original recipient.
    pub fn stage_captured_input_to(&mut self, sender: &InputSender, event: InputEv) -> bool {
        self.game_input
            .push_to(GameInputEvent::Captured(event), sender)
    }

    /// Whether the current frame has input or overflow state to submit.
    pub fn has_staged_game_input(&self) -> bool {
        !self.game_input.is_empty()
    }

    /// Connect the panel capture channel, or detach it when capture ends.
    pub fn set_input_sender(&mut self, sender: Option<InputSender>) {
        self.input_sender = sender;
    }

    /// Whether the panel's focused-slot capture channel is attached.
    pub fn input_connected(&self) -> bool {
        self.input_sender.is_some()
    }

    /// Duplicate the current capture sender for a press-time recipient.
    pub fn duplicate_input_sender(&self) -> Option<InputSender> {
        self.input_sender.as_ref().map(InputSender::duplicate)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn staged_game_input_for_test(&self) -> Vec<(GameInputEvent, Option<&InputSender>)> {
        self.game_input
            .iter()
            .map(|staged| (staged.event, staged.sender.as_ref()))
            .collect()
    }

    fn clear_map_confirmation(&mut self) {
        self.walk = None;
        self.group_walk = None;
        self.teleport = None;
    }
}

impl<Io> OperatorSession<Io> {
    pub(super) fn map_row<S: crate::surface::SlotSurface<Io = Io>>(
        &mut self,
        row: MapRow,
        targets: super::commands::Targets<'_>,
        request: RequestId,
        surface: &mut S,
        submitted: &mut super::commands::Submitted,
    ) {
        if matches!(targets, super::commands::Targets::Application) {
            if self.app_admitted(request) {
                self.map_row_for(row, targets, request, &[], surface, submitted);
            }
            return;
        }
        let names = self.admitted_names(request);
        if !names.is_empty() {
            self.map_row_for(row, targets, request, &names, surface, submitted);
        }
        self.release_names(names);
    }

    /// `row` for the admitted `names` (none for an application target).
    fn map_row_for<S: crate::surface::SlotSurface<Io = Io>>(
        &mut self,
        row: MapRow,
        targets: super::commands::Targets<'_>,
        request: RequestId,
        names: &[String],
        surface: &mut S,
        submitted: &mut super::commands::Submitted,
    ) {
        use super::commands::Applied;

        let command = row.command();
        let application = matches!(targets, super::commands::Targets::Application);
        let mut models = surface.models();
        let Some(desk) = models.map.as_deref_mut() else {
            let refused =
                Applied::Rejected(Rejection::NoModel, Some("map desk unavailable".into()));
            if application {
                self.settle_app(request, command, refused, submitted);
            } else {
                for name in names {
                    self.settle_member(request, command, name, refused.clone(), submitted);
                }
            }
            return;
        };

        if application {
            let applied = match row {
                MapRow::OpenDemand => self.open_map_demand(desk, request),
                MapRow::ReleaseDemand => {
                    if desk.needs_release() {
                        self.release_map_demand(desk);
                        Applied::Done
                    } else {
                        Applied::Skipped(SkipReason::NotReady)
                    }
                }
                MapRow::BakeNow => self.bake_map_now(desk, request),
                MapRow::DeclineBake => {
                    if desk.bake_gate.prompt() == crate::MapBakePrompt::Asking {
                        desk.bake_gate.decline();
                        Applied::Done
                    } else {
                        Applied::Rejected(
                            Rejection::NotApplicable,
                            Some("map bake prompt is not asking".into()),
                        )
                    }
                }
                MapRow::WalkTo => self.walk_to(desk, names, targets),
                MapRow::GroupWalk | MapRow::Teleport | MapRow::DebugCommand | MapRow::GameInput => {
                    Applied::Rejected(
                        Rejection::NotApplicable,
                        Some("profile row has application scope".into()),
                    )
                }
            };
            self.settle_app(request, command, applied, submitted);
            return;
        }

        match row {
            MapRow::WalkTo => {
                let applied = self.walk_to(desk, names, targets);
                for name in names {
                    self.settle_member(request, command, name, applied.clone(), submitted);
                }
            }
            MapRow::GroupWalk => {
                let Some(selection) = models.fleet.as_deref().map(crate::FleetModel::marks) else {
                    let refused = Applied::Rejected(
                        Rejection::NoModel,
                        Some("marked rows unavailable".into()),
                    );
                    for name in names {
                        self.settle_member(request, command, name, refused.clone(), submitted);
                    }
                    return;
                };
                self.group_walk(desk, selection, names, targets, request, submitted);
            }
            MapRow::Teleport => {
                let applied = self.teleport(desk, names, targets);
                for name in names {
                    self.settle_member(request, command, name, applied.clone(), submitted);
                }
            }
            MapRow::DebugCommand => {
                let Some(staged) = desk.debug_command.take() else {
                    for name in names {
                        self.settle_member(
                            request,
                            command,
                            name,
                            Applied::Rejected(Rejection::NotStaged, None),
                            submitted,
                        );
                    }
                    return;
                };
                if matches!(targets, super::commands::Targets::Marked) {
                    let Some(selection) = models.fleet.as_deref().map(crate::FleetModel::marks)
                    else {
                        for name in names {
                            self.settle_member(
                                request,
                                command,
                                name,
                                Applied::Rejected(
                                    Rejection::NoModel,
                                    Some("marked rows unavailable".into()),
                                ),
                                submitted,
                            );
                        }
                        return;
                    };
                    let Some(snapshot) = staged.snapshot else {
                        for name in names {
                            self.settle_member(
                                request,
                                command,
                                name,
                                Applied::Rejected(Rejection::NotStaged, None),
                                submitted,
                            );
                        }
                        return;
                    };
                    let mut results = Vec::new();
                    let report = crate::run_marked_command(selection, snapshot, |name| {
                        let result = self.debug_command(name, &staged.command);
                        results.push((name.to_owned(), result.clone()));
                        result
                    });
                    submitted.set_marked(report);
                    for name in names {
                        let applied = match results.iter().find(|(row, _)| row == name) {
                            Some((_, Ok(()))) => Applied::Done,
                            Some((_, Err(error))) => {
                                Applied::Rejected(Rejection::NotApplicable, Some(error.clone()))
                            }
                            None => Applied::Rejected(Rejection::NotApplicable, None),
                        };
                        self.settle_member(request, command, name, applied, submitted);
                    }
                } else if matches!(
                    targets,
                    super::commands::Targets::Profile(_) | super::commands::Targets::Selected
                ) && names.len() == 1
                {
                    let sent = if staged.internal {
                        self.debug_command_internal(&names[0], &staged.command)
                    } else {
                        self.debug_command(&names[0], &staged.command)
                    };
                    let applied = sent.map_or_else(
                        |error| Applied::Rejected(Rejection::NotApplicable, Some(error)),
                        |_| Applied::Done,
                    );
                    for name in names {
                        self.settle_member(request, command, name, applied.clone(), submitted);
                    }
                } else {
                    for name in names {
                        self.settle_member(
                            request,
                            command,
                            name,
                            Applied::Rejected(Rejection::NotApplicable, None),
                            submitted,
                        );
                    }
                }
            }
            MapRow::GameInput => {
                let applied = self.game_input(desk, names, targets);
                for name in names {
                    self.settle_member(request, command, name, applied.clone(), submitted);
                }
            }
            MapRow::OpenDemand | MapRow::ReleaseDemand | MapRow::BakeNow | MapRow::DeclineBake => {
                for name in names {
                    self.settle_member(
                        request,
                        command,
                        name,
                        Applied::Rejected(Rejection::NotApplicable, None),
                        submitted,
                    );
                }
            }
        }
    }

    fn walk_to(
        &self,
        desk: &mut MapDesk,
        names: &[String],
        targets: super::commands::Targets<'_>,
    ) -> super::commands::Applied {
        use super::commands::Applied;
        let name = match (targets, names) {
            (super::commands::Targets::Profile(_) | super::commands::Targets::Selected, [name]) => {
                Some(name.as_str())
            }
            (super::commands::Targets::Application, []) => None,
            _ => return Applied::Rejected(Rejection::NotApplicable, None),
        };
        let Some(staged) = desk.walk.take() else {
            return Applied::Rejected(Rejection::NotStaged, None);
        };
        let planned = staged.destination;
        match self.confirm_walk(staged, name) {
            Ok(destination) => {
                desk.walk_dest = Some(destination);
                Applied::Done
            }
            Err(error) => {
                if error.kind() == crate::walk_map::WalkActionErrorKind::NoPath {
                    desk.walk_dest = planned;
                }
                if name.is_none() {
                    Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()))
                } else {
                    walk_action_applied(error)
                }
            }
        }
    }

    fn group_walk(
        &mut self,
        desk: &mut MapDesk,
        selection: &crate::MarkedSelection,
        names: &[String],
        targets: super::commands::Targets<'_>,
        request: RequestId,
        submitted: &mut super::commands::Submitted,
    ) {
        use super::commands::Applied;
        let command = OperatorCommand::GroupWalk;
        if !matches!(targets, super::commands::Targets::Marked) {
            for name in names {
                self.settle_member(
                    request,
                    command,
                    name,
                    Applied::Rejected(Rejection::NotApplicable, None),
                    submitted,
                );
            }
            return;
        }
        let Some(staged) = desk.group_walk.take() else {
            for name in names {
                self.settle_member(
                    request,
                    command,
                    name,
                    Applied::Rejected(Rejection::NotStaged, None),
                    submitted,
                );
            }
            return;
        };
        let planned = staged.destination();
        let report = self.confirm_marked_walk(selection, staged);
        if report.done_count() != 0 {
            desk.walk_dest = planned;
        }
        for name in names {
            let applied = match report.rows().iter().find(|row| row.profile() == name) {
                Some(row) if row.outcome().is_done() => Applied::Done,
                Some(row) if row.outcome().is_skipped() => {
                    Applied::Skipped(SkipReason::WalkExcluded)
                }
                Some(row) if row.outcome().is_failed() => Applied::Failed(
                    FailureKind::JobFailed,
                    row.detail()
                        .or_else(|| row.outcome().reason())
                        .map(str::to_owned),
                ),
                _ => Applied::Rejected(Rejection::NotApplicable, None),
            };
            self.settle_member(request, command, name, applied, submitted);
        }
        submitted.set_bulk(report);
    }

    fn teleport(
        &self,
        desk: &mut MapDesk,
        names: &[String],
        targets: super::commands::Targets<'_>,
    ) -> super::commands::Applied {
        use super::commands::Applied;
        if !matches!(
            targets,
            super::commands::Targets::Profile(_) | super::commands::Targets::Selected
        ) || names.len() != 1
        {
            return Applied::Rejected(Rejection::NotApplicable, None);
        }
        let Some(staged) = desk.teleport.take() else {
            return Applied::Rejected(Rejection::NotStaged, None);
        };
        self.confirm_teleport(staged, &names[0])
            .map_or_else(walk_action_applied, |_| Applied::Done)
    }

    fn game_input(
        &self,
        desk: &mut MapDesk,
        names: &[String],
        targets: super::commands::Targets<'_>,
    ) -> super::commands::Applied {
        use super::commands::Applied;
        if !matches!(
            targets,
            super::commands::Targets::Profile(_) | super::commands::Targets::Selected
        ) || names.len() != 1
        {
            desk.game_input.clear();
            return Applied::Rejected(Rejection::NotApplicable, None);
        }
        if desk.game_input.is_empty() {
            return Applied::Rejected(Rejection::NotStaged, None);
        }
        let name = &names[0];
        let mut failure = desk
            .game_input
            .is_overflowed()
            .then(|| "game input frame is full".to_owned());
        for staged in desk.game_input.iter() {
            match staged.event {
                GameInputEvent::Captured(event) => {
                    match staged.sender(desk.input_sender.as_ref()) {
                        Some(sender) if sender.send(event) => {}
                        Some(_) => {
                            failure = Some("capture input channel is disconnected".to_owned());
                            break;
                        }
                        None => {
                            failure = Some("capture input channel is unavailable".to_owned());
                            break;
                        }
                    }
                }
                GameInputEvent::Wire(command) => match self.play.as_ref() {
                    Some(play) => play.queue_wire(name, command),
                    None => {
                        failure = Some("no active play session".to_owned());
                        break;
                    }
                },
            }
        }
        desk.game_input.clear();
        failure.map_or(Applied::Done, |detail| {
            Applied::Rejected(Rejection::NotApplicable, Some(detail))
        })
    }

    fn open_map_demand(
        &mut self,
        desk: &mut MapDesk,
        request: RequestId,
    ) -> super::commands::Applied {
        use super::commands::Applied;
        let Some(staged) = desk.staged_demand.take() else {
            return Applied::Rejected(Rejection::NotStaged, None);
        };
        let demand = staged.demand;
        let handle = match desk.bake_gate.open_profile(&staged.profile, demand) {
            Ok(handle) => handle,
            Err(error) => {
                return Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()));
            }
        };
        desk.map_demand = Some(handle);
        desk.demand_kind = Some(demand);
        demand_applied(desk, request)
    }

    fn bake_map_now(&mut self, desk: &mut MapDesk, request: RequestId) -> super::commands::Applied {
        use super::commands::Applied;
        let Some(profile) = desk.staged_bake_profile.take() else {
            return Applied::Rejected(Rejection::NotStaged, None);
        };
        desk.bake_gate.accept();
        let handle = match desk.bake_gate.open_profile(&profile, WalkMapDemand::Images) {
            Ok(handle) => handle,
            Err(error) => {
                return Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()));
            }
        };
        desk.map_demand = Some(handle);
        desk.demand_kind = Some(WalkMapDemand::Images);
        demand_applied(desk, request)
    }

    fn release_map_demand(&mut self, desk: &mut MapDesk) {
        desk.map_demand = None;
        desk.demand_kind = None;
        for request in std::mem::take(&mut desk.pending_demand) {
            self.settle_later(
                request,
                Subject::Application,
                Settlement::Completed(Completion::Skipped(SkipReason::NotReady)),
                Some("map demand released before it became ready"),
            );
        }
        crate::walk_map::reap_walk_map_demand();
    }

    /// Settle owner-held OpenMapDemand/BakeMapNow rows after their map worker
    /// reaches a terminal state.
    pub fn poll_map_demand(&mut self, desk: &mut MapDesk) {
        let Some(status) = desk.map_demand.as_ref().map(WalkMapDemandHandle::status) else {
            return;
        };
        let state = status.state();
        if matches!(state, WalkMapJobState::Queued | WalkMapJobState::Running) {
            return;
        }
        let requests = std::mem::take(&mut desk.pending_demand);
        match state {
            WalkMapJobState::Ready => {
                for request in requests {
                    self.settle_later(
                        request,
                        Subject::Application,
                        Settlement::Completed(Completion::Done),
                        None,
                    );
                }
            }
            WalkMapJobState::Paused | WalkMapJobState::Cancelled => {
                for request in requests {
                    self.settle_later(
                        request,
                        Subject::Application,
                        Settlement::Completed(Completion::Skipped(SkipReason::NotReady)),
                        Some("map demand did not become ready"),
                    );
                }
            }
            WalkMapJobState::Failed => {
                let detail = status.failure().map(str::to_owned);
                for request in requests {
                    self.settle_later(
                        request,
                        Subject::Application,
                        Settlement::Failed(CommandFailure::Failed(FailureKind::JobFailed)),
                        detail.as_deref(),
                    );
                }
            }
            WalkMapJobState::Queued | WalkMapJobState::Running => {}
        }
    }

    fn confirm_walk(
        &self,
        staged: StagedMapAction,
        name: Option<&str>,
    ) -> Result<Tile, WalkActionError> {
        use host_play::walk_map::WalkRequest;

        let play = self.play.as_ref();
        let profile = play.and_then(|play| play.server_profile());
        let members = profile
            .as_ref()
            .map_or(&host_play::WorldMembersFact::Unknown, |profile| {
                profile.world_members()
            });
        let origin = name.and_then(|name| self.walk_origin(name));
        let state = self.walk_state(name);
        let map_members = state.map_members;
        let bank = name.map_or_else(nav_planner::bank_fetch::BankRows::default, |name| {
            self.bank_rows(name)
        });
        let input = self.walk_risk_input(name);
        let mut command_destination = None;
        let result = WalkRequest {
            slot: name,
            origin,
            destination: staged.destination,
            options: staged.options,
            map_members,
            members,
        }
        .run(|| {
            let command = staged.command.map_err(WalkActionError::into_host)?;
            command_destination = Some(command.destination());
            let play = play.ok_or(host_play::walk_map::ActionError::NoFocus)?;
            play.map_walk(
                command,
                &staged.context.to_host(),
                &state,
                &bank,
                input,
                self.walk_arms(),
            )
        });
        match result {
            Ok(_) => {
                let destination =
                    command_destination.expect("successful map walk consumed a map command");
                if let Some(name) = name {
                    self.frames.clear_step_latch(name);
                }
                let _ = self.frames.take_walk_clear();
                Ok(destination)
            }
            Err(error) => Err(WalkActionError::from_host(error)),
        }
    }

    fn confirm_teleport(
        &self,
        staged: StagedMapAction,
        _name: &str,
    ) -> Result<Tile, WalkActionError> {
        let command = staged.command?;
        let destination = command.destination();
        let play = self
            .play
            .as_ref()
            .ok_or_else(|| WalkActionError::from_host(host_play::walk_map::ActionError::NoFocus))?;
        play.map_teleport(command, &staged.context.to_host())
            .map(|()| destination)
            .map_err(WalkActionError::from_host)
    }
}

fn walk_action_applied(error: WalkActionError) -> super::commands::Applied {
    use super::commands::Applied;
    match error.kind() {
        crate::walk_map::WalkActionErrorKind::NoSelection => {
            Applied::Rejected(Rejection::NotStaged, Some(error.to_string()))
        }
        crate::walk_map::WalkActionErrorKind::NoOrigin => Applied::Skipped(SkipReason::NotLoggedIn),
        crate::walk_map::WalkActionErrorKind::RunningScript => {
            Applied::Skipped(SkipReason::WalkExcluded)
        }
        crate::walk_map::WalkActionErrorKind::NoFocus
        | crate::walk_map::WalkActionErrorKind::Unauthorized
        | crate::walk_map::WalkActionErrorKind::WrongAction => {
            Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()))
        }
        _ => Applied::Failed(FailureKind::JobFailed, Some(error.to_string())),
    }
}

fn demand_applied(desk: &mut MapDesk, request: RequestId) -> super::commands::Applied {
    use super::commands::Applied;
    let Some(handle) = desk.map_demand.as_ref() else {
        return Applied::Rejected(Rejection::NotApplicable, None);
    };
    let status = handle.status();
    match status.state() {
        WalkMapJobState::Ready => Applied::Done,
        WalkMapJobState::Queued | WalkMapJobState::Running => {
            desk.pending_demand.push(request);
            Applied::Later
        }
        WalkMapJobState::Paused | WalkMapJobState::Cancelled => {
            Applied::Skipped(SkipReason::NotReady)
        }
        WalkMapJobState::Failed => {
            Applied::Failed(FailureKind::JobFailed, status.failure().map(str::to_owned))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targeted_game_input_keeps_its_press_recipient_after_focus_changes() {
        let (old, old_probe) = InputSender::probe();
        let (new, new_probe) = InputSender::probe();
        let mut desk = MapDesk::default();
        desk.set_input_sender(Some(old.duplicate()));
        let down = InputEv::Down {
            button: 2,
            x: 10,
            y: 20,
        };
        assert!(desk.stage_captured_input_to(&old, down));
        desk.set_input_sender(Some(new));

        let staged = desk.game_input.iter().next().expect("staged press");
        let sender = staged
            .sender(desk.input_sender.as_ref())
            .expect("the press keeps its original sender");
        assert!(sender.send(down));
        assert_eq!(old_probe.try_iter().collect::<Vec<_>>(), [down]);
        assert!(new_probe.try_iter().next().is_none());
    }

    #[test]
    fn input_queue_overflow_stays_visible_until_frame_reset() {
        let mut desk = MapDesk::default();
        let event = GameInputEvent::Captured(InputEv::Up);
        for _ in 0..GAME_INPUT_CAPACITY {
            assert!(desk.stage_game_input(event));
        }
        assert!(!desk.stage_game_input(event));
        assert!(desk.game_input.is_overflowed());
        assert!(desk.has_staged_game_input());

        desk.begin_game_input_frame();
        assert!(!desk.game_input.is_overflowed());
        assert!(!desk.has_staged_game_input());
    }
}
