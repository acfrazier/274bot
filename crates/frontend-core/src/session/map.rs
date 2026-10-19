use std::sync::Arc;

use nav::bank_fetch::BankRows;
use nav::map::identity::Digest;
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;

use crate::launch::BoundProfile;
use crate::walk_map::{
    observed_walk_services, RouteProjectionView, WalkActionError, WalkActionKind, WalkCatalogue,
    WalkExclude, WalkMap, WalkMapContext, WalkObservedService, WalkReadyCatalogue, WalkSlotStatus,
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
    pub fn manual_walk_route(&self, name: &str) -> Option<Arc<nav::router::Route>> {
        let arm = self.walk_arms().lock().unwrap().get(name).cloned()?;
        let route = arm.lock().unwrap().route.clone();
        route
    }
    /// Consume and dispatch one focused WalkTo. Risk evidence, bank rows,
    /// eligibility, host membership and the arm set are read only here.
    pub fn confirm_walk(
        &self,
        map: &mut WalkMap,
        context: WalkMapContext,
        name: Option<&str>,
        options: FindOptions,
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
        let destination = map.pending().map(|pending| pending.reported_destination());
        let state = self.walk_state(name);
        let map_members = state.map_members;
        let bank = name.map_or_else(BankRows::default, |name| self.bank_rows(name));
        let input = self.walk_risk_input(name);
        let mut command_destination = None;
        let result = WalkRequest {
            slot: name,
            origin,
            destination,
            options,
            map_members,
            members,
        }
        .run(|| {
            let command = map
                .confirm(WalkActionKind::Walk, context, origin, options)
                .map_err(WalkActionError::into_host)?;
            command_destination = Some(command.destination());
            let play = play.ok_or(host_play::walk_map::ActionError::NoFocus)?;
            play.map_walk(
                command,
                &context.to_host(),
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

    /// Consume and dispatch a focused Debug Teleport.
    pub fn confirm_teleport(
        &self,
        map: &mut WalkMap,
        context: WalkMapContext,
        name: Option<&str>,
        options: FindOptions,
    ) -> Result<Tile, WalkActionError> {
        let origin = name.and_then(|name| self.walk_origin(name));
        let command = map.confirm(WalkActionKind::Teleport, context, origin, options)?;
        let destination = command.destination();
        let play = self
            .play
            .as_ref()
            .ok_or_else(|| WalkActionError::from_host(host_play::walk_map::ActionError::NoFocus))?;
        play.map_teleport(command, &context.to_host())
            .map(|()| destination)
            .map_err(WalkActionError::from_host)
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
    fn command(self) -> crate::command::OperatorCommand {
        use crate::command::OperatorCommand as C;
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

/// The map desk a front end lends (placeholder until the map family lands).
#[derive(Debug, Default)]
pub struct MapDesk {}

impl<Io> OperatorSession<Io> {
    pub(super) fn map_row<S: crate::surface::SlotSurface<Io = Io>>(
        &mut self,
        row: MapRow,
        request: api::identity::RequestId,
        _surface: &mut S,
        submitted: &mut super::commands::Submitted,
    ) {
        use super::commands::Applied;
        let command = row.command();
        let applied = || {
            Applied::Rejected(
                crate::command::Rejection::NoModel,
                Some("map rows: in progress".into()),
            )
        };
        if command.subject() == crate::command::SubjectKind::Application {
            if self.app_admitted(request) {
                self.settle_app(request, command, applied(), submitted);
            }
        } else {
            for name in self.admitted_names(request) {
                self.settle_member(request, command, &name, applied(), submitted);
            }
        }
    }
}
