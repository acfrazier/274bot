//! Permission composition and durable danger levels at host admissions.
use std::cell::OnceCell;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use nav::router::FindOptions;
use nav::zones::ZoneExempt;
use script::native::{RiskPolicy, WalkBit, WalkOptions, WalkPermissions};

/// Runtime net activation is held until the integrated S2c gate passes.
pub const NET_AVAILABLE: bool = false;

/// Stored global danger-routing level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerLevel {
    Never,
    WhenSurvivable,
    Always,
}

impl DangerLevel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::WhenSurvivable => "When survivable",
            Self::Always => "Always",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Never => Self::WhenSurvivable,
            Self::WhenSurvivable => Self::Always,
            Self::Always => Self::Never,
        }
    }

    fn risk_policy(self, script: WalkPermissions, walk: WalkOptions) -> RiskPolicy {
        if walk.allow_danger_zones == WalkBit::Forbid {
            return RiskPolicy::Avoid;
        }

        if walk
            .allow_danger_zones
            .resolve(self == Self::Always, script.allow_danger_zones)
        {
            return RiskPolicy::Proceed;
        }

        match self {
            Self::Never | Self::Always => RiskPolicy::Avoid,
            Self::WhenSurvivable => RiskPolicy::Inherit,
        }
    }
}

/// Durable global grants published by either frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WalkGlobals {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    pub allow_danger_zones: bool,
    pub survivable_routing: bool,
}

impl Default for WalkGlobals {
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: true,
        }
    }
}

impl WalkGlobals {
    /// Return a deny-all value for an unreadable or malformed preference file.
    pub const fn fail_closed() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: false,
        }
    }

    /// Read all globals in one pass from the shared preferences, ignoring
    /// unrelated ones. An absent file or `nav` member is a first-run
    /// default; a refused file or a malformed `nav` is an error, so callers
    /// can use [`Self::fail_closed`].
    pub fn read(preferences: &super::local_state::Preferences) -> io::Result<Self> {
        match preferences.document()?.get("nav") {
            None => Ok(Self::default()),
            Some(nav) => serde_json::from_value(nav.clone())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }

    /// Project the stored pair of booleans to the three-level setting.
    pub const fn danger_level(self) -> DangerLevel {
        if self.allow_danger_zones {
            DangerLevel::Always
        } else if self.survivable_routing {
            DangerLevel::WhenSurvivable
        } else {
            DangerLevel::Never
        }
    }

    /// Gate the middle level until the runtime net is available.
    pub const fn effective_danger_level(self) -> DangerLevel {
        match self.danger_level() {
            DangerLevel::WhenSurvivable if !NET_AVAILABLE => DangerLevel::Never,
            level => level,
        }
    }

    /// Set the canonical boolean representation of a danger-routing level.
    pub fn set_danger_level(&mut self, level: DangerLevel) {
        (self.allow_danger_zones, self.survivable_routing) = match level {
            DangerLevel::Never => (false, false),
            DangerLevel::WhenSurvivable => (false, true),
            DangerLevel::Always => (true, false),
        };
    }

    /// Resolve script and walk overrides without modifying scoped named grants.
    pub fn risk_policy(self, script: WalkPermissions, walk: WalkOptions) -> RiskPolicy {
        self.effective_danger_level().risk_policy(script, walk)
    }

    /// Resolve verdict authority separately from router grants. While held,
    /// inherited requests retain precisely the router's pre-S2b outcome;
    /// explicit script and per-walk overrides still use the admission seam.
    pub fn enforces_risk(self, script: WalkPermissions, walk: WalkOptions) -> bool {
        NET_AVAILABLE || script.allow_danger_zones || walk.allow_danger_zones != WalkBit::Inherit
    }

    /// Manual WalkTo has no script grant. Its danger control is one admission only.
    pub fn manual_options(self, danger_this_walk: bool) -> FindOptions {
        FindOptions {
            allow_teleports: self.allow_teleports,
            allow_wilderness: self.allow_wilderness,
            allow_bank_fetch: self.allow_bank_fetch,
            zones: if self.allow_danger_zones || danger_this_walk {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            },
            ..FindOptions::default()
        }
    }

    fn native_options(self, script: WalkPermissions, walk: WalkOptions) -> FindOptions {
        let policy = self.risk_policy(script, walk);
        self.native_options_with_policy(script, walk, policy)
    }

    fn native_options_with_policy(
        self,
        script: WalkPermissions,
        walk: WalkOptions,
        policy: RiskPolicy,
    ) -> FindOptions {
        FindOptions {
            allow_teleports: walk
                .allow_teleports
                .resolve(self.allow_teleports, script.allow_teleports),
            allow_wilderness: walk
                .allow_wilderness
                .resolve(self.allow_wilderness, script.allow_wilderness),
            // Native supply preparation belongs to the card's Provisioner, not BankBudget.
            allow_bank_fetch: false,
            zones: if policy == RiskPolicy::Proceed {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            },
            ..FindOptions::default()
        }
    }
}

