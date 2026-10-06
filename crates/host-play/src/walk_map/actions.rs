use std::fmt;
use std::sync::atomic::Ordering;

use api::snapshot::WorldTile;
use nav::bank_fetch::{plan_bank_fetch, BankRows, BankStep};
use nav::map::identity::Digest;
use nav::map::spatial::{snap_walkable, GameTile};
use nav::router::{FindOptions, Leg, Route};
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;

use super::catalogue::{safe_standable, tile, world_tile};
use super::Catalogue;
use crate::{Play, SlotArm, WalkArms};
pub(crate) const LEGACY_ZONES_DETAIL: &str = "zones: unavailable (legacy grid pack)";

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    NoSelection,
    Blocked,
    NoOrigin,
    OriginNotStandable,
    NoFocus,
    Stale,
    NoNavigation,
    NoPath,
    BlockedByZones { detail: Option<String> },
    InsufficientItems { detail: String },
    MembersOnly,
    Unauthorized,
    WrongAction,
    InvalidCoordinates,
    RunningScript,
}
impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSelection => f.write_str("Select a map destination first"),
            Self::Blocked => {
                f.write_str("No walkable tile within radius 16 (or no proven POI stand)")
            }
            Self::NoOrigin => f.write_str("Walk/Teleport unavailable: no observed player"),
            Self::OriginNotStandable => {
                f.write_str("Walk unavailable: observed player tile is not standable")
            }
            Self::NoFocus => f.write_str("Walk/Teleport unavailable: no focused running slot"),
            Self::Stale => {
                f.write_str("Map selection expired: nav identity or map binding changed")
            }
            Self::NoNavigation => f.write_str("Navigation unavailable"),
            Self::NoPath => {
                f.write_str("No path to the selected destination with these routing options")
            }
            Self::BlockedByZones { detail: Some(detail) } => write!(
                f,
                "{detail}; tick \"Route through danger zones\" to walk anyway"
            ),
            Self::BlockedByZones { detail: None } => f.write_str(
                "No path without crossing a danger zone; tick \"Route through danger zones\" to walk anyway",
            ),
            Self::InsufficientItems { detail } => write!(f, "Insufficient route supplies: {detail}"),
            Self::MembersOnly => f.write_str("This route requires a members' world"),
            Self::Unauthorized => {
                f.write_str("Debug Teleport requires a local loopback engine target")
            }
            Self::WrongAction => f.write_str("Map confirmation has a different action"),
            Self::InvalidCoordinates => f.write_str("Enter x,z,plane (plane 0–3)"),
            Self::RunningScript => f.write_str("running a script (stop the script to include it)"),
        }
    }
}
impl ActionError {
    /// The short reason a bulk report names for a slot this refused.
    pub fn short(&self) -> &'static str {
        match self {
            Self::NoSelection => "no destination selected",
            Self::Blocked => "destination blocked",
            Self::NoOrigin => "no position yet",
            Self::OriginNotStandable => "standing on a blocked tile",
            Self::NoFocus => "not logged in",
            Self::Stale => "stale",
            Self::NoNavigation => "navigation unavailable",
            Self::NoPath => "no path",
            Self::BlockedByZones { .. } => "blocked by danger zones",
            Self::InsufficientItems { .. } => "insufficient route supplies",
            Self::MembersOnly => "members-only path",
            Self::Unauthorized => "not authorised",
            Self::WrongAction => "wrong action",
            Self::InvalidCoordinates => "invalid coordinates",
            Self::RunningScript => "running a script",
        }
    }
}
impl std::error::Error for ActionError {}

pub(crate) fn route_supply_shortfall_detail(
    world: &NavWorld,
    state: &WorldState,
    requirements: impl IntoIterator<Item = (i32, i32)>,
) -> Option<String> {
    let shortfalls: Vec<_> = requirements
        .into_iter()
        .filter_map(|(id, count)| {
            let carried = state.inv.get(&id).copied().unwrap_or(0);
            let short = count.saturating_sub(carried);
            (short > 0).then(|| {
                let name = world
                    .transport_item_name(id)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("item #{id}"));
                format!("{short} more {name} (need {count}, carrying {carried})")
            })
        })
        .collect();
    (!shortfalls.is_empty()).then(|| shortfalls.join("; "))
}

/// One operator WalkTo request, including refusals before a command can be
/// built. Frontends and group dispatch use this boundary once per requested
/// slot, never from availability checks or the traveller's frame pump.
pub struct WalkRequest<'a> {
    pub slot: Option<&'a str>,
    pub origin: Option<Tile>,
    pub destination: Option<Tile>,
    pub options: FindOptions,
    pub map_members: bool,
    pub members: &'a crate::WorldMembersFact,
}

