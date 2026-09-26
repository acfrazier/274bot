use std::fmt;
use std::sync::atomic::Ordering;

use nav::map::identity::Digest;
use nav::map::spatial::{snap_walkable, GameTile};
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;

use super::catalogue::{safe_standable, tile, world_tile};
use super::Catalogue;
use crate::{Play, SlotArm, WalkArms};

/// Process-unique slot lifetime plus the operator-selected world epoch. A uid
/// or a reused username alone is not a lifetime. No bot/map state is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusToken {
    lifetime: u64,
    world_epoch: u64,
}
impl FocusToken {
    pub fn capture(arm: &SlotArm) -> Self {
        Self {
            lifetime: arm.queue_owner,
            world_epoch: arm.world_generation.load(Ordering::Acquire),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapContext {
    pub focus: Option<FocusToken>,
    pub nav: Digest,
    pub overlay: Option<Digest>,
    /// Profile binding epoch. Slot login/session replacement is the focus
    /// token, not destination identity.
    pub generation: u64,
}

impl MapContext {
    /// Destination identity: tile/POI binding is nav, overlay, and profile
    /// generation. A focus token is a bot, not part of the destination.
    pub fn dest_eq(&self, other: &Self) -> bool {
        self.nav == other.nav
            && self.overlay == other.overlay
            && self.generation == other.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Walk,
    Teleport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    NoSelection,
    Blocked,
    NoOrigin,
    NoFocus,
    Stale,
    NoNavigation,
    NoPath,
    Unauthorized,
    WrongAction,
    InvalidCoordinates,
    RunningScript,
}
impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSelection => "Select a map destination first",
            Self::Blocked => "No walkable tile within radius 16 (or no proven POI stand)",
            Self::NoOrigin => "Walk/Teleport unavailable: no observed player",
            Self::NoFocus => "Walk/Teleport unavailable: no focused running slot",
            Self::Stale => "Map selection expired: nav identity or map binding changed",
            Self::NoNavigation => "Navigation unavailable",
            Self::NoPath => "No path to the selected destination with these routing options",
            Self::Unauthorized => "Debug Teleport requires a local loopback engine target",
            Self::WrongAction => "Map confirmation has a different action",
            Self::InvalidCoordinates => "Enter x,z,plane (plane 0–3)",
            Self::RunningScript => "running a script (stop the script to include it)",
        })
    }
}
impl std::error::Error for ActionError {}

/// Why a slot cannot take a group Walk. The panel only renders these codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkExclude {
    NotLoggedIn,
    NoPosition,
    RunningScript,
}

impl fmt::Display for WalkExclude {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl WalkExclude {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotLoggedIn => "not logged in",
            Self::NoPosition => "no position yet",
            Self::RunningScript => "running a script (stop the script to include it)",
        }
    }
}

/// Observed origin for an eligible Walk target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkSlotReady {
    pub origin: Tile,
}

/// Host eligibility for one slot. The panel greys `Excluded` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkSlotStatus {
    Eligible(WalkSlotReady),
    Excluded(WalkExclude),
}

impl WalkSlotStatus {
    pub fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible(_))
    }
}

/// One named slot for [`Play::map_walk_group`].
#[derive(Debug, Clone, Copy)]
pub struct WalkSlotRequest<'a> {
    pub name: &'a str,
    pub state: &'a WorldState,
    pub bank: &'a [(i32, i32)],
}

/// Shared destination after the selection is consumed. Each bot gets its own
/// [`MapCommand`] from this plan (own origin and routing options).
#[derive(Debug, Clone, Copy)]
pub struct MapWalkPlan {
    dest: MapContext,
    destination: Tile,
    options: FindOptions,
}

impl MapWalkPlan {
    pub fn destination(&self) -> Tile {
        self.destination
    }

    pub fn options(&self) -> FindOptions {
        self.options
    }