pub(crate) fn globals_for(bot: Option<&super::NavBot>) -> WalkGlobals {
    match bot.and_then(|bot| bot.walk_globals_store.as_ref()) {
        Some(preferences) => {
            WalkGlobals::read(preferences).unwrap_or_else(|_| WalkGlobals::fail_closed())
        }
        None => bot
            .and_then(|bot| bot.walk_globals.as_ref())
            .map_or_else(WalkGlobals::default, |globals| *globals.lock().unwrap()),
    }
}

/// Record the held-off middle level once per session, never from a tick path.
pub(crate) fn log_runtime_net_gate(slot: &str, globals: WalkGlobals, logged: &mut bool) {
    if globals.danger_level() == DangerLevel::WhenSurvivable
        && globals.effective_danger_level() == DangerLevel::Never
        && !*logged
    {
        *logged = true;
        api::host_log!(
            api::hostlog::Category::NavEvent,
            api::hostlog::Level::Info,
            slot = slot,
            "When survivable is not available yet: acts as Never; inherited routing is unchanged"
        );
    }
}

/// Resolve native search options and risk policy from one durable settings read.
/// Callback-scoped permissions override the per-bot mirror; when absent, the
/// mirror remains the fallback. Load API Gather values come from the slot lock.
pub(crate) fn native_admission(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    walk: WalkOptions,
    session_permissions: Option<script::native::WalkPermissions>,
) -> (FindOptions, RiskPolicy, bool) {
    let mut all = navs.lock().unwrap();
    let mut bot = all.get_mut(name);
    let globals = globals_for(bot.as_deref());
    if let Some(bot) = bot.as_deref_mut() {
        log_runtime_net_gate(name, globals, &mut bot.runtime_gate_logged);
    }
    // The slot-locked value describes this callback. The bot copy is updated
    // afterward, so it may belong to the preceding card revision.
    let script = session_permissions
        .or_else(|| bot.as_deref().and_then(|bot| bot.native_permissions))
        .unwrap_or_default();
    resolve_native_admission(globals, script, walk)
}

fn resolve_native_admission(
    globals: WalkGlobals,
    script: WalkPermissions,
    walk: WalkOptions,
) -> (FindOptions, RiskPolicy, bool) {
    let policy = globals.risk_policy(script, walk);
    (
        globals.native_options_with_policy(script, walk, policy),
        policy,
        globals.enforces_risk(script, walk),
    )
}

/// One callback's borrowed danger authority. Construction only borrows inputs.
/// The first selection query resolves the same permissions as a native walk.
pub(crate) struct SelectionDangerQuery<'a> {
    navs: &'a Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &'a str,
    world: &'a nav::world::NavWorld,
    snapshot: Option<&'a api::snapshot::GameSnapshot>,
    session_permissions: Option<WalkPermissions>,
    resolved: OnceCell<SelectionDanger>,
}

enum SelectionDanger {
    Allowed,
    Filtered(nav::zones::ZoneFilterOwned),
    Unavailable,
}

impl<'a> SelectionDangerQuery<'a> {
    pub(crate) fn new(
        navs: &'a Arc<Mutex<HashMap<String, super::NavBot>>>,
        name: &'a str,
        world: &'a nav::world::NavWorld,
        snapshot: Option<&'a api::snapshot::GameSnapshot>,
        session_permissions: Option<WalkPermissions>,
    ) -> Self {
        Self {
            navs,
            name,
            world,
            snapshot,
            session_permissions,
            resolved: OnceCell::new(),
        }
    }

    fn resolved(&self) -> &SelectionDanger {
        self.resolved.get_or_init(|| {
            let (options, _, _) = native_admission(
                self.navs,
                self.name,
                WalkOptions::default(),
                self.session_permissions,
            );
            if options.zones.is_all() {
                return SelectionDanger::Allowed;
            }
            let Some(table) = self.world.graph.zones.as_ref() else {
                return SelectionDanger::Unavailable;
            };
            let combat = self
                .snapshot
                .and_then(|snapshot| snapshot.local_player())
                .map(|player| player.player.combat_level);
            match nav::zones::ZoneFilterOwned::new(table.zones().len(), combat, &[], options.zones)
            {
                Ok(filter) => SelectionDanger::Filtered(filter),
                Err(_) => SelectionDanger::Unavailable,
            }
        })
    }
}