impl WalkRequest<'_> {
    pub fn run(
        self,
        action: impl FnOnce() -> Result<Route, ActionError>,
    ) -> Result<Route, ActionError> {
        let outcome = action();
        if api::hostlog::enabled(api::hostlog::Category::NavEvent) {
            api::hostlog::emit(
                api::hostlog::Emit {
                    category: api::hostlog::Category::NavEvent,
                    level: api::hostlog::Level::Info,
                    slot: self.slot,
                    always_stderr: false,
                },
                format_args!(
                    "WalkTo origin={} destination={} teleports={} wilderness={} bank_fetch={} map_members={} members_source={} {}",
                    WalkTile(self.origin),
                    WalkTile(self.destination),
                    self.options.allow_teleports,
                    self.options.allow_wilderness,
                    self.options.allow_bank_fetch,
                    self.map_members,
                    MembersSource(self.members),
                    WalkOutcome(&outcome),
                ),
            );
        }
        if let Err(reason) = outcome.as_ref() {
            emit_walk_rejected(self.slot, self.destination, self.origin, reason);
        }
        outcome
    }

    /// A group confirmation refused before dispatch still gets one receipt per
    /// selected slot. Both frontends use the host's current observed origins.
    pub fn refuse_group(
        play: Option<&crate::Play>,
        names: &[String],
        destination: Option<Tile>,
        options: FindOptions,
        members: &crate::WorldMembersFact,
        error: ActionError,
    ) {
        let origins: Vec<_> = {
            let statuses = play.map(|p| crate::play_status::lock_statuses(&p.statuses));
            names
                .iter()
                .map(|name| {
                    statuses.as_ref().and_then(|rows| {
                        rows.iter()
                            .find(|s| s.username == *name)
                            .and_then(|s| s.ready_tile())
                            .map(|(x, z, level)| Tile { x, z, level })
                    })
                })
                .collect()
        };
        // A sink may read slot status itself; never invoke it under that lock.
        for (name, origin) in names.iter().zip(origins) {
            let _ = WalkRequest {
                slot: Some(name),
                origin,
                destination,
                options,
                map_members: members.map_members(),
                members,
            }
            .run(|| Err(error.clone()));
        }
    }
}

struct MembersSource<'a>(&'a crate::WorldMembersFact);
impl fmt::Display for MembersSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::{WorldMembersFact, WorldMembersSource};
        f.write_str(match self.0 {
            WorldMembersFact::Unknown => "unknown",
            WorldMembersFact::Known { source, .. } => match source {
                WorldMembersSource::ExplicitOverride => "explicit",
                WorldMembersSource::Rs2b2tWorlds => "rs2b2t",
                WorldMembersSource::LocalWorldJson { .. } => "local-world-json",
            },
        })
    }
}

struct WalkTile(Option<Tile>);
impl fmt::Display for WalkTile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(tile) => write!(f, "({},{},{})", tile.x, tile.z, tile.level),
            None => f.write_str("unknown"),
        }
    }
}

struct WalkOutcome<'a>(&'a Result<Route, ActionError>);
impl fmt::Display for WalkOutcome<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let route = match self.0 {
            Ok(route) => route,
            Err(reason) => return write!(f, "refused={reason:?}"),
        };
        let steps: usize = route
            .legs
            .iter()
            .map(|leg| match leg {
                nav::router::Leg::Walk { tiles } => tiles.len().saturating_sub(1),
                nav::router::Leg::Transport { .. } => 0,
            })
            .sum();
        write!(
            f,
            "success legs={} walk_steps={steps} transports=[",
            route.legs.len()
        )?;
        let mut separator = "";
        for leg in &route.legs {
            if let nav::router::Leg::Transport { edge } = leg {
                write!(
                    f,
                    "{separator}{:?}:{}@({},{},{})",
                    edge.kind, edge.loc_id, edge.at.x, edge.at.z, edge.at.level
                )?;
                separator = ",";
            }
        }
        f.write_str("]")
    }
}