    /// One guarded Walk command for `name`. `origin` is that bot's observed tile.
    pub fn command(self, name: &str, origin: Tile) -> MapCommand {
        MapCommand {
            kind: ActionKind::Walk,
            context: MapContext {
                focus: None,
                nav: self.dest.nav,
                overlay: self.dest.overlay,
                generation: self.dest.generation,
            },
            origin,
            destination: self.destination,
            options: self.options,
            slot: name.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkSlotOutcomeKind {
    Walking,
    Excluded(WalkExclude),
    Failed(ActionError),
}

impl WalkSlotOutcomeKind {
    fn summary_reason(self) -> Option<&'static str> {
        match self {
            Self::Walking => None,
            Self::Excluded(WalkExclude::NotLoggedIn) => Some("not logged in"),
            Self::Excluded(WalkExclude::NoPosition) => Some("no position yet"),
            Self::Excluded(WalkExclude::RunningScript) => Some("running a script"),
            Self::Failed(ActionError::NoPath) => Some("no path"),
            Self::Failed(ActionError::Stale) => Some("stale"),
            Self::Failed(_) => Some("failed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkSlotOutcome {
    pub name: String,
    pub kind: WalkSlotOutcomeKind,
}

/// Per-bot Walk results. Summary matches Start all: `4 walking, 1 no path: bot3`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupWalkReport {
    pub outcomes: Vec<WalkSlotOutcome>,
}

impl GroupWalkReport {
    pub fn walking_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o.kind, WalkSlotOutcomeKind::Walking))
            .count()
    }

    pub fn summary(&self) -> String {
        let walking = self.walking_count();
        let mut parts = vec![format!("{walking} walking")];
        let mut seen: Vec<(&'static str, Vec<&str>)> = Vec::new();
        for outcome in &self.outcomes {
            let Some(reason) = outcome.kind.summary_reason() else {
                continue;
            };
            if let Some((_, names)) = seen.iter_mut().find(|(r, _)| *r == reason) {
                names.push(outcome.name.as_str());
            } else {
                seen.push((reason, vec![outcome.name.as_str()]));
            }
        }
        for (reason, names) in seen {
            parts.push(format!("{} {reason}: {}", names.len(), names.join(", ")));
        }
        parts.join(", ")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Layers {
    pub terrain: bool,
    pub banks: bool,
    pub transports: bool,
    pub teleports: bool,
    pub labels: bool,
    pub other_pois: bool,
    pub doors: bool,
    pub dots: bool,
    pub collision: bool,
    pub reach: bool,
}
impl Default for Layers {
    fn default() -> Self {
        Self {
            terrain: true,
            banks: true,
            transports: true,
            teleports: true,
            labels: true,
            other_pois: false,
            doors: false,
            dots: false,
            collision: false,
            reach: false,
        }
    }
}
/// One map click or search. `requested` is the exact tile; `target` is the
/// optional radius-16 walk snap. Walk needs `target`. Debug Teleport uses
/// `requested`, including blocked ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub requested: Tile,
    pub target: Option<Tile>,
    pub poi: Option<usize>,
    context: Option<MapContext>,
}
/// One operator view, independent of routing policy and independent of bots.
pub struct MapModel {
    pub center: [f64; 2],
    pub scale: f64,
    pub plane: u8,
    pub layers: Layers,
    context: Option<MapContext>,
    pending: Option<Selection>,
}
impl Default for MapModel {
    fn default() -> Self {
        Self {
            center: [3222.5, 3218.5],
            scale: 4.0,
            plane: 0,
            layers: Layers::default(),
            context: None,
            pending: None,
        }
    }
}
impl MapModel {
    pub fn bind(&mut self, context: MapContext) -> bool {
        if self.context.is_some_and(|c| c.dest_eq(&context)) {
            self.context = Some(context);
            return false;
        }
        self.context = Some(context);
        self.clear_selection();
        true
    }
    pub fn context(&self) -> Option<MapContext> {
        self.context
    }
    pub fn pending(&self) -> Option<&Selection> {
        self.pending.as_ref()
    }
    pub fn clear_selection(&mut self) {
        self.pending = None;
    }
    pub fn close(&mut self) {
        self.pending = None;
        self.context = None;
    }
    pub fn set_plane(&mut self, plane: u8) -> bool {
        if plane >= 4 || self.plane == plane {
            return false;
        }
        self.plane = plane;
        self.clear_selection();
        true
    }
    pub fn recenter(&mut self, observed: Option<Tile>) {
        self.clear_selection();
        let t = observed
            .filter(|t| (0..4).contains(&t.level))
            .unwrap_or(Tile {
                x: 3222,
                z: 3218,
                level: 0,
            });
        self.center = [f64::from(t.x) + 0.5, f64::from(t.z) + 0.5];
        self.plane = t.level as u8;
    }
    pub fn select_tile(&mut self, world: &NavWorld, requested: Tile) -> Option<Tile> {
        let target = u8::try_from(requested.level)
            .ok()
            .and_then(|plane| {
                snap_walkable(
                    GameTile {
                        x: requested.x,
                        z: requested.z,
                        plane,
                    },
                    |t| {
                        let t = t.into();
                        safe_standable(world, t) && world.collision.walkable(t)
                    },
                )
            })
            .map(|t| tile(t.into()));
        if (0..4).contains(&requested.level) {
            self.plane = requested.level as u8;
        }
        self.pending = Some(Selection {
            requested,
            target,
            poi: None,
            context: self.context,
        });
        target
    }
    pub fn parse_coordinates(input: &str) -> Result<Tile, ActionError> {
        let mut parts = input
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty());
        let mut next = || {
            parts
                .next()
                .and_then(|s| s.parse::<i32>().ok())
                .ok_or(ActionError::InvalidCoordinates)
        };
        let t = Tile {
            x: next()?,
            z: next()?,
            level: next()?,
        };
        if parts.next().is_some() || !(0..4).contains(&t.level) {
            return Err(ActionError::InvalidCoordinates);
        }
        Ok(t)
    }
    pub fn select_poi(&mut self, catalogue: &Catalogue, index: usize) -> Result<(), ActionError> {
        if self
            .context
            .is_none_or(|c| c.nav != catalogue.nav_identity() || c.overlay != Some(catalogue.key()))
        {
            self.clear_selection();
            return Err(ActionError::Stale);
        }
        let entry = catalogue.entry(index).ok_or(ActionError::NoSelection)?;
        let requested = entry.anchor();
        self.plane = requested.level as u8;
        self.center = [f64::from(requested.x) + 0.5, f64::from(requested.z) + 0.5];
        // Never radius-snap a label/annotation: it remains a view jump unless
        // the actual anchor (or the physical entity's separate stand) is valid.
        self.pending = Some(Selection {
            requested,
            target: entry.walk_target(),
            poi: Some(index),
            context: self.context,
        });
        Ok(())
    }
    /// Walk destination is the snapped target. Teleport destination is the
    /// requested tile. Does not consume the selection. Destination identity is
    /// nav/overlay/generation. Walk/Teleport use the bot focused at confirm.
    pub fn availability(
        &self,
        kind: ActionKind,
        current: &MapContext,
        origin: Option<Tile>,
    ) -> Result<Tile, ActionError> {
        let pending = self.pending.as_ref().ok_or(ActionError::NoSelection)?;
        if !pending.context.is_some_and(|c| c.dest_eq(current)) {
            return Err(ActionError::Stale);
        }
        let destination = match kind {
            ActionKind::Walk => pending.target.ok_or(ActionError::Blocked)?,
            ActionKind::Teleport => {
                let t = pending.requested;
                if !(0..4).contains(&t.level) || t.x < 0 || t.z < 0 {
                    return Err(ActionError::InvalidCoordinates);
                }
                t
            }
        };
        if origin.is_none_or(|t| !(0..4).contains(&t.level)) {
            return Err(ActionError::NoOrigin);
        }
        current.focus.ok_or(ActionError::NoFocus)?;
        Ok(destination)
    }
    /// Consume even a refused confirmation; no login/focus change can execute a
    /// latent click. MapCommand is deliberately not Clone/Copy.
    /// Walk uses the snapped target (Blocked on a miss). Teleport uses the
    /// requested tile, including blocked ground.
    pub fn confirm(
        &mut self,
        kind: ActionKind,
        current: &MapContext,
        origin: Option<Tile>,
        options: FindOptions,
    ) -> Result<MapCommand, ActionError> {
        let result = self.availability(kind, current, origin);
        self.pending = None;
        let destination = result?;
        Ok(MapCommand {
            kind,
            context: *current,
            origin: origin.ok_or(ActionError::NoOrigin)?,
            destination,
            options,
            slot: String::new(),
        })
    }

    /// Consume the pending Walk destination once. Each bot then gets its own
    /// command from the returned plan. Origin is per-bot, not part of dest.
    pub fn confirm_walk_plan(
        &mut self,
        current: &MapContext,
        options: FindOptions,
    ) -> Result<MapWalkPlan, ActionError> {
        let pending = self.pending.take().ok_or(ActionError::NoSelection)?;
        if !pending.context.is_some_and(|c| c.dest_eq(current)) {
            return Err(ActionError::Stale);
        }
        let destination = pending.target.ok_or(ActionError::Blocked)?;
        Ok(MapWalkPlan {
            dest: MapContext {
                focus: None,
                nav: current.nav,
                overlay: current.overlay,
                generation: current.generation,
            },
            destination,
            options,
        })
    }
}

#[derive(Debug)]
pub struct MapCommand {
    kind: ActionKind,
    context: MapContext,
    origin: Tile,
    destination: Tile,
    options: FindOptions,
    slot: String,
}
impl MapCommand {
    pub fn destination(&self) -> Tile {
        self.destination
    }
    pub fn options(&self) -> FindOptions {
        self.options
    }
    pub fn context(&self) -> MapContext {
        self.context
    }
    pub fn origin(&self) -> Tile {
        self.origin
    }
    fn slot_name<'a>(&'a self, focused: Option<&'a str>) -> Result<&'a str, ActionError> {
        if self.slot.is_empty() {
            focused.ok_or(ActionError::NoFocus)
        } else {
            Ok(self.slot.as_str())
        }
    }
    fn check(&self, current: &MapContext, kind: ActionKind) -> Result<(), ActionError> {
        if self.kind != kind {
            return Err(ActionError::WrongAction);
        }
        if !self.context.dest_eq(current) {
            return Err(ActionError::Stale);
        }
        Ok(())
    }
    /// Shared offline/host seam. Production frontends use Play::map_walk, which
    /// additionally verifies the actual running slot and bound nav world.
    pub fn walk_on(
        self,
        world: &NavWorld,
        current: &MapContext,
        name: &str,
        state: &WorldState,
        bank: &[(i32, i32)],
        arms: &WalkArms,
    ) -> Result<Route, ActionError> {
        self.check(current, ActionKind::Walk)?;
        if !safe_standable(world, world_tile(self.origin)) {
            return Err(ActionError::NoOrigin);
        }
        if !safe_standable(world, world_tile(self.destination)) {
            return Err(ActionError::Blocked);
        }
        crate::arm_walk_on(
            world,
            self.origin,
            self.destination,
            self.options,
            state,
            bank,
            arms,
            Some(name),
        )
        .map_err(|_| ActionError::NoPath)
    }
}
impl Play {
    pub fn map_focus(&self, name: &str) -> Option<FocusToken> {
        self.arms
            .get(name)
            .filter(|arm| !arm.stop.load(Ordering::Acquire))
            .map(|arm| FocusToken::capture(arm))
    }

    fn script_blocks_walk(&self, name: &str) -> bool {
        matches!(
            self.script_state(name),
            script::RunState::Starting
                | script::RunState::Running
                | script::RunState::Paused
                | script::RunState::Stopping
        )
    }

    fn slot_status(&self, name: &str) -> Option<crate::SlotStatus> {
        crate::play_status::lock_statuses(&self.statuses)
            .iter()
            .find(|s| s.username == name)
            .cloned()
    }

    /// One place for Walk eligibility and its reason codes. The panel renders
    /// them; it does not decide them.
    pub fn walk_eligibility(&self, name: &str) -> WalkSlotStatus {
        if self.map_focus(name).is_none() {
            return WalkSlotStatus::Excluded(WalkExclude::NotLoggedIn);
        }
        let Some(status) = self.slot_status(name) else {
            return WalkSlotStatus::Excluded(WalkExclude::NotLoggedIn);
        };
        if !status.connected && !status.ingame {
            return WalkSlotStatus::Excluded(WalkExclude::NotLoggedIn);
        }
        let Some((x, z, level)) = status.ready_tile() else {
            return WalkSlotStatus::Excluded(WalkExclude::NoPosition);
        };
        if self.script_blocks_walk(name) {
            return WalkSlotStatus::Excluded(WalkExclude::RunningScript);
        }
        WalkSlotStatus::Eligible(WalkSlotReady {
            origin: Tile { x, z, level },
        })
    }

    fn validate_map_command(
        &self,
        command: &MapCommand,
        current: &MapContext,
        kind: ActionKind,
    ) -> Result<String, ActionError> {
        command.check(current, kind)?;
        self.connection
            .require_bot_operation()
            .map_err(|_| ActionError::Unauthorized)?;
        let name = command.slot_name(self.focused.as_deref())?.to_string();
        if self.map_focus(&name).is_none() {
            return Err(ActionError::NoFocus);
        }
        if kind == ActionKind::Walk && self.script_blocks_walk(&name) {
            return Err(ActionError::RunningScript);
        }
        let statuses = crate::play_status::lock_statuses(&self.statuses);
        if !statuses
            .iter()
            .any(|s| s.username == name && s.ready_tile().is_some())
        {
            return Err(ActionError::NoOrigin);
        }
        if let Some(identity) = self.server_profile().and_then(|p| p.nav_identity()) {
            if Digest::from_hex(&identity.nav_sha256).ok() != Some(current.nav) {
                return Err(ActionError::Stale);
            }
        }
        Ok(name)
    }
    pub fn map_walk(
        &self,
        command: MapCommand,
        current: &MapContext,
        state: &WorldState,
        bank: &[(i32, i32)],
        arms: &WalkArms,
    ) -> Result<Route, ActionError> {
        let name = self.validate_map_command(&command, current, ActionKind::Walk)?;
        let world = self.world.as_deref().ok_or(ActionError::NoNavigation)?;
        command.walk_on(world, current, &name, state, bank, arms)
    }

    /// Consume a destination plan into one `map_walk` per requested slot.
    /// Ineligible slots are excluded with host reason codes; eligible slots
    /// each get their own command and origin.
    pub fn map_walk_group(
        &self,
        plan: MapWalkPlan,
        dest: &MapContext,
        slots: &[WalkSlotRequest<'_>],
        arms: &WalkArms,
    ) -> GroupWalkReport {
        let mut outcomes = Vec::with_capacity(slots.len());
        for req in slots {
            let status = self.walk_eligibility(req.name);
            let kind = match status {
                WalkSlotStatus::Excluded(reason) => WalkSlotOutcomeKind::Excluded(reason),
                WalkSlotStatus::Eligible(ready) => {
                    let command = plan.command(req.name, ready.origin);
                    let current = MapContext {
                        focus: self.map_focus(req.name),
                        nav: dest.nav,
                        overlay: dest.overlay,
                        generation: dest.generation,
                    };
                    match self.map_walk(command, &current, req.state, req.bank, arms) {
                        Ok(_) => WalkSlotOutcomeKind::Walking,
                        Err(error) => WalkSlotOutcomeKind::Failed(error),
                    }
                }
            };
            outcomes.push(WalkSlotOutcome {
                name: req.name.to_string(),
                kind,
            });
        }
        GroupWalkReport { outcomes }
    }

    /// Same Local+loopback rule as [`crate::walk_map::debug_teleport_authorized`].
    pub fn map_teleport_authorized(&self) -> bool {
        crate::walk_map::debug_teleport_authorized(
            self.connection.target(),
            self.connection.game_host(),
        )
    }
    pub fn map_teleport(
        &self,
        command: MapCommand,
        current: &MapContext,
    ) -> Result<(), ActionError> {
        let name = self.validate_map_command(&command, current, ActionKind::Teleport)?;
        debug_authorized(self.connection.target(), self.connection.game_host())?;
        // Keep the existing CLIENT_CHEAT queue and its session lock. Recheck and
        // enqueue together, so disconnect cannot clear it then receive old work.
        let statuses = crate::play_status::lock_statuses(&self.statuses);
        if !statuses
            .iter()
            .any(|s| s.username == name && s.ready_tile().is_some())
        {
            return Err(ActionError::NoOrigin);
        }
        let mut queues = self.cheats.lock().unwrap();
        let queue = queues.get_mut(&name).ok_or(ActionError::NoFocus)?;
        queue.push_back(teleport_command(command.destination)?);
        drop(queues);
        drop(statuses);
        self.wake(&name);
        Ok(())
    }
}
pub(super) fn debug_authorized(target: client::BotTarget, host: &str) -> Result<(), ActionError> {
    if crate::walk_map::debug_teleport_authorized(target, host) {
        Ok(())
    } else {
        Err(ActionError::Unauthorized)
    }
}
pub(super) fn teleport_command(t: Tile) -> Result<String, ActionError> {
    if !(0..4).contains(&t.level) || t.x < 0 || t.z < 0 {
        return Err(ActionError::InvalidCoordinates);
    }
    Ok(api::interact::tele_args(t.level, t.x, t.z))
}