impl script::native::WalkDangerQuery for SelectionDangerQuery<'_> {
    fn check(
        &self,
        target: api::WorldTile,
        loc_id: Option<i32>,
        radius: u16,
        arrival: nav::contract::ArrivalKind,
    ) -> script::native::WalkDanger<'_> {
        use script::native::WalkDanger;

        let filter = match self.resolved() {
            SelectionDanger::Allowed => return WalkDanger::Allowed,
            SelectionDanger::Unavailable => return WalkDanger::Unavailable,
            SelectionDanger::Filtered(filter) => filter,
        };
        let Some(table) = self.world.graph.zones.as_ref() else {
            return WalkDanger::Unavailable;
        };
        let Some(filter) = filter.view(table) else {
            return WalkDanger::Unavailable;
        };
        let mut denied = None;
        for tile in
            selection_access_tiles(self.world, self.snapshot, target, loc_id, radius, arrival)
        {
            match filter
                .blocking_at(&self.world.graph.wilderness, tile)
                .next()
            {
                None => return WalkDanger::Allowed,
                Some(index) => denied = denied.or(Some(index)),
            }
        }
        match denied {
            Some(index) => {
                let zone = &table.zones()[usize::from(index)];
                let label = match table.key(index) {
                    nav::zones::ZoneKey::Group(group) => &table.groups()[usize::from(group)].label,
                    nav::zones::ZoneKey::Zone(_) => &table.kinds()[usize::from(zone.kind)].label,
                };
                WalkDanger::Denied { zone: label }
            }
            // No legal access tile is a geometry failure, not evidence of danger.
            None => WalkDanger::Allowed,
        }
    }

    fn safe_access_goals(
        &self,
        target: api::WorldTile,
        loc_id: Option<i32>,
        radius: u16,
        arrival: nav::contract::ArrivalKind,
        existing: Option<&script::native::WalkGoals>,
    ) -> Result<Option<script::native::WalkGoals>, script::native::WalkGoalsError> {
        use script::native::{WalkGoals, WalkGoalsError};

        let filter = match self.resolved() {
            SelectionDanger::Allowed => return Ok(None),
            SelectionDanger::Unavailable => return Err(WalkGoalsError::Empty),
            SelectionDanger::Filtered(filter) => filter,
        };
        let Some(table) = self.world.graph.zones.as_ref() else {
            return Err(WalkGoalsError::Empty);
        };
        let Some(filter) = filter.view(table) else {
            return Err(WalkGoalsError::Empty);
        };
        let is_denied = |tile| {
            filter
                .blocking_at(&self.world.graph.wilderness, tile)
                .next()
                .is_some()
        };

        let mut safe_count = 0usize;
        let mut denied = false;
        if let Some(existing) = existing {
            for &tile in existing.as_slice() {
                if is_denied(tile) {
                    denied = true;
                } else {
                    safe_count += 1;
                }
            }
        } else {
            for tile in
                selection_access_tiles(self.world, self.snapshot, target, loc_id, radius, arrival)
            {
                if is_denied(tile) {
                    denied = true;
                } else {
                    safe_count += 1;
                }
            }
        }

        if !denied {
            return Ok(None);
        }
        if safe_count == 0 {
            return Err(WalkGoalsError::Empty);
        }
        if safe_count > WalkGoals::MAX {
            return Err(WalkGoalsError::TooMany {
                count: safe_count,
                limit: WalkGoals::MAX,
            });
        }

        let mut safe_goals = Vec::with_capacity(safe_count);
        if let Some(existing) = existing {
            for &tile in existing.as_slice() {
                if !is_denied(tile) {
                    safe_goals.push(tile);
                }
            }
            let mut safe_preferred = Vec::with_capacity(existing.preferred().len());
            for &tile in existing.preferred() {
                if !is_denied(tile) {
                    safe_preferred.push(tile);
                }
            }
            Ok(Some(WalkGoals::with_preferred(
                safe_goals,
                &safe_preferred,
            )?))
        } else {
            for tile in
                selection_access_tiles(self.world, self.snapshot, target, loc_id, radius, arrival)
            {
                if !is_denied(tile) {
                    safe_goals.push(tile);
                }
            }
            Ok(Some(WalkGoals::new(safe_goals)?))
        }
    }
}

/// Scalar radius geometry shared by the route cursor and selection's lazy
/// access scan. Candidate storage and collision filtering stay with each caller.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RouteRadiusGeometry {
    target: api::WorldTile,
    radius: i32,
    side: usize,
    len: usize,
}

impl RouteRadiusGeometry {
    pub(crate) fn new(target: api::WorldTile, radius: i32) -> Option<Self> {
        let radius = radius.clamp(0, 104);
        let side = usize::try_from(radius)
            .ok()?
            .checked_mul(2)?
            .checked_add(1)?;
        Some(Self {
            target,
            radius,
            side,
            len: side.checked_mul(side)?,
        })
    }

    pub(crate) const fn radius(self) -> i32 {
        self.radius
    }

    pub(crate) const fn len(self) -> usize {
        self.len
    }