/// Emit the one terminal receipt for an armed operator WalkTo. The caller
/// supplies the exact [`nav::traveller::TravelOutcome`] before clearing the
/// route; the original route supplies the failing leg's transport identity.
pub(crate) fn emit_walk_terminal(
    slot: Option<&str>,
    destination: WorldTile,
    outcome: &nav::traveller::TravelOutcome,
    leg: Option<usize>,
    route: &Route,
    legacy_zones_unavailable: bool,
) {
    if !api::hostlog::enabled(api::hostlog::Category::NavEvent) {
        return;
    }
    api::hostlog::emit(
        api::hostlog::Emit {
            category: api::hostlog::Category::NavEvent,
            level: api::hostlog::Level::Info,
            slot,
            always_stderr: false,
        },
        format_args!(
            "WalkTo outcome={} destination={} at={} reason={} leg={} transport={}{}",
            TerminalStatus(outcome),
            WorldWalkTile(Some(destination)),
            WorldWalkTile(Some(terminal_at(outcome))),
            TerminalReason(outcome),
            LegNumber(leg),
            TerminalTransport { route, leg },
            if legacy_zones_unavailable {
                " zones: unavailable (legacy grid pack)"
            } else {
                ""
            },
        ),
    );
}

/// Close an armed operator WalkTo that was replaced or lost with its session.
pub(crate) fn emit_walk_cancelled(
    slot: Option<&str>,
    destination: WorldTile,
    at: Option<WorldTile>,
    route_generation: u64,
    reason: &'static str,
) {
    if !api::hostlog::enabled(api::hostlog::Category::NavEvent) {
        return;
    }
    api::hostlog::emit(
        api::hostlog::Emit {
            category: api::hostlog::Category::NavEvent,
            level: api::hostlog::Level::Info,
            slot,
            always_stderr: false,
        },
        format_args!(
            "WalkTo outcome=cancelled destination={} at={} reason={} leg=- transport=- route_generation={}{}",
            WorldWalkTile(Some(destination)),
            WorldWalkTile(at),
            reason,
            route_generation,
            if reason == "UserInput" { " (cancelled by user input)" } else { "" },
        ),
    );
}

/// Close an operator WalkTo whose host-side prerequisite phase failed before
/// the traveller could produce a [`nav::traveller::TravelOutcome`].
pub(crate) fn emit_walk_aborted(
    slot: Option<&str>,
    destination: WorldTile,
    at: Option<WorldTile>,
    reason: &str,
    legacy_zones_unavailable: bool,
) {
    if !api::hostlog::enabled(api::hostlog::Category::NavEvent) {
        return;
    }
    api::hostlog::emit(
        api::hostlog::Emit {
            category: api::hostlog::Category::NavEvent,
            level: api::hostlog::Level::Info,
            slot,
            always_stderr: false,
        },
        format_args!(
            "WalkTo outcome=aborted destination={} at={} reason={} leg=- transport=-{}",
            WorldWalkTile(Some(destination)),
            WorldWalkTile(at),
            reason,
            if legacy_zones_unavailable {
                " zones: unavailable (legacy grid pack)"
            } else {
                ""
            },
        ),
    );
}

fn emit_walk_rejected(
    slot: Option<&str>,
    destination: Option<Tile>,
    at: Option<Tile>,
    reason: &ActionError,
) {
    if !api::hostlog::enabled(api::hostlog::Category::NavEvent) {
        return;
    }
    api::hostlog::emit(
        api::hostlog::Emit {
            category: api::hostlog::Category::NavEvent,
            level: api::hostlog::Level::Info,
            slot,
            always_stderr: false,
        },
        format_args!(
            "WalkTo outcome=aborted destination={} at={} reason={reason:?} leg=- transport=-",
            WalkTile(destination),
            WalkTile(at),
        ),
    );
}

fn terminal_at(outcome: &nav::traveller::TravelOutcome) -> WorldTile {
    use nav::traveller::TravelOutcome;
    match outcome {
        TravelOutcome::Arrived { at }
        | TravelOutcome::Stalled { at, .. }
        | TravelOutcome::Refused { at, .. }
        | TravelOutcome::Blocked { at, .. }
        | TravelOutcome::EvidenceUnproven { at, .. }
        | TravelOutcome::GaveUp { at, .. } => *at,
    }
}

struct WorldWalkTile(Option<WorldTile>);
impl fmt::Display for WorldWalkTile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(tile) => write!(f, "({},{},{})", tile.x, tile.z, tile.level),
            None => f.write_str("unknown"),
        }
    }
}

struct TerminalStatus<'a>(&'a nav::traveller::TravelOutcome);
impl fmt::Display for TerminalStatus<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.0, nav::traveller::TravelOutcome::Arrived { .. }) {
            f.write_str("arrived")
        } else {
            f.write_str("aborted")
        }
    }
}

