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
    /// Frontend/profile binding epoch, including login/session replacement.
    pub generation: u64,
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
}
impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSelection => "Select a map destination first",
            Self::Blocked => "No walkable tile within radius 16 (or no proven POI stand)",
            Self::NoOrigin => "Walk/Teleport unavailable: no observed player",
            Self::NoFocus => "Walk/Teleport unavailable: no focused running slot",
            Self::Stale => "Map selection expired: focus, lifetime, world or map identity changed",
            Self::NoNavigation => "Navigation unavailable",
            Self::NoPath => "No path to the selected destination with these routing options",
            Self::Unauthorized => "Debug Teleport requires a local loopback engine target",
            Self::WrongAction => "Map confirmation has a different action",
            Self::InvalidCoordinates => "Enter x,z,plane (plane 0–3)",
        })
    }
}
impl std::error::Error for ActionError {}

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
        if self.context == Some(context) {
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
    pub fn select_coordinates(
        &mut self,
        world: &NavWorld,
        input: &str,
    ) -> Result<Option<Tile>, ActionError> {
        let requested = Self::parse_coordinates(input)?;
        let target = self.select_tile(world, requested);
        self.center = [f64::from(requested.x) + 0.5, f64::from(requested.z) + 0.5];
        Ok(target)
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
    /// requested tile. Does not consume the selection.
    pub fn availability(
        &self,
        kind: ActionKind,
        current: &MapContext,
        origin: Option<Tile>,
    ) -> Result<Tile, ActionError> {
        let pending = self.pending.as_ref().ok_or(ActionError::NoSelection)?;
        if pending.context != Some(*current) {
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
    fn check(&self, current: &MapContext, kind: ActionKind) -> Result<(), ActionError> {
        if self.kind != kind {
            return Err(ActionError::WrongAction);
        }
        if self.context != *current {
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
    fn validate_map_command(
        &self,
        command: &MapCommand,
        current: &MapContext,
        kind: ActionKind,
    ) -> Result<&str, ActionError> {
        command.check(current, kind)?;
        self.connection
            .require_bot_operation()
            .map_err(|_| ActionError::Unauthorized)?;
        let name = self.focused.as_deref().ok_or(ActionError::NoFocus)?;
        if current.focus.is_none() || self.map_focus(name) != current.focus {
            return Err(ActionError::Stale);
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
        command.walk_on(world, current, name, state, bank, arms)
    }
    pub fn map_teleport(
        &self,
        command: MapCommand,
        current: &MapContext,
    ) -> Result<(), ActionError> {
        let name = self.validate_map_command(&command, current, ActionKind::Teleport)?;
        let host = match &self.connection {
            crate::play_bootstrap::PlayConnection::Legacy(options) => options.host.as_str(),
            crate::play_bootstrap::PlayConnection::Bound { template, .. } => {
                template.profile().client().game_host()
            }
        };
        debug_authorized(self.connection.target(), host)?;
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
        let queue = queues.get_mut(name).ok_or(ActionError::NoFocus)?;
        queue.push_back(teleport_command(command.destination)?);
        drop(queues);
        drop(statuses);
        self.wake(name);
        Ok(())
    }
}
pub(super) fn debug_authorized(target: client::BotTarget, host: &str) -> Result<(), ActionError> {
    if target != client::BotTarget::Local || !crate::is_loopback_host(host) {
        Err(ActionError::Unauthorized)
    } else {
        Ok(())
    }
}
pub(super) fn teleport_command(t: Tile) -> Result<String, ActionError> {
    if !(0..4).contains(&t.level) || t.x < 0 || t.z < 0 {
        return Err(ActionError::InvalidCoordinates);
    }
    Ok(api::interact::tele_args(t.level, t.x, t.z))
}