    pub(crate) fn candidate_at(self, index: usize) -> Option<api::WorldTile> {
        if index >= self.len {
            return None;
        }
        let dx = i32::try_from(index / self.side)
            .ok()?
            .checked_sub(self.radius)?;
        let dz = i32::try_from(index % self.side)
            .ok()?
            .checked_sub(self.radius)?;
        Some(api::WorldTile {
            x: self.target.x.checked_add(dx)?,
            z: self.target.z.checked_add(dz)?,
            level: self.target.level,
        })
    }
}

/// The scalar footprint predicate used by captured route goals and the
/// allocation-free selection scan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocAccessGeometry {
    target: api::WorldTile,
    approach: Option<api::query::loc_approach::LocApproach>,
    radius: u32,
}

impl LocAccessGeometry {
    fn from_loc(
        loc: &api::snapshot::LocView,
        target: api::WorldTile,
        radius: i32,
    ) -> Option<Self> {
        use api::query::loc_approach::{distance_from, LocApproach};

        distance_from(loc, target)?;
        Some(Self {
            target,
            approach: LocApproach::from_loc_def(
                loc.width,
                loc.length,
                loc.angle,
                loc.force_approach,
            ),
            radius: u32::try_from(radius.clamp(0, 104)).ok()?,
        })
    }

    pub(crate) fn candidate_at(
        self,
        tile: api::WorldTile,
        flags: i32,
    ) -> Option<api::WorldTile> {
        let approach = self.approach?;
        if flags & client::dash3d::CollisionFlag::SQ_BLOCKED != 0
            || !approach.can_operate(self.target, tile, flags)
        {
            return None;
        }
        let max_x = self
            .target
            .x
            .checked_add(i32::from(approach.width).checked_sub(1)?)?;
        let max_z = self
            .target
            .z
            .checked_add(i32::from(approach.length).checked_sub(1)?)?;
        let nearest_x = tile.x.clamp(self.target.x, max_x);
        let nearest_z = tile.z.clamp(self.target.z, max_z);
        (tile
            .x
            .abs_diff(nearest_x)
            .max(tile.z.abs_diff(nearest_z))
            <= self.radius)
            .then_some(tile)
    }
}

/// A scene-clipped scalar enumeration window for the route's loc footprint.
/// It stores no collision flags; route capture and selection read them through
/// their own existing storage/borrowed-scene paths.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocAccessWindow {
    base_x: i32,
    base_z: i32,
    level: i32,
    start_x: i32,
    start_z: i32,
    height: usize,
    len: usize,
}

impl LocAccessWindow {
    fn new(
        scene: &api::snapshot::SceneView,
        target: api::WorldTile,
        approach: Option<api::query::loc_approach::LocApproach>,
    ) -> Option<Self> {
        let (start_x, start_z, width, height) = if let Some(approach) = approach {
            let dst_x = target.x.checked_sub(scene.base_x)?;
            let dst_z = target.z.checked_sub(scene.base_z)?;
            let width = i32::from(approach.width);
            let height = i32::from(approach.length);
            let start_x = dst_x.checked_sub(1)?.max(0);
            let start_z = dst_z.checked_sub(1)?.max(0);
            let end_x = dst_x.checked_add(width)?.min(scene.width.checked_sub(1)?);
            let end_z = dst_z.checked_add(height)?.min(scene.height.checked_sub(1)?);
            if start_x > end_x || start_z > end_z {
                (start_x, start_z, 0, 0)
            } else {
                (
                    start_x,
                    start_z,
                    usize::try_from(end_x.checked_sub(start_x)?.checked_add(1)?).ok()?,
                    usize::try_from(end_z.checked_sub(start_z)?.checked_add(1)?).ok()?,
                )
            }
        } else {
            (0, 0, 0, 0)
        };
        Some(Self {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            start_x,
            start_z,
            height,
            len: width.checked_mul(height)?,
        })
    }

    pub(crate) const fn len(self) -> usize {
        self.len
    }

    fn local_at(self, index: usize) -> Option<(i32, i32)> {
        if index >= self.len || self.height == 0 {
            return None;
        }
        Some((
            self.start_x
                .checked_add(i32::try_from(index / self.height).ok()?)?,
            self.start_z
                .checked_add(i32::try_from(index % self.height).ok()?)?,
        ))
    }

    pub(crate) fn tile_at(self, index: usize) -> Option<api::WorldTile> {
        let (lx, lz) = self.local_at(index)?;
        Some(api::WorldTile {
            x: self.base_x.checked_add(lx)?,
            z: self.base_z.checked_add(lz)?,
            level: self.level,
        })
    }

    pub(crate) fn collision_index(self, index: usize, scene_height: i32) -> Option<usize> {
        let (lx, lz) = self.local_at(index)?;
        usize::try_from(lx)
            .ok()?
            .checked_mul(usize::try_from(scene_height).ok()?)?
            .checked_add(usize::try_from(lz).ok()?)
    }
}