struct TerminalReason<'a>(&'a nav::traveller::TravelOutcome);
impl fmt::Display for TerminalReason<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use nav::traveller::TravelOutcome;
        match self.0 {
            TravelOutcome::Arrived { .. } => f.write_str("-"),
            TravelOutcome::Stalled { why, .. } => write!(f, "Stalled({why:?})"),
            TravelOutcome::Refused { reason, .. } => write!(f, "{reason:?}"),
            TravelOutcome::Blocked { .. } => f.write_str("Blocked"),
            TravelOutcome::EvidenceUnproven { verdict, .. } => {
                write!(f, "EvidenceUnproven({verdict:?})")
            }
            TravelOutcome::GaveUp { .. } => f.write_str("GaveUp"),
        }
    }
}

struct LegNumber(Option<usize>);
impl fmt::Display for LegNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(index) => write!(f, "{}", index + 1),
            None => f.write_str("-"),
        }
    }
}

struct TerminalTransport<'a> {
    route: &'a Route,
    leg: Option<usize>,
}
impl fmt::Display for TerminalTransport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self
            .leg
            .and_then(|index| self.route.legs.get(index))
            .and_then(|leg| match leg {
                Leg::Transport { edge } => Some(edge),
                Leg::Walk { .. } => None,
            }) {
            Some(edge) => write!(f, "{:?}:{}", edge.kind, edge.loc_id),
            None => f.write_str("-"),
        }
    }
}

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
    /// The slot's bank memory rows ([`crate::Play::bank_rows`]).
    pub bank: &'a BankRows,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkSlotOutcomeKind {
    Walking,
    Excluded(WalkExclude),
    Failed(ActionError),
}

impl WalkSlotOutcomeKind {
    /// The short reason a slot did not start walking, in the wording every
    /// front end shows; `None` for a slot that is walking.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Walking => None,
            Self::Excluded(WalkExclude::NotLoggedIn) => Some("not logged in"),
            Self::Excluded(WalkExclude::NoPosition) => Some("no position yet"),
            Self::Excluded(WalkExclude::RunningScript) => Some("running a script"),
            Self::Failed(error) => Some(error.short()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkSlotOutcome {
    pub name: String,
    pub kind: WalkSlotOutcomeKind,
}

/// Per-bot Walk results, in request order. Front ends fold them into their
/// bulk report (`frontend_core::MarkedWalkReport`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupWalkReport {
    pub outcomes: Vec<WalkSlotOutcome>,
    /// The selected world is a legacy grid pack without a zone catalog.
    pub legacy_zones_unavailable: bool,
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
        let origin = origin.ok_or(ActionError::NoOrigin)?;
        if !(0..4).contains(&origin.level) {
            return Err(ActionError::InvalidCoordinates);
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
fn blocking_zones_for_walk(
    world: &NavWorld,
    origin: Tile,
    destination: Tile,
    mut options: FindOptions,
    slot: &WalkSlotRequest<'_>,
    arms: &WalkArms,
) -> Option<Vec<nav::zones::ZoneKey>> {
    options.essence = arms
        .lock()
        .unwrap()
        .get(slot.name)
        .and_then(|arm| arm.lock().unwrap().traveller.essence());
    let from = world_tile(origin);
    let destination = world_tile(destination);
    let direct = nav::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        destination,
        options,
        slot.state,
        &[],
    )
    .filter(|keys| !keys.is_empty());
    if direct.is_some() || !options.allow_bank_fetch {
        return direct;
    }
    let missing = nav::router::find_missing_item_reqs_with_avoid(
        &world.collision,
        &world.graph,
        from,
        destination,
        options,
        slot.state,
        &[],
    )?;
    let plan = plan_bank_fetch(
        &missing,
        slot.state,
        &slot.bank.planning_rows(&missing),
        world.banks(),
        from,
        &world.collision,
    )?;
    let access = plan.steps.iter().find_map(|step| match step {
        BankStep::Walk { x, z, level } => Some(WorldTile {
            x: *x,
            z: *z,
            level: *level,
        }),
        _ => None,
    })?;
    nav::router::find_blocking_zones(
        &world.collision,
        &world.graph,
        from,
        access,
        options,
        slot.state,
        &[],
    )
    .filter(|keys| !keys.is_empty())
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
        bank: &BankRows,
        arms: &WalkArms,
    ) -> Result<Route, ActionError> {
        self.check(current, ActionKind::Walk)?;
        if !safe_standable(world, world_tile(self.origin)) {
            return Err(ActionError::OriginNotStandable);
        }
        if !safe_standable(world, world_tile(self.destination)) {
            return Err(ActionError::Blocked);
        }
        let result = crate::arm_walk_on(
            world,
            self.origin,
            self.destination,
            self.options,
            state,
            bank,
            arms,
            Some(name),
        );
        if result.is_err() {
            // Diagnose a legal item-gated route before an all-exempt zone
            // detour: a cheaper unsafe walk must not hide an unpaid fare.
            if let Some(missing) = nav::router::find_missing_item_reqs(
                &world.collision,
                &world.graph,
                world_tile(self.origin),
                world_tile(self.destination),
                self.options,
                state,
            ) {
                let shortfalls = route_supply_shortfall_detail(
                    world,
                    state,
                    missing.into_iter().filter_map(|req| match req {
                        nav::router::MissingReq::Carry { id, count } => Some((id, count)),
                        nav::router::MissingReq::WearAny { .. } => None,
                    }),
                );
                if let Some(detail) = shortfalls {
                    return Err(ActionError::InsufficientItems { detail });
                }
            }
            if let Some(keys) = blocking_zones_for_walk(
                world,
                self.origin,
                self.destination,
                self.options,
                &WalkSlotRequest { name, state, bank },
                arms,
            ) {
                let detail = world
                    .graph
                    .zones
                    .as_ref()
                    .map(|table| crate::blocked_zone_detail(table, &keys));
                if let Some(detail) = detail.as_deref() {
                    crate::walk_map::emit_walk_aborted(
                        Some(name),
                        world_tile(self.destination),
                        Some(world_tile(self.origin)),
                        detail,
                        false,
                    );
                }
                return Err(ActionError::BlockedByZones { detail });
            }
        }
        match result {
            Ok(route) => Ok(route),
            Err(_) if !state.map_members => {
                let members_state = state.clone().with_map_members(true);
                if nav::router::find_with(
                    &world.collision,
                    &world.graph,
                    world_tile(self.origin),
                    world_tile(self.destination),
                    self.options,
                    &members_state,
                )
                .is_ok()
                {
                    Err(ActionError::MembersOnly)
                } else {
                    Err(ActionError::NoPath)
                }
            }
            Err(_) => Err(ActionError::NoPath),
        }
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
        bank: &BankRows,
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
        let legacy_zones_unavailable = self
            .world
            .as_deref()
            .is_some_and(|world| world.graph.zones.is_none());
        let profile = self.server_profile();
        for req in slots {
            let status = self.walk_eligibility(req.name);
            let origin = crate::play_status::lock_statuses(&self.statuses)
                .iter()
                .find(|s| s.username == req.name)
                .and_then(|s| s.ready_tile())
                .map(|(x, z, level)| Tile { x, z, level });
            let result = WalkRequest {
                slot: Some(req.name),
                origin,
                destination: Some(plan.destination),
                options: plan.options,
                map_members: req.state.map_members,
                members: profile
                    .as_ref()
                    .map_or(&crate::WorldMembersFact::Unknown, |p| p.world_members()),
            }
            .run(|| {
                let ready = match status {
                    WalkSlotStatus::Eligible(ready) => ready,
                    WalkSlotStatus::Excluded(reason) => {
                        return Err(match reason {
                            WalkExclude::NotLoggedIn => ActionError::NoFocus,
                            WalkExclude::NoPosition => ActionError::NoOrigin,
                            WalkExclude::RunningScript => ActionError::RunningScript,
                        })
                    }
                };
                let command = plan.command(req.name, ready.origin);
                let current = MapContext {
                    focus: self.map_focus(req.name),
                    nav: dest.nav,
                    overlay: dest.overlay,
                    generation: dest.generation,
                };
                self.map_walk(command, &current, req.state, req.bank, arms)
            });
            let kind = match (status, result) {
                (WalkSlotStatus::Excluded(reason), _) => WalkSlotOutcomeKind::Excluded(reason),
                (_, Ok(_)) => WalkSlotOutcomeKind::Walking,
                (_, Err(error)) => WalkSlotOutcomeKind::Failed(error),
            };
            outcomes.push(WalkSlotOutcome {
                name: req.name.to_string(),
                kind,
            });
        }
        GroupWalkReport {
            outcomes,
            legacy_zones_unavailable,
        }
    }

    /// Same Local+loopback rule as [`crate::walk_map::debug_teleport_authorized`].
    pub fn map_teleport_authorized(&self) -> bool {
        crate::walk_map::debug_teleport_authorized(
            self.connection.profile_class(),
            self.connection.game_host(),
        )
    }
    pub fn map_teleport(
        &self,
        command: MapCommand,
        current: &MapContext,
    ) -> Result<(), ActionError> {
        let name = self.validate_map_command(&command, current, ActionKind::Teleport)?;
        debug_authorized(self.connection.profile_class(), self.connection.game_host())?;
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
pub(super) fn debug_authorized(class: crate::ProfileClass, host: &str) -> Result<(), ActionError> {
    if crate::walk_map::debug_teleport_authorized(class, host) {
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