/// Resolve the exact live-loc geometry inputs used by the router, without
/// capturing flags or allocating a candidate vector.
pub(crate) fn route_loc_access_geometry<'a>(
    snapshot: &'a api::snapshot::GameSnapshot,
    target: api::WorldTile,
    loc_id: i32,
    radius: i32,
) -> Option<(
    LocAccessGeometry,
    LocAccessWindow,
    &'a api::snapshot::SceneView,
)> {
    let scene = snapshot.scene();
    if !api::query::SceneQuery::new(scene, None).contains(target) {
        return None;
    }
    let loc = snapshot
        .locs()
        .iter()
        .find(|loc| loc.tile == target && loc.id == loc_id)?;
    let geometry = LocAccessGeometry::from_loc(loc, target, radius)?;
    let window = LocAccessWindow::new(scene, target, geometry.approach)?;
    Some((geometry, window, scene))
}

/// Return the packed collision flags used by both route arrival admission and
/// the selector's scalar Reach predicate.
pub(crate) fn packed_arrival_flags(
    collision: &nav::collision::WorldCollision,
    tile: api::WorldTile,
) -> Option<i32> {
    if !(0..=3).contains(&tile.level) {
        return None;
    }
    let x = i64::from(tile.x) - i64::from(collision.origin.x);
    let z = i64::from(tile.z) - i64::from(collision.origin.z);
    if x < 0
        || z < 0
        || usize::try_from(x).ok()? >= collision.width
        || usize::try_from(z).ok()? >= collision.height
    {
        return None;
    }
    i32::try_from(collision.walkable_word(tile.x, tile.z, tile.level)).ok()
}

/// The same scalar destination geometry and packed arrival rule used by the
/// route cursor, without capturing flags or constructing candidate vectors.
pub(crate) fn selection_access_tiles<'a>(
    world: &'a nav::world::NavWorld,
    snapshot: Option<&'a api::snapshot::GameSnapshot>,
    target: api::WorldTile,
    loc_id: Option<i32>,
    radius: u16,
    arrival: nav::contract::ArrivalKind,
) -> impl Iterator<Item = api::WorldTile> + 'a {
    use api::query::{is_arrived, CollisionArrivalProbe};

    let radius_geometry = RouteRadiusGeometry::new(target, i32::from(radius))
        .expect("a clamped access radius fits the route window");
    let exact = radius == 0;
    let live_geometry = if exact {
        None
    } else {
        loc_id.and_then(|loc_id| {
            snapshot.and_then(|snapshot| {
                route_loc_access_geometry(snapshot, target, loc_id, radius_geometry.radius())
            })
        })
    };
    let candidate_count = if exact {
        1
    } else {
        live_geometry.map_or(radius_geometry.len(), |(_, window, _)| window.len())
    };
    let collision = &world.collision;
    let probe = CollisionArrivalProbe::new(move |tile: api::WorldTile| {
        packed_arrival_flags(collision, tile)
    });
    (0..candidate_count).filter_map(move |index| {
        if exact {
            return Some(target);
        }
        if let Some((geometry, window, scene)) = live_geometry {
            let tile = window.tile_at(index)?;
            return geometry.candidate_at(tile, scene.collision_at(tile)?);
        }
        let tile = radius_geometry.candidate_at(index)?;
        if !world.collision.standable(tile) {
            return None;
        }
        // Explicit loc routes without a modeled in-scene footprint use the
        // router's standable-radius estimate, not plain-tile Reach filtering.
        (loc_id.is_some()
            || arrival == nav::contract::ArrivalKind::Area
            || is_arrived(tile, target, i32::from(radius), || &probe))
        .then_some(tile)
    })
}

/// Compiled callers using the existing bool wire (Sherlock's clue machine and
/// watchdog walks) treat false as Inherit. Isolate v1/v2 remain unchanged.
pub(crate) fn compiled_options(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    opts: FindOptions,
) -> FindOptions {
    let mut all = navs.lock().unwrap();
    let Some(bot) = all.get_mut(name) else {
        return opts;
    };
    if bot.compat_v1 {
        return opts;
    }
    let globals = globals_for(Some(bot));
    log_runtime_net_gate(name, globals, &mut bot.runtime_gate_logged);
    let Some(script) = bot.native_permissions else {
        // Legacy bool calls keep teleport/wilderness/bank authority unchanged.
        // Only the host's danger preference is inherited.
        let zones =
            if globals.risk_policy(Default::default(), Default::default()) == RiskPolicy::Proceed {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            };
        return FindOptions {
            zones: opts
                .zones
                .union(zones)
                .expect("all or empty fits named capacity"),
            ..opts
        };
    };
    let bit = |allow| {
        if allow {
            WalkBit::Allow
        } else {
            WalkBit::Inherit
        }
    };
    let resolved = globals.native_options(
        script,
        WalkOptions {
            allow_teleports: bit(opts.allow_teleports),
            allow_wilderness: bit(opts.allow_wilderness),
            allow_danger_zones: WalkBit::Inherit,
        },
    );
    FindOptions {
        zones: opts
            .zones
            .union(resolved.zones)
            .expect("all or empty does not exceed named capacity"),
        allow_teleports: resolved.allow_teleports,
        allow_wilderness: resolved.allow_wilderness,
        allow_bank_fetch: false,
        ..opts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_world() -> nav::world::NavWorld {
        let origin = api::WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let (walk, blocked) = nav::collision::pack_walk(&[0; 4]);
        nav::world::NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin,
                width: 1,
                height: 1,
                walk,
                blocked,
                flags: None,
            },
            nav::transport::TransportGraph::default(),
            Vec::new(),
        )
    }

    #[test]
    fn selection_query_construction_does_not_acquire_nav_mutex() {
        use std::sync::mpsc;
        use std::time::Duration;

        let world = empty_world();
        let navs = Arc::new(Mutex::new(HashMap::<String, super::super::NavBot>::new()));
        let guard = navs.lock().unwrap();
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (constructed_tx, constructed_rx) = mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                started_tx.send(()).unwrap();
                let _query = SelectionDangerQuery::new(&navs, "alice", &world, None, None);
                constructed_tx.send(()).unwrap();
            });
            started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("query constructor worker started");
            let constructed = constructed_rx.recv_timeout(Duration::from_secs(2));
            drop(guard);
            assert!(
                constructed.is_ok(),
                "construction must not inspect nav state before selection"
            );
        });
    }

    #[test]
    fn native_admission_prefers_callback_permissions_and_keeps_mirror_fallback() {
        let previous = WalkPermissions {
            allow_danger_zones: true,
            ..Default::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".into(),
            super::super::NavBot {
                native_permissions: Some(previous),
                ..Default::default()
            },
        )])));
        let (current_options, current_policy, _) = native_admission(
            &navs,
            "alice",
            WalkOptions::default(),
            Some(WalkPermissions::default()),
        );
        assert_eq!(current_policy, RiskPolicy::Avoid);
        assert!(!current_options.zones.is_all());

        let (fallback_options, fallback_policy, _) =
            native_admission(&navs, "alice", WalkOptions::default(), None);
        assert_eq!(fallback_policy, RiskPolicy::Proceed);
        assert!(fallback_options.zones.is_all());
    }

    #[test]
    fn native_permission_truth_table_and_manual_fetch_isolation() {
        for (global, script, inherited) in [
            (false, false, false),
            (false, true, true),
            (true, false, true),
            (true, true, true),
        ] {
            let globals = WalkGlobals {
                allow_teleports: global,
                allow_wilderness: global,
                allow_bank_fetch: true,
                allow_danger_zones: global,
                survivable_routing: true,
            };
            let script = WalkPermissions {
                allow_teleports: script,
                allow_wilderness: script,
                allow_danger_zones: script,
            };
            for (bit, expected) in [
                (WalkBit::Inherit, inherited),
                (WalkBit::Allow, true),
                (WalkBit::Forbid, false),
            ] {
                let opts = globals.native_options(
                    script,
                    WalkOptions {
                        allow_teleports: bit,
                        allow_wilderness: bit,
                        allow_danger_zones: bit,
                    },
                );
                assert_eq!(opts.allow_teleports, expected);
                assert_eq!(opts.allow_wilderness, expected);
                assert_eq!(opts.zones.is_all(), expected);
                assert!(!opts.allow_bank_fetch);
            }
            assert!(globals.manual_options(false).allow_bank_fetch);
            assert_eq!(globals.manual_options(false).zones.is_all(), global);
            assert!(globals.manual_options(true).zones.is_all());
            assert_eq!(globals.manual_options(false).zones.is_all(), global);
        }
    }

    /// The scratch-rooted process service's shared preferences.
    fn preferences(scratch: &script::IsolatedEnv) -> super::super::local_state::Preferences {
        super::super::local_state::LocalStateService::open_process(
            super::super::local_state::StateRoot::new(scratch.home.join(".274bot")),
        )
        .preferences()
    }
    /// Retry the globals write while it stays queued past the frontend's
    /// bounded durability wait; under machine load it settles later. The
    /// update re-reads and re-applies its merge on a conflict, so a retry
    /// after a late-landing write converges instead of duplicating it.
    fn set_nav_patiently(
        preferences: &super::super::local_state::Preferences,
        value: serde_json::Value,
    ) {
        let start = std::time::Instant::now();
        loop {
            match preferences.set("nav", value.clone()) {
                Ok(()) => return,
                Err(error)
                    if error.to_string().contains("not durable within")
                        && start.elapsed() < std::time::Duration::from_secs(60) =>
                {
                    std::thread::yield_now();
                }
                Err(error) => panic!("globals write failed: {error}"),
            }
        }
    }

    #[test]
    fn durable_globals_are_reread_at_native_and_manual_admissions() {
        let scratch = script::IsolatedEnv::enter("walk-globals-reread");
        let preferences = preferences(&scratch);
        let bot = super::super::NavBot {
            walk_globals_store: Some(preferences.clone()),
            native_permissions: Some(WalkPermissions::default()),
            ..Default::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([("alice".into(), bot)])));
        for enabled in [true, false, true] {
            set_nav_patiently(
                &preferences,
                serde_json::json!({
                    "allow_teleports": enabled,
                    "allow_wilderness": enabled,
                    "allow_bank_fetch": enabled,
                    "allow_danger_zones": enabled,
                }),
            );
            let (native, policy, _) =
                native_admission(&navs, "alice", WalkOptions::default(), None);
            assert_eq!(
                policy,
                if enabled {
                    RiskPolicy::Proceed
                } else {
                    RiskPolicy::Avoid
                }
            );
            let compiled = compiled_options(&navs, "alice", FindOptions::default());
            let manual = WalkGlobals::read(&preferences)
                .unwrap()
                .manual_options(false);
            for options in [native, compiled, manual] {
                assert_eq!(options.allow_teleports, enabled);
                assert_eq!(options.allow_wilderness, enabled);
                assert_eq!(options.zones.is_all(), enabled);
            }
            assert!(!native.allow_bank_fetch && !compiled.allow_bank_fetch);
            assert_eq!(manual.allow_bank_fetch, enabled);
        }
    }

    #[test]
    fn isolate_booleans_do_not_inherit_native_globals() {
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "isolate".into(),
            super::super::NavBot {
                walk_globals: Some(Arc::new(Mutex::new(WalkGlobals {
                    allow_teleports: true,
                    allow_wilderness: true,
                    allow_bank_fetch: true,
                    allow_danger_zones: true,
                    survivable_routing: true,
                }))),
                ..Default::default()
            },
        )])));
        for allow_bank_fetch in [false, true] {
            let explicit = FindOptions {
                allow_bank_fetch,
                ..Default::default()
            };
            let compiled = compiled_options(&navs, "isolate", explicit);
            assert_eq!(compiled.allow_teleports, explicit.allow_teleports);
            assert_eq!(compiled.allow_wilderness, explicit.allow_wilderness);
            assert_eq!(compiled.allow_bank_fetch, explicit.allow_bank_fetch);
            assert!(
                compiled.zones.is_all(),
                "S2b applies shared risk settings independently of frozen v1 H booleans"
            );
        }
    }
    #[test]
    fn walk_globals_round_trip_default_absent_old_file_and_fail_closed_error() {
        let scratch = script::IsolatedEnv::enter("walk-globals-round-trip");
        let preferences = preferences(&scratch);
        let path = preferences.path();

        let defaults = WalkGlobals::default();
        assert!(!defaults.allow_teleports);
        assert!(!defaults.allow_wilderness);
        assert!(!defaults.allow_bank_fetch);
        assert!(!defaults.allow_danger_zones);
        assert!(defaults.survivable_routing);
        assert_eq!(defaults.danger_level(), DangerLevel::WhenSurvivable);
        assert_eq!(defaults.effective_danger_level(), DangerLevel::Never);
        assert_eq!(
            serde_json::from_value::<WalkGlobals>(serde_json::to_value(defaults).unwrap()).unwrap(),
            defaults
        );
        assert_eq!(WalkGlobals::read(&preferences).unwrap(), defaults);

        // A file an older build (or the other frontend's process) wrote.
        vault::write_private_file(&path, br#"{"nav":{"allow_danger_zones":false}}"#).unwrap();
        preferences.reload().unwrap();
        let migrated = WalkGlobals::read(&preferences).unwrap();
        assert_eq!(migrated.danger_level(), DangerLevel::WhenSurvivable);
        assert!(migrated.survivable_routing);
        assert_eq!(migrated.effective_danger_level(), DangerLevel::Never);

        vault::write_private_file(&path, b"{malformed").unwrap();
        assert!(preferences.reload().is_err());
        assert!(WalkGlobals::read(&preferences).is_err());
        assert_eq!(
            WalkGlobals::fail_closed(),
            WalkGlobals {
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                allow_danger_zones: false,
                survivable_routing: false,
            }
        );
    }

    #[test]
    fn danger_policy_exhaustively_resolves_levels_script_bits_and_walk_bits() {
        for stored_level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            let mut globals = WalkGlobals::fail_closed();
            globals.set_danger_level(stored_level);
            assert_eq!(globals.danger_level(), stored_level);
            let effective_level = if stored_level == DangerLevel::WhenSurvivable && !NET_AVAILABLE {
                DangerLevel::Never
            } else {
                stored_level
            };
            assert_eq!(
                globals.effective_danger_level(),
                effective_level,
                "gate for {stored_level:?}"
            );
            for script_grant in [false, true] {
                let script = WalkPermissions {
                    allow_danger_zones: script_grant,
                    ..WalkPermissions::default()
                };
                for walk_bit in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
                    let walk = WalkOptions {
                        allow_danger_zones: walk_bit,
                        ..WalkOptions::default()
                    };
                    let expected = if walk_bit == WalkBit::Forbid {
                        RiskPolicy::Avoid
                    } else if walk_bit.resolve(effective_level == DangerLevel::Always, script_grant)
                    {
                        RiskPolicy::Proceed
                    } else {
                        match effective_level {
                            DangerLevel::Never | DangerLevel::Always => RiskPolicy::Avoid,
                            DangerLevel::WhenSurvivable => RiskPolicy::Inherit,
                        }
                    };
                    assert_eq!(
                        globals.risk_policy(script, walk),
                        expected,
                        "stored={stored_level:?}, effective={effective_level:?}, script={script_grant}, walk={walk_bit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn complete_danger_precedence_table_is_tested_independently_of_net_activation() {
        for level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            for script_grant in [false, true] {
                let script = WalkPermissions {
                    allow_danger_zones: script_grant,
                    ..WalkPermissions::default()
                };
                for walk_bit in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
                    let walk = WalkOptions {
                        allow_danger_zones: walk_bit,
                        ..WalkOptions::default()
                    };
                    let expected = match (level, script_grant, walk_bit) {
                        (_, _, WalkBit::Forbid) => RiskPolicy::Avoid,
                        (_, _, WalkBit::Allow) | (_, true, WalkBit::Inherit) => RiskPolicy::Proceed,
                        (DangerLevel::Never, false, WalkBit::Inherit) => RiskPolicy::Avoid,
                        (DangerLevel::WhenSurvivable, false, WalkBit::Inherit) => {
                            RiskPolicy::Inherit
                        }
                        (DangerLevel::Always, false, WalkBit::Inherit) => RiskPolicy::Proceed,
                    };
                    assert_eq!(
                        level.risk_policy(script, walk),
                        expected,
                        "level={level:?}, script={script_grant}, walk={walk_bit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn inherited_avoid_does_not_remove_named_zone_grants() {
        use nav::zones::ZoneKey;

        let named = ZoneExempt::named(&[ZoneKey::Zone(7)]).unwrap();
        let globals = WalkGlobals {
            survivable_routing: false,
            ..WalkGlobals::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".into(),
            super::super::NavBot {
                native_permissions: Some(WalkPermissions::default()),
                walk_globals: Some(Arc::new(Mutex::new(globals))),
                ..Default::default()
            },
        )])));
        let resolved = compiled_options(
            &navs,
            "alice",
            FindOptions {
                zones: named,
                ..FindOptions::default()
            },
        );
        assert_eq!(resolved.zones, named);
        assert!(!resolved.zones.is_all());
    }
}

#[cfg(test)]
mod held_gate_tests {
    use super::*;

    #[test]
    fn inherited_and_explicit_enforcement_are_distinct_while_held() {
        const { assert!(!NET_AVAILABLE) };
        for level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            let mut globals = WalkGlobals::default();
            globals.set_danger_level(level);
            assert!(!globals.enforces_risk(WalkPermissions::default(), WalkOptions::default()));
            for bit in [WalkBit::Allow, WalkBit::Forbid] {
                assert!(globals.enforces_risk(
                    WalkPermissions::default(),
                    WalkOptions {
                        allow_danger_zones: bit,
                        ..Default::default()
                    },
                ));
            }
            assert!(globals.enforces_risk(
                WalkPermissions {
                    allow_danger_zones: true,
                    ..Default::default()
                },
                WalkOptions::default(),
            ));
        }
    }

    #[test]
    fn repeated_native_compiled_and_manual_admissions_share_one_session_note() {
        let navs = Arc::new(Mutex::new(HashMap::from([
            ("alice".to_owned(), super::super::NavBot::default()),
            ("bob".to_owned(), super::super::NavBot::default()),
        ])));
        assert!(!navs.lock().unwrap()["alice"].runtime_gate_logged);
        native_admission(&navs, "alice", WalkOptions::default(), None);
        assert!(navs.lock().unwrap()["alice"].runtime_gate_logged);
        assert!(!navs.lock().unwrap()["bob"].runtime_gate_logged);
        for _ in 0..3 {
            compiled_options(&navs, "alice", FindOptions::default());
            native_admission(&navs, "alice", WalkOptions::default(), None);
            let mut all = navs.lock().unwrap();
            log_runtime_net_gate(
                "alice",
                WalkGlobals::default(),
                &mut all.get_mut("alice").unwrap().runtime_gate_logged,
            );
            assert!(all["alice"].runtime_gate_logged);
        }
    }
}
