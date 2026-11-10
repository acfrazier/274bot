//! Base world access and path-local location state for the interpreter.

use crate::profile::{EngineProfile, LocDuration};
use crate::{Coord, LocTypeId, Ticks};
use thiserror::Error;

/// Stable identity for a location in the caller's placement table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacedId(pub u32);

/// One 8×8 map zone, in zone coordinates and a height plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneKey {
    /// East-west zone coordinate, equal to the tile x coordinate divided by eight.
    pub x: i32,
    /// North-south zone coordinate, equal to the tile z coordinate divided by eight.
    pub z: i32,
    /// Height plane.
    pub level: i32,
}

impl ZoneKey {
    /// Return the zone that contains `coord`.
    pub const fn of(coord: Coord) -> Self {
        Self {
            x: coord.x() >> 3,
            z: coord.z() >> 3,
            level: coord.level(),
        }
    }
}

/// Identity of one engine location object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LocObj {
    /// A location loaded from the world's placement table.
    Placed(PlacedId),
    /// A dynamic location created by this path. The id is never reused in that path.
    Created(u32),
}

/// Type, shape, and angle of a location.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocInfo {
    /// Location type id.
    pub ty: LocTypeId,
    /// Engine location shape.
    pub shape: u8,
    /// Engine location angle.
    pub angle: u8,
}

/// Life-cycle policy of one location object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Life {
    /// A placed location returns to its base state after its life ends.
    Respawn,
    /// A created location leaves the zone list after its life ends.
    Despawn,
}

/// Path-local state for one stable location object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocRecord {
    /// Fixed packed tile coordinate.
    pub tile: Coord,
    /// Fixed layer, derived from the base shape (`src/engine/entity/Loc.ts:42-52`).
    pub layer: u8,
    /// Fixed life-cycle policy.
    pub life: Life,
    /// State to which a respawning location returns.
    pub base: LocInfo,
    /// Current type, shape, and angle.
    pub cur: LocInfo,
    /// Whether the engine currently exposes this location.
    pub active: bool,
    /// Inclusive tick bounds until the life ends. `None` means no life runs.
    pub left: Option<Ticks>,
    /// The life may have ended, so current state and activity are unknown.
    pub unknown: bool,
}

/// One location loaded from the immutable base world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlacedLoc {
    /// Stable id in the world's placement table.
    pub id: PlacedId,
    /// Fixed packed tile coordinate.
    pub tile: Coord,
    /// Base type, shape, and angle.
    pub info: LocInfo,
}

/// Three-valued fact supplied by the world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tri {
    /// The fact is true.
    Yes,
    /// The fact is false.
    No,
    /// The world cannot prove either value.
    #[default]
    Unknown,
}

/// Allocation status of a destination zone on members and free worlds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Alloc {
    /// Whether the zone is allocated on a members world.
    pub on_members: Tri,
    /// Whether the zone is allocated on a free world.
    pub on_free: Tri,
}

/// Immutable base-world queries used by the symbolic interpreter.
pub trait World {
    /// Return the placed locations of one 8×8 zone in engine list order.
    /// The engine loads only active placements and appends them in map order
    /// (`src/engine/GameMap.ts:284-286`, `src/engine/zone/Zone.ts:229-233`).
    fn zone_locs(&self, zone: ZoneKey) -> &[PlacedLoc];

    /// Resolve a placed-location identity without scanning zone lists.
    fn placed_loc(&self, id: PlacedId) -> Option<PlacedLoc>;

    /// Return allocation facts for the zone that contains `tile`.
    fn zone_alloc(&self, tile: Coord) -> Alloc;

    /// Return whether the NPC list has spare capacity when known.
    ///
    /// Real navigation worlds do not model live NPC population and keep the default `Unknown`.
    fn npc_spare_capacity(&self) -> Tri {
        Tri::Unknown
    }
}

/// An engine validator fault or a world state that the interpreter cannot prove.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WorldError {
    /// The engine rejected a packed coordinate outside its accepted integer range.
    #[error("engine rejected packed coordinate {value}")]
    InvalidCoordinate {
        /// Rejected packed coordinate value.
        value: i32,
    },
    /// The type id is outside the location type encoding.
    #[error("engine rejected location type id {value}")]
    InvalidLocType {
        /// Rejected type id.
        value: i32,
    },
    /// The engine rejected a location shape.
    #[error("engine rejected location shape {value}")]
    InvalidLocShape {
        /// Rejected shape value.
        value: u8,
    },
    /// The engine rejected a location angle.
    #[error("engine rejected location angle {value}")]
    InvalidLocAngle {
        /// Rejected angle value.
        value: u8,
    },
    /// The active engine profile rejected a location duration.
    #[error("engine rejected location duration {value}")]
    InvalidDuration {
        /// Rejected duration value.
        value: i32,
    },
    /// The elapsed tick interval is not ordered.
    #[error("invalid elapsed tick interval {min}..={max}")]
    InvalidTickRange {
        /// Least elapsed ticks.
        min: u32,
        /// Greatest elapsed ticks.
        max: u32,
    },
    /// A zone coordinate is outside the engine's zone grid.
    #[error("invalid zone key {zone:?}")]
    InvalidZoneKey {
        /// Rejected zone key.
        zone: ZoneKey,
    },
    /// A base placement does not contain valid engine location data.
    #[error("invalid world placement {id:?}")]
    InvalidWorldPlacement {
        /// Invalid placement identity.
        id: PlacedId,
    },
    /// The world's zone list contains one placed id more than once.
    #[error("duplicate placed location {id:?}")]
    DuplicatePlacedId {
        /// Duplicate placement identity.
        id: PlacedId,
    },
    /// A zone list or its base placements do not match the requested zone.
    #[error("invalid location list for zone {zone:?}")]
    InvalidWorldZoneList {
        /// Zone whose location list is invalid.
        zone: ZoneKey,
    },
    /// A cached location life can have ended.
    #[error("location {obj:?} has an uncertain life state")]
    UnknownLifeState {
        /// Location whose state is uncertain.
        obj: LocObj,
    },
    /// A zone list contains a location with an uncertain life state.
    #[error("zone {zone:?} contains a location with an uncertain life state")]
    UnknownZoneState {
        /// Zone whose list cannot be used.
        zone: ZoneKey,
    },
    /// The world cannot resolve a placed id.
    #[error("world cannot resolve placed location {obj:?}")]
    MissingPlacedObject {
        /// Unresolved location identity.
        obj: LocObj,
    },
    /// The overlay has no record for a created id.
    #[error("overlay has no created location {obj:?}")]
    MissingCreatedObject {
        /// Unresolved location identity.
        obj: LocObj,
    },
    /// A location that must be linked is absent from its zone list.
    #[error("location {obj:?} is absent from zone {zone:?}")]
    ObjectNotInZone {
        /// Missing location identity.
        obj: LocObj,
        /// Zone that should contain the location.
        zone: ZoneKey,
    },
    /// The path exhausted the stable created-location id space.
    #[error("created location id space is exhausted")]
    CreatedIdExhausted,
    /// The zone-list mutation counter cannot advance without wrapping.
    #[error("zone-list stamp is exhausted for {zone:?}")]
    ListStampExhausted {
        /// Zone whose stamp cannot advance.
        zone: ZoneKey,
    },
}

impl WorldError {
    /// Return whether this error is an engine validator fault.
    pub const fn is_engine_fault(&self) -> bool {
        matches!(
            self,
            Self::InvalidCoordinate { .. }
                | Self::InvalidLocType { .. }
                | Self::InvalidLocShape { .. }
                | Self::InvalidLocAngle { .. }
                | Self::InvalidDuration { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectEntry {
    obj: LocObj,
    record: LocRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ZoneList {
    key: ZoneKey,
    order: Vec<LocObj>,
    stamp: u32,
}

#[derive(Clone, Copy)]
enum ZoneOrder<'a> {
    Base(&'a [PlacedLoc]),
    Overlay(&'a [LocObj]),
}

impl ZoneOrder<'_> {
    fn len(self) -> usize {
        match self {
            Self::Base(locs) => locs.len(),
            Self::Overlay(locs) => locs.len(),
        }
    }

    fn get(self, index: usize) -> Option<LocObj> {
        match self {
            Self::Base(locs) => locs.get(index).map(|loc| LocObj::Placed(loc.id)),
            Self::Overlay(locs) => locs.get(index).copied(),
        }
    }

    fn position(self, obj: LocObj) -> Option<usize> {
        (0..self.len()).find(|&index| self.get(index) == Some(obj))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Aging {
    Keep(Ticks),
    Uncertain(Ticks),
    Expired,
}

/// Copy-on-write location objects and ordered zone lists for one interpreter path.
///
/// The overlay has no bot-owned or cross-entry state. Its vector capacities are available through
/// [`WorldOverlay::owned_bytes`] for the entry memory bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldOverlay {
    objects: Vec<ObjectEntry>,
    zones: Vec<ZoneList>,
    created: u32,
    loc_duration: LocDuration,
}

impl Default for WorldOverlay {
    fn default() -> Self {
        Self::with_profile(EngineProfile::R289)
    }
}

impl WorldOverlay {
    /// Create a location overlay using `profile`'s location-duration policy.
    pub fn with_profile(profile: EngineProfile) -> Self {
        Self {
            objects: Vec::new(),
            zones: Vec::new(),
            created: 0,
            loc_duration: profile.loc_duration,
        }
    }

    fn loc_life(&self, tile: Coord, duration: i32) -> Result<Option<Ticks>, WorldError> {
        validate_duration_policy(duration, self.loc_duration)?;
        match (duration, self.loc_duration) {
            (0, LocDuration::ZeroInInstance { min_x }) if tile.x() >= min_x => Ok(None),
            (0, _) => Err(WorldError::InvalidDuration { value: duration }),
            (positive, _) => Ok(Some(exact_ticks(positive))),
        }
    }
    /// Add a location or change the first location on `tile` with the layer of `info.shape`.
    ///
    /// The search follows the zone's current list order and includes inactive placed locations.
    /// Mirrors `src/engine/script/handlers/LocOps.ts:18-42` and the unsafe ordered search in
    /// `src/engine/zone/Zone.ts:508-514`.
    /// Zero is permanent only when the selected profile allows it at `tile`.
    pub fn loc_add(
        &mut self,
        w: &dyn World,
        tile: Coord,
        info: LocInfo,
        duration: i32,
    ) -> Result<LocObj, WorldError> {
        validate_coord(tile)?;
        validate_loc_type(info.ty)?;
        validate_angle(info.angle)?;
        let layer = loc_shape_layer(info.shape)?;
        let life = self.loc_life(tile, duration)?;

        let zone = ZoneKey::of(tile);
        let order = self.zone_order(w, zone)?;
        self.ensure_zone_known(w, zone, order)?;
        let mut found = None;
        for index in 0..order.len() {
            let obj = order
                .get(index)
                .ok_or(WorldError::InvalidWorldZoneList { zone })?;
            let record = self.record(w, obj)?;
            if record.tile == tile && record.layer == layer {
                found = Some(obj);
                break;
            }
        }
        if let Some(obj) = found {
            self.change_existing(w, obj, info, life)?;
            return Ok(obj);
        }

        let id = self.created;
        let next_created = id.checked_add(1).ok_or(WorldError::CreatedIdExhausted)?;
        let next_stamp = self.next_stamp(zone)?;
        let zone_index = self.ensure_zone_cached(w, zone)?;
        let obj = LocObj::Created(id);
        self.insert_record(
            obj,
            LocRecord {
                tile,
                layer,
                life: Life::Despawn,
                base: info,
                cur: info,
                active: true,
                left: life,
                unknown: false,
            },
        );
        self.zones[zone_index].order.push(obj);
        self.zones[zone_index].stamp = next_stamp;
        self.created = next_created;
        Ok(obj)
    }

    /// Change the type of one object while preserving its current shape and angle.
    /// A zero duration uses the selected profile's rule at the object's fixed tile.
    pub fn loc_change(
        &mut self,
        w: &dyn World,
        obj: LocObj,
        ty: LocTypeId,
        duration: i32,
    ) -> Result<(), WorldError> {
        validate_duration_policy(duration, self.loc_duration)?;
        let life = if duration == 0 {
            self.loc_life(self.fixed_tile(w, obj)?, duration)?
        } else {
            Some(exact_ticks(duration))
        };
        validate_loc_type(ty)?;
        let previous = self.record(w, obj)?;
        if matches!(obj, LocObj::Created(_)) && !previous.active {
            return Ok(());
        }
        let mut info = previous.cur;
        info.ty = ty;
        self.change_existing(w, obj, info, life)
    }

    /// Delete an active object, or do nothing when the object is already inactive.
    /// Mirrors `src/engine/World.ts:1417-1439` and `src/engine/zone/Zone.ts:266-281`.
    pub fn loc_del(&mut self, w: &dyn World, obj: LocObj, duration: i32) -> Result<(), WorldError> {
        validate_positive_duration(duration)?;
        let record = self.record(w, obj)?;
        if !record.active {
            return Ok(());
        }

        let zone = ZoneKey::of(record.tile);
        let order = self.zone_order(w, zone)?;
        self.ensure_zone_known(w, zone, order)?;
        if order.position(obj).is_none() {
            return Err(WorldError::ObjectNotInZone { obj, zone });
        }
        let next_stamp = self.next_stamp(zone)?;
        let zone_index = self.ensure_zone_cached(w, zone)?;
        let object_index = self.ensure_record_cached(w, obj)?;
        let list = &mut self.zones[zone_index].order;
        let position = list
            .iter()
            .position(|&candidate| candidate == obj)
            .ok_or(WorldError::ObjectNotInZone { obj, zone })?;
        list.remove(position);
        if record.life == Life::Respawn {
            list.push(obj);
        }
        let object = &mut self.objects[object_index].record;
        object.active = false;
        object.left = if record.life == Life::Respawn {
            Some(exact_ticks(duration))
        } else {
            None
        };
        self.zones[zone_index].stamp = next_stamp;
        Ok(())
    }

    /// Find the first active object of `ty` on `tile` in current zone-list order.
    /// Mirrors `src/engine/script/handlers/LocOps.ts:79-94` and `src/engine/zone/Zone.ts:283-290`.
    pub fn loc_find(
        &self,
        w: &dyn World,
        tile: Coord,
        ty: LocTypeId,
    ) -> Result<Option<LocObj>, WorldError> {
        validate_loc_type(ty)?;
        validate_coord(tile)?;
        let zone = ZoneKey::of(tile);
        let order = self.zone_order(w, zone)?;
        self.ensure_zone_known(w, zone, order)?;
        for index in 0..order.len() {
            let obj = order
                .get(index)
                .ok_or(WorldError::InvalidWorldZoneList { zone })?;
            let record = self.record(w, obj)?;
            if record.active && record.tile == tile && record.cur.ty == ty {
                return Ok(Some(obj));
            }
        }
        Ok(None)
    }

    /// Return active objects in one zone, from the end of its list to the start.
    /// Mirrors `src/engine/script/handlers/LocOps.ts:96-112` and `src/engine/script/ScriptIterators.ts:377-383`.
    /// `src/engine/zone/Zone.ts:483-489` filters to active objects; `src/datastruct/LinkList.ts:97-109` reverses order.
    pub fn zone_iter(&self, w: &dyn World, zone: ZoneKey) -> Result<Vec<LocObj>, WorldError> {
        let order = self.zone_order(w, zone)?;
        self.ensure_zone_known(w, zone, order)?;
        let mut objects = Vec::with_capacity(order.len());
        for index in (0..order.len()).rev() {
            let obj = order
                .get(index)
                .ok_or(WorldError::InvalidWorldZoneList { zone })?;
            if self.record(w, obj)?.active {
                objects.push(obj);
            }
        }
        Ok(objects)
    }

    /// Read the current type, shape, angle, and activity of one object.
    pub fn read(&self, w: &dyn World, obj: LocObj) -> Result<(LocInfo, bool), WorldError> {
        let record = self.record(w, obj)?;
        Ok((record.cur, record.active))
    }

    /// Read the fixed tile of one object.
    pub fn tile(&self, w: &dyn World, obj: LocObj) -> Result<Coord, WorldError> {
        Ok(self.record(w, obj)?.tile)
    }

    /// Return the current list-change stamp for `zone`, or zero when it has not changed.
    pub fn stamp(&self, zone: ZoneKey) -> u32 {
        self.zone_index(zone)
            .map(|index| self.zones[index].stamp)
            .unwrap_or(0)
    }

    /// Age all location lives by an inclusive elapsed-tick interval.
    ///
    /// An interval that can end a life but cannot prove that it did marks that object unknown.
    /// Later object or zone-list access then returns an unsupported-state error.
    /// Mirrors `src/engine/entity/Loc.ts:54-73` and the zone phase in `src/engine/World.ts:963-970`.
    pub fn age(&mut self, w: &dyn World, elapsed: Ticks) -> Result<(), WorldError> {
        if elapsed.min > elapsed.max {
            return Err(WorldError::InvalidTickRange {
                min: elapsed.min,
                max: elapsed.max,
            });
        }

        for index in 0..self.objects.len() {
            let record = self.objects[index].record;
            if record.unknown || !matches!(age_result(record.left, elapsed), Some(Aging::Expired)) {
                continue;
            }
            if !expiry_changes_zone(&record) {
                continue;
            }
            let obj = self.objects[index].obj;
            let zone = ZoneKey::of(record.tile);
            let order = self.zone_order(w, zone)?;
            self.ensure_zone_known_after_age(w, zone, order, elapsed)?;
            if order.position(obj).is_none() {
                return Err(WorldError::ObjectNotInZone { obj, zone });
            }
            let mut prior_events = 0u32;
            for prior in &self.objects[..index] {
                if prior.record.unknown
                    || age_result(prior.record.left, elapsed) != Some(Aging::Expired)
                    || !expiry_changes_zone(&prior.record)
                    || ZoneKey::of(prior.record.tile) != zone
                {
                    continue;
                }
                prior_events = prior_events.saturating_add(1);
            }
            if self
                .stamp(zone)
                .checked_add(prior_events)
                .and_then(|stamp| stamp.checked_add(1))
                .is_none()
            {
                return Err(WorldError::ListStampExhausted { zone });
            }
            self.ensure_zone_cached(w, zone)?;
        }

        for index in 0..self.objects.len() {
            let record = self.objects[index].record;
            if record.unknown {
                continue;
            }
            let Some(left) = record.left else {
                continue;
            };
            match age_result(Some(left), elapsed) {
                Some(Aging::Keep(remaining)) => self.objects[index].record.left = Some(remaining),
                Some(Aging::Uncertain(remaining)) => {
                    self.objects[index].record.left = Some(remaining);
                    self.objects[index].record.unknown = true;
                }
                Some(Aging::Expired) => self.expire(index, record)?,
                None => {}
            }
        }
        Ok(())
    }

    /// Return the capacity bytes owned by overlay vectors and copied zone orders.
    ///
    /// This includes the object and zone entry vectors and every nested zone-order vector. It does
    /// not count borrowed world slices or vectors returned to callers by [`WorldOverlay::zone_iter`].
    pub fn owned_bytes(&self) -> usize {
        let mut bytes = self
            .objects
            .capacity()
            .saturating_mul(std::mem::size_of::<ObjectEntry>());
        bytes = bytes.saturating_add(
            self.zones
                .capacity()
                .saturating_mul(std::mem::size_of::<ZoneList>()),
        );
        for zone in &self.zones {
            bytes = bytes.saturating_add(
                zone.order
                    .capacity()
                    .saturating_mul(std::mem::size_of::<LocObj>()),
            );
        }
        bytes
    }

    fn change_existing(
        &mut self,
        w: &dyn World,
        obj: LocObj,
        info: LocInfo,
        life: Option<Ticks>,
    ) -> Result<(), WorldError> {
        // World.changeLoc and Zone.changeLoc define these active, life, and list transitions
        // (`src/engine/World.ts:1365-1400`, `src/engine/zone/Zone.ts:243-264`).
        let previous = self.record(w, obj)?;
        if matches!(obj, LocObj::Created(_)) && !previous.active {
            return Ok(());
        }
        let zone = ZoneKey::of(previous.tile);
        let order = self.zone_order(w, zone)?;
        self.ensure_zone_known(w, zone, order)?;
        if order.position(obj).is_none() {
            return Err(WorldError::ObjectNotInZone { obj, zone });
        }
        let next_stamp = self.next_stamp(zone)?;
        let zone_index = self.ensure_zone_cached(w, zone)?;
        let object_index = self.ensure_record_cached(w, obj)?;
        let record = &mut self.objects[object_index].record;
        record.cur = info;
        record.active = true;
        record.left = if record.life == Life::Despawn || record.cur != record.base {
            life
        } else {
            None
        };
        move_to_end(&mut self.zones[zone_index].order, obj);
        self.zones[zone_index].stamp = next_stamp;
        Ok(())
    }

    // Loc.turn delegates each expiry to World according to lifecycle and active state
    // (`src/engine/entity/Loc.ts:54-73`, `src/engine/World.ts:1442-1461`).
    fn expire(&mut self, index: usize, record: LocRecord) -> Result<(), WorldError> {
        let obj = self.objects[index].obj;
        let zone = ZoneKey::of(record.tile);
        match record.life {
            Life::Respawn if !record.active => {
                let stamp = self.next_stamp(zone)?;
                let zone_index = self
                    .zone_index(zone)
                    .ok_or(WorldError::ObjectNotInZone { obj, zone })?;
                let current = &mut self.objects[index].record;
                current.cur = current.base;
                current.active = true;
                current.left = None;
                self.zones[zone_index].stamp = stamp;
            }
            Life::Respawn if record.cur != record.base => {
                let stamp = self.next_stamp(zone)?;
                let zone_index = self
                    .zone_index(zone)
                    .ok_or(WorldError::ObjectNotInZone { obj, zone })?;
                move_to_end(&mut self.zones[zone_index].order, obj);
                let current = &mut self.objects[index].record;
                current.cur = current.base;
                current.active = true;
                current.left = None;
                self.zones[zone_index].stamp = stamp;
            }
            Life::Despawn if record.active => {
                let stamp = self.next_stamp(zone)?;
                let zone_index = self
                    .zone_index(zone)
                    .ok_or(WorldError::ObjectNotInZone { obj, zone })?;
                let position = self.zones[zone_index]
                    .order
                    .iter()
                    .position(|&candidate| candidate == obj)
                    .ok_or(WorldError::ObjectNotInZone { obj, zone })?;
                self.zones[zone_index].order.remove(position);
                let current = &mut self.objects[index].record;
                current.active = false;
                current.left = None;
                self.zones[zone_index].stamp = stamp;
            }
            _ => self.objects[index].record.left = None,
        }
        Ok(())
    }

    fn ensure_zone_known(
        &self,
        w: &dyn World,
        zone: ZoneKey,
        order: ZoneOrder<'_>,
    ) -> Result<(), WorldError> {
        for index in 0..order.len() {
            let obj = order
                .get(index)
                .ok_or(WorldError::InvalidWorldZoneList { zone })?;
            match self.record(w, obj) {
                Ok(_) => {}
                Err(WorldError::UnknownLifeState { .. }) => {
                    return Err(WorldError::UnknownZoneState { zone });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn ensure_zone_known_after_age(
        &self,
        w: &dyn World,
        zone: ZoneKey,
        order: ZoneOrder<'_>,
        elapsed: Ticks,
    ) -> Result<(), WorldError> {
        for index in 0..order.len() {
            let obj = order
                .get(index)
                .ok_or(WorldError::InvalidWorldZoneList { zone })?;
            let record = match self.record(w, obj) {
                Ok(record) => record,
                Err(WorldError::UnknownLifeState { .. }) => {
                    return Err(WorldError::UnknownZoneState { zone });
                }
                Err(error) => return Err(error),
            };
            if matches!(age_result(record.left, elapsed), Some(Aging::Uncertain(_))) {
                return Err(WorldError::UnknownZoneState { zone });
            }
        }
        Ok(())
    }

    fn fixed_tile(&self, w: &dyn World, obj: LocObj) -> Result<Coord, WorldError> {
        if let Some(index) = self.object_index(obj) {
            return Ok(self.objects[index].record.tile);
        }
        match obj {
            LocObj::Placed(id) => {
                let placement = w
                    .placed_loc(id)
                    .ok_or(WorldError::MissingPlacedObject { obj })?;
                if placement.id != id {
                    return Err(WorldError::InvalidWorldPlacement { id });
                }
                Ok(record_from_placed(placement)?.tile)
            }
            LocObj::Created(_) => Err(WorldError::MissingCreatedObject { obj }),
        }
    }

    fn record(&self, w: &dyn World, obj: LocObj) -> Result<LocRecord, WorldError> {
        if let Some(index) = self.object_index(obj) {
            let record = self.objects[index].record;
            if record.unknown {
                return Err(WorldError::UnknownLifeState { obj });
            }
            return Ok(record);
        }
        match obj {
            LocObj::Placed(id) => {
                let placement = w
                    .placed_loc(id)
                    .ok_or(WorldError::MissingPlacedObject { obj })?;
                if placement.id != id {
                    return Err(WorldError::InvalidWorldPlacement { id });
                }
                Ok(record_from_placed(placement)?)
            }
            LocObj::Created(_) => Err(WorldError::MissingCreatedObject { obj }),
        }
    }

    fn ensure_record_cached(&mut self, w: &dyn World, obj: LocObj) -> Result<usize, WorldError> {
        if let Some(index) = self.object_index(obj) {
            if self.objects[index].record.unknown {
                return Err(WorldError::UnknownLifeState { obj });
            }
            return Ok(index);
        }
        let record = match obj {
            LocObj::Placed(id) => {
                let placement = w
                    .placed_loc(id)
                    .ok_or(WorldError::MissingPlacedObject { obj })?;
                if placement.id != id {
                    return Err(WorldError::InvalidWorldPlacement { id });
                }
                record_from_placed(placement)?
            }
            LocObj::Created(_) => return Err(WorldError::MissingCreatedObject { obj }),
        };
        Ok(self.insert_record(obj, record))
    }

    fn insert_record(&mut self, obj: LocObj, record: LocRecord) -> usize {
        match self.objects.binary_search_by_key(&obj, |entry| entry.obj) {
            Ok(index) => {
                self.objects[index].record = record;
                index
            }
            Err(index) => {
                self.objects.insert(index, ObjectEntry { obj, record });
                index
            }
        }
    }

    fn zone_order<'a>(
        &'a self,
        w: &'a dyn World,
        zone: ZoneKey,
    ) -> Result<ZoneOrder<'a>, WorldError> {
        validate_zone_key(zone)?;
        if let Some(index) = self.zone_index(zone) {
            return Ok(ZoneOrder::Overlay(&self.zones[index].order));
        }
        let placements = w.zone_locs(zone);
        validate_world_zone(zone, placements)?;
        Ok(ZoneOrder::Base(placements))
    }

    fn ensure_zone_cached(&mut self, w: &dyn World, zone: ZoneKey) -> Result<usize, WorldError> {
        if let Some(index) = self.zone_index(zone) {
            return Ok(index);
        }
        validate_zone_key(zone)?;
        let placements = w.zone_locs(zone);
        validate_world_zone(zone, placements)?;
        let mut order = Vec::with_capacity(placements.len());
        for placement in placements {
            order.push(LocObj::Placed(placement.id));
        }
        let entry = ZoneList {
            key: zone,
            order,
            stamp: 0,
        };
        let index = match self.zones.binary_search_by_key(&zone, |item| item.key) {
            Ok(index) => index,
            Err(index) => {
                self.zones.insert(index, entry);
                index
            }
        };
        Ok(index)
    }

    fn next_stamp(&self, zone: ZoneKey) -> Result<u32, WorldError> {
        self.stamp(zone)
            .checked_add(1)
            .ok_or(WorldError::ListStampExhausted { zone })
    }

    fn object_index(&self, obj: LocObj) -> Option<usize> {
        self.objects
            .binary_search_by_key(&obj, |entry| entry.obj)
            .ok()
    }

    fn zone_index(&self, zone: ZoneKey) -> Option<usize> {
        self.zones
            .binary_search_by_key(&zone, |entry| entry.key)
            .ok()
    }
}

// The script validator accepts packed coordinates in 0..=i32::MAX
// (`src/engine/script/ScriptValidators.ts:94-100`); CoordGrid unpacks their fields
// (`src/engine/CoordGrid.ts:129-133`).
fn validate_coord(coord: Coord) -> Result<(), WorldError> {
    if coord.0 < 0 {
        return Err(WorldError::InvalidCoordinate { value: coord.0 });
    }
    Ok(())
}

// Loc packs a 14-bit type id (`src/engine/entity/Loc.ts:20-24`). The command validator also checks
// the selected pack's type count (`src/engine/script/ScriptValidators.ts:106`).
fn validate_loc_type(ty: LocTypeId) -> Result<(), WorldError> {
    if !(0..=0x3fff).contains(&ty.0) {
        return Err(WorldError::InvalidLocType { value: ty.0 });
    }
    Ok(())
}

fn validate_angle(angle: u8) -> Result<(), WorldError> {
    if angle > 3 {
        return Err(WorldError::InvalidLocAngle { value: angle });
    }
    Ok(())
}

// Matches src/engine/routefinder/flags.ts:120-150.
fn loc_shape_layer(shape: u8) -> Result<u8, WorldError> {
    match shape {
        0..=3 => Ok(0),
        4..=8 => Ok(1),
        9..=21 => Ok(2),
        22 => Ok(3),
        _ => Err(WorldError::InvalidLocShape { value: shape }),
    }
}

fn validate_loc_info(info: LocInfo) -> Result<u8, WorldError> {
    validate_loc_type(info.ty)?;
    validate_angle(info.angle)?;
    loc_shape_layer(info.shape)
}

// R377's LOC_ADD and LOC_CHANGE handlers allow zero only at instance coordinates
// (`src/engine/script/handlers/LocOps.ts:26-27,64-65`, `src/engine/CoordGrid.ts:24-27`).
fn validate_duration_policy(duration: i32, policy: LocDuration) -> Result<(), WorldError> {
    if duration < 0 || (duration == 0 && policy == LocDuration::Positive) {
        return Err(WorldError::InvalidDuration { value: duration });
    }
    Ok(())
}

// LOC_DEL keeps the positive duration validator (`src/engine/script/handlers/LocOps.ts:74-76`).
fn validate_positive_duration(duration: i32) -> Result<(), WorldError> {
    if duration < 1 {
        return Err(WorldError::InvalidDuration { value: duration });
    }
    Ok(())
}

fn validate_zone_key(zone: ZoneKey) -> Result<(), WorldError> {
    if !(0..=2047).contains(&zone.x)
        || !(0..=2047).contains(&zone.z)
        || !(0..=3).contains(&zone.level)
    {
        return Err(WorldError::InvalidZoneKey { zone });
    }
    Ok(())
}

fn validate_world_zone(zone: ZoneKey, placements: &[PlacedLoc]) -> Result<(), WorldError> {
    for (index, placement) in placements.iter().enumerate() {
        if validate_coord(placement.tile).is_err()
            || ZoneKey::of(placement.tile) != zone
            || validate_loc_info(placement.info).is_err()
        {
            return Err(WorldError::InvalidWorldPlacement { id: placement.id });
        }
        if placements[..index]
            .iter()
            .any(|previous| previous.id == placement.id)
        {
            return Err(WorldError::DuplicatePlacedId { id: placement.id });
        }
    }
    Ok(())
}

fn record_from_placed(placement: PlacedLoc) -> Result<LocRecord, WorldError> {
    let layer = validate_loc_info(placement.info)
        .map_err(|_| WorldError::InvalidWorldPlacement { id: placement.id })?;
    validate_coord(placement.tile)
        .map_err(|_| WorldError::InvalidWorldPlacement { id: placement.id })?;
    Ok(LocRecord {
        tile: placement.tile,
        layer,
        life: Life::Respawn,
        base: placement.info,
        cur: placement.info,
        active: true,
        left: None,
        unknown: false,
    })
}

fn exact_ticks(duration: i32) -> Ticks {
    let ticks = duration as u32;
    Ticks {
        min: ticks,
        max: ticks,
    }
}

// Entity stores at least one tick and World decrements life in the zone phase
// (`src/engine/entity/Entity.ts:36-43`, `src/engine/World.ts:963-970`).
fn age_result(left: Option<Ticks>, elapsed: Ticks) -> Option<Aging> {
    let left = left?;
    if left.min > left.max {
        return None;
    }
    let remaining = Ticks {
        min: left.min.saturating_sub(elapsed.max),
        max: left.max.saturating_sub(elapsed.min),
    };
    if remaining.max == 0 {
        Some(Aging::Expired)
    } else if remaining.min == 0 {
        Some(Aging::Uncertain(remaining))
    } else {
        Some(Aging::Keep(remaining))
    }
}

fn expiry_changes_zone(record: &LocRecord) -> bool {
    match record.life {
        Life::Respawn => !record.active || record.cur != record.base,
        Life::Despawn => record.active,
    }
}

fn move_to_end(order: &mut Vec<LocObj>, obj: LocObj) {
    if let Some(position) = order.iter().position(|&candidate| candidate == obj) {
        let moved = order.remove(position);
        order.push(moved);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Alloc, LocInfo, LocObj, PlacedId, PlacedLoc, Tri, World, WorldError, WorldOverlay, ZoneKey,
    };
    use crate::profile::EngineProfile;
    use crate::{Coord, LocTypeId, Ticks};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Fixture {
        zones: BTreeMap<ZoneKey, Vec<PlacedLoc>>,
        placements: BTreeMap<PlacedId, PlacedLoc>,
        alloc: Alloc,
        spare: Tri,
    }

    impl Fixture {
        fn with_placed(locs: &[PlacedLoc]) -> Self {
            let mut fixture = Self::default();
            for &loc in locs {
                fixture
                    .zones
                    .entry(ZoneKey::of(loc.tile))
                    .or_default()
                    .push(loc);
                fixture.placements.insert(loc.id, loc);
            }
            fixture
        }
    }

    impl World for Fixture {
        fn zone_locs(&self, zone: ZoneKey) -> &[PlacedLoc] {
            self.zones.get(&zone).map(Vec::as_slice).unwrap_or(&[])
        }

        fn placed_loc(&self, id: PlacedId) -> Option<PlacedLoc> {
            self.placements.get(&id).copied()
        }

        fn zone_alloc(&self, _tile: Coord) -> Alloc {
            self.alloc
        }

        fn npc_spare_capacity(&self) -> Tri {
            self.spare
        }
    }

    struct DefaultCapacityWorld;

    impl World for DefaultCapacityWorld {
        fn zone_locs(&self, _zone: ZoneKey) -> &[PlacedLoc] {
            &[]
        }

        fn placed_loc(&self, _id: PlacedId) -> Option<PlacedLoc> {
            None
        }

        fn zone_alloc(&self, _tile: Coord) -> Alloc {
            Alloc::default()
        }
    }

    fn tile(x: i32, z: i32) -> Coord {
        Coord::new(x, 0, z)
    }

    fn info(ty: i32, shape: u8) -> LocInfo {
        LocInfo {
            ty: LocTypeId(ty),
            shape,
            angle: 0,
        }
    }

    fn placed(id: u32, tile: Coord, info: LocInfo) -> PlacedLoc {
        PlacedLoc {
            id: PlacedId(id),
            tile,
            info,
        }
    }

    #[test]
    fn row_17_loc_identity_does_not_rebind_an_old_reference() {
        let world = Fixture::default();
        let at = tile(10, 20);
        let mut overlay = WorldOverlay::default();
        let first = overlay
            .loc_add(&world, at, info(1, 10), 3)
            .expect("first dynamic object is created");
        assert_eq!(first, LocObj::Created(0));
        assert_eq!(
            overlay
                .loc_find(&world, at, LocTypeId(1))
                .expect("find succeeds"),
            Some(first)
        );
        overlay
            .loc_del(&world, first, 1)
            .expect("first object is deleted");
        let second = overlay
            .loc_add(&world, at, info(2, 10), 3)
            .expect("a new dynamic object is created");
        assert_eq!(second, LocObj::Created(1));
        overlay
            .loc_change(&world, first, LocTypeId(3), 3)
            .expect("mutation of an inactive created object is a no-op");
        assert_eq!(
            overlay
                .read(&world, first)
                .expect("old identity remains readable")
                .0
                .ty,
            LocTypeId(1)
        );
        assert_eq!(
            overlay
                .read(&world, second)
                .expect("new identity remains active")
                .0
                .ty,
            LocTypeId(2)
        );
        overlay
            .age(&world, Ticks { min: 3, max: 3 })
            .expect("created object's life ends");
        assert_eq!(
            overlay
                .read(&world, second)
                .expect("expired object remains readable"),
            (info(2, 10), false)
        );
        assert_eq!(
            overlay
                .loc_find(&world, at, LocTypeId(2))
                .expect("expired object is absent"),
            None
        );
    }

    #[test]
    fn row_18_deleted_placed_object_is_reused_and_respawns() {
        let at = tile(10, 20);
        let world = Fixture::with_placed(&[placed(7, at, info(10, 0))]);
        let obj = LocObj::Placed(PlacedId(7));
        let mut overlay = WorldOverlay::default();
        overlay
            .loc_del(&world, obj, 500)
            .expect("placed object is deleted");
        assert_eq!(
            overlay
                .loc_add(&world, at, info(11, 0), 3)
                .expect("inactive placement is reused"),
            obj
        );
        overlay
            .age(&world, Ticks { min: 2, max: 2 })
            .expect("two ticks age the new life");
        assert_eq!(
            overlay
                .read(&world, obj)
                .expect("changed object remains active")
                .0
                .ty,
            LocTypeId(11)
        );
        overlay
            .age(&world, Ticks { min: 1, max: 1 })
            .expect("third tick ends the new life");
        assert_eq!(
            overlay
                .read(&world, obj)
                .expect("placed object reverts")
                .0
                .ty,
            LocTypeId(10)
        );
        assert_eq!(
            overlay
                .loc_find(&world, at, LocTypeId(10))
                .expect("base type is found"),
            Some(obj)
        );
    }

    #[test]
    fn row_19_second_delete_does_not_replace_the_first_life() {
        let at = tile(10, 20);
        let world = Fixture::with_placed(&[placed(8, at, info(10, 0))]);
        let obj = LocObj::Placed(PlacedId(8));
        let mut overlay = WorldOverlay::default();
        overlay
            .loc_del(&world, obj, 3)
            .expect("first delete starts a life");
        let stamp = overlay.stamp(ZoneKey::of(at));
        overlay
            .loc_del(&world, obj, 500)
            .expect("second delete is a no-op");
        assert_eq!(overlay.stamp(ZoneKey::of(at)), stamp);
        assert_eq!(
            overlay.objects[overlay.object_index(obj).expect("placed record is cached")]
                .record
                .left,
            Some(Ticks { min: 3, max: 3 })
        );
        overlay
            .age(&world, Ticks { min: 3, max: 3 })
            .expect("first life ends");
        assert_eq!(
            overlay
                .loc_find(&world, at, LocTypeId(10))
                .expect("respawned object is found"),
            Some(obj)
        );
    }

    #[test]
    fn row_20_loc_add_uses_order_after_moving_the_first_match() {
        let at = tile(10, 20);
        let first = placed(1, at, info(10, 0));
        let second = placed(2, at, info(11, 0));
        let world = Fixture::with_placed(&[first, second]);
        let mut overlay = WorldOverlay::default();
        assert_eq!(
            overlay
                .loc_add(&world, at, info(20, 0), 5)
                .expect("first placement changes"),
            LocObj::Placed(first.id)
        );
        assert_eq!(
            overlay
                .loc_add(&world, at, info(21, 0), 5)
                .expect("second placement changes"),
            LocObj::Placed(second.id)
        );
        assert_eq!(
            overlay
                .read(&world, LocObj::Placed(first.id))
                .expect("first state reads")
                .0
                .ty,
            LocTypeId(20)
        );
        assert_eq!(
            overlay
                .read(&world, LocObj::Placed(second.id))
                .expect("second state reads")
                .0
                .ty,
            LocTypeId(21)
        );
    }

    #[test]
    fn row_21_reads_observe_delete_add_change_and_fixed_tile() {
        let at = tile(10, 20);
        let world = Fixture::with_placed(&[placed(9, at, info(10, 0))]);
        let obj = LocObj::Placed(PlacedId(9));
        let mut overlay = WorldOverlay::default();
        overlay.loc_del(&world, obj, 5).expect("delete succeeds");
        assert_eq!(
            overlay
                .loc_find(&world, at, LocTypeId(10))
                .expect("deleted type lookup succeeds"),
            None
        );
        assert_eq!(
            overlay
                .loc_add(&world, at, info(11, 0), 5)
                .expect("add reuses placed object"),
            obj
        );
        assert_eq!(
            overlay.read(&world, obj).expect("add is visible").0,
            info(11, 0)
        );
        overlay
            .loc_change(&world, obj, LocTypeId(12), 5)
            .expect("change succeeds");
        assert_eq!(
            overlay.read(&world, obj).expect("change is visible").0.ty,
            LocTypeId(12)
        );
        assert_eq!(overlay.tile(&world, obj).expect("tile is stable"), at);
    }

    #[test]
    fn row_22_zone_iteration_is_reverse_order_and_skips_inactive_objects() {
        let at = tile(10, 20);
        let first = placed(10, at, info(10, 0));
        let second = placed(11, at, info(11, 0));
        let third = placed(12, at, info(12, 0));
        let world = Fixture::with_placed(&[first, second, third]);
        let mut overlay = WorldOverlay::default();
        overlay
            .loc_del(&world, LocObj::Placed(second.id), 5)
            .expect("second object is inactive");
        assert_eq!(
            overlay
                .zone_iter(&world, ZoneKey::of(at))
                .expect("iteration succeeds"),
            vec![LocObj::Placed(third.id), LocObj::Placed(first.id)]
        );
        overlay
            .loc_change(&world, LocObj::Placed(first.id), LocTypeId(20), 5)
            .expect("first object moves to the list end");
        assert_eq!(
            overlay
                .zone_iter(&world, ZoneKey::of(at))
                .expect("iteration sees changed order"),
            vec![LocObj::Placed(first.id), LocObj::Placed(third.id)]
        );
    }

    #[test]
    fn row_23_zone_stamp_changes_after_list_mutation() {
        let at = tile(10, 20);
        let first = placed(13, at, info(10, 0));
        let second = placed(14, at, info(11, 0));
        let world = Fixture::with_placed(&[first, second]);
        let mut overlay = WorldOverlay::default();
        let zone = ZoneKey::of(at);
        let _iterator_snapshot = overlay
            .zone_iter(&world, zone)
            .expect("initial iteration succeeds");
        let initial_stamp = overlay.stamp(zone);
        overlay
            .loc_del(&world, LocObj::Placed(first.id), 3)
            .expect("deletion changes the zone list");
        assert_ne!(overlay.stamp(zone), initial_stamp);
    }

    #[test]
    fn row_27_uncertain_expiry_marks_state_and_zone_unknown() {
        let world = Fixture::default();
        let at = tile(10, 20);
        let mut overlay = WorldOverlay::default();
        let obj = overlay
            .loc_add(&world, at, info(20, 10), 3)
            .expect("created object starts a life");
        overlay
            .age(&world, Ticks { min: 2, max: 2 })
            .expect("known prefix keeps the life active");
        overlay
            .age(&world, Ticks { min: 0, max: 1 })
            .expect("possible expiry records an unknown state");
        let read_error = overlay
            .read(&world, obj)
            .expect_err("object read fails closed");
        assert!(matches!(read_error, WorldError::UnknownLifeState { .. }));
        assert!(!read_error.is_engine_fault());
        let list_error = overlay
            .loc_find(&world, at, LocTypeId(20))
            .expect_err("zone list read fails closed");
        assert!(matches!(list_error, WorldError::UnknownZoneState { .. }));
        assert!(!list_error.is_engine_fault());
    }

    #[test]
    fn r377_zero_loc_duration_is_permanent_at_the_instance_threshold_only() {
        let world = Fixture::default();
        let mut overlay = WorldOverlay::with_profile(EngineProfile::R377);
        assert_eq!(
            WorldOverlay::default(),
            WorldOverlay::with_profile(EngineProfile::R289)
        );
        assert_ne!(
            WorldOverlay::default(),
            WorldOverlay::with_profile(EngineProfile::R377)
        );

        let instance_tile = tile(6464, 20);
        let permanent = overlay
            .loc_add(&world, instance_tile, info(1, 10), 0)
            .expect("zero-duration add is permanent in the instance");
        let permanent_index = overlay
            .object_index(permanent)
            .expect("created object is cached");
        assert_eq!(overlay.objects[permanent_index].record.left, None);
        overlay
            .loc_change(&world, permanent, LocTypeId(2), 0)
            .expect("zero-duration change is permanent in the instance");
        assert_eq!(overlay.objects[permanent_index].record.left, None);
        overlay
            .age(
                &world,
                Ticks {
                    min: u32::MAX,
                    max: u32::MAX,
                },
            )
            .expect("permanent object has no running life");
        assert_eq!(
            overlay
                .read(&world, permanent)
                .expect("permanent object remains active"),
            (info(2, 10), true)
        );

        let static_world = Fixture::with_placed(&[placed(20, instance_tile, info(10, 10))]);
        let static_object = LocObj::Placed(PlacedId(20));
        let mut static_overlay = WorldOverlay::with_profile(EngineProfile::R377);
        assert_eq!(
            static_overlay
                .loc_add(&static_world, instance_tile, info(11, 10), 0)
                .expect("zero-duration add changes an instance placement"),
            static_object
        );
        let static_index = static_overlay
            .object_index(static_object)
            .expect("placed object is cached");
        assert_eq!(static_overlay.objects[static_index].record.left, None);
        static_overlay
            .age(
                &static_world,
                Ticks {
                    min: u32::MAX,
                    max: u32::MAX,
                },
            )
            .expect("zero-duration placement does not expire");
        assert_eq!(
            static_overlay
                .read(&static_world, static_object)
                .expect("placement remains active"),
            (info(11, 10), true)
        );

        let outside_tile = tile(6463, 20);
        let invalid_add = overlay
            .loc_add(&world, outside_tile, info(3, 10), 0)
            .expect_err("zero-duration add is invalid outside the instance");
        assert!(matches!(
            invalid_add,
            WorldError::InvalidDuration { value: 0 }
        ));
        assert!(invalid_add.is_engine_fault());
        let outside = overlay
            .loc_add(&world, outside_tile, info(3, 10), 1)
            .expect("positive-duration add remains valid outside the instance");
        let invalid_change = overlay
            .loc_change(&world, outside, LocTypeId(4), 0)
            .expect_err("zero-duration change is invalid outside the instance");
        assert!(matches!(
            invalid_change,
            WorldError::InvalidDuration { value: 0 }
        ));
        assert!(invalid_change.is_engine_fault());
        assert_eq!(
            overlay
                .read(&world, outside)
                .expect("failed change leaves object intact"),
            (info(3, 10), true)
        );

        let invalid_delete = overlay
            .loc_del(&world, permanent, 0)
            .expect_err("loc deletion still requires a positive duration");
        assert!(matches!(
            invalid_delete,
            WorldError::InvalidDuration { value: 0 }
        ));
        assert!(invalid_delete.is_engine_fault());

        let positive = overlay
            .loc_add(&world, tile(6465, 20), info(5, 10), 2)
            .expect("positive duration still starts a life");
        let positive_index = overlay
            .object_index(positive)
            .expect("positive object is cached");
        assert_eq!(
            overlay.objects[positive_index].record.left,
            Some(Ticks { min: 2, max: 2 })
        );
        overlay
            .age(&world, Ticks { min: 2, max: 2 })
            .expect("positive duration expires");
        assert_eq!(
            overlay
                .read(&world, positive)
                .expect("expired object remains readable"),
            (info(5, 10), false)
        );
    }

    #[test]
    fn world_default_npc_spare_capacity_is_unknown_and_fixture_can_override() {
        assert_eq!(DefaultCapacityWorld.npc_spare_capacity(), Tri::Unknown);
        let world = Fixture {
            spare: Tri::Yes,
            ..Fixture::default()
        };
        assert_eq!(world.npc_spare_capacity(), Tri::Yes);
    }

    #[test]
    fn malformed_coordinates_shapes_angles_and_durations_are_typed_engine_faults() {
        let world = Fixture::default();
        let mut overlay = WorldOverlay::default();
        let invalid_coord = overlay
            .loc_add(&world, Coord(-1), info(1, 10), 1)
            .expect_err("negative packed coordinate fails");
        assert!(matches!(
            invalid_coord,
            WorldError::InvalidCoordinate { value: -1 }
        ));
        assert!(invalid_coord.is_engine_fault());
        let invalid_type = overlay
            .loc_add(&world, tile(1, 1), info(0x4000, 10), 1)
            .expect_err("type outside the encoded field fails");
        assert!(matches!(
            invalid_type,
            WorldError::InvalidLocType { value: 0x4000 }
        ));
        assert!(invalid_type.is_engine_fault());

        let invalid_shape = overlay
            .loc_add(&world, tile(1, 1), info(1, 23), 1)
            .expect_err("shape outside engine range fails");
        assert!(matches!(
            invalid_shape,
            WorldError::InvalidLocShape { value: 23 }
        ));
        assert!(invalid_shape.is_engine_fault());

        let invalid_angle = overlay
            .loc_add(
                &world,
                tile(1, 1),
                LocInfo {
                    angle: 4,
                    ..info(1, 10)
                },
                1,
            )
            .expect_err("angle outside engine range fails");
        assert!(matches!(
            invalid_angle,
            WorldError::InvalidLocAngle { value: 4 }
        ));
        assert!(invalid_angle.is_engine_fault());

        let invalid_duration = overlay
            .loc_add(&world, tile(1, 1), info(1, 10), 0)
            .expect_err("zero duration fails");
        assert!(matches!(
            invalid_duration,
            WorldError::InvalidDuration { value: 0 }
        ));
        assert!(invalid_duration.is_engine_fault());
    }

    #[test]
    fn owned_bytes_reports_every_overlay_vector_capacity() {
        let at = tile(10, 20);
        let world = Fixture::with_placed(&[placed(15, at, info(10, 0))]);
        let mut overlay = WorldOverlay::default();
        overlay
            .loc_del(&world, LocObj::Placed(PlacedId(15)), 3)
            .expect("mutation copies one zone");
        let expected = overlay.objects.capacity() * std::mem::size_of::<super::ObjectEntry>()
            + overlay.zones.capacity() * std::mem::size_of::<super::ZoneList>()
            + overlay
                .zones
                .iter()
                .map(|zone| zone.order.capacity() * std::mem::size_of::<LocObj>())
                .sum::<usize>();
        assert_eq!(overlay.owned_bytes(), expected);
        assert_eq!(overlay.clone(), overlay);
    }

    #[test]
    fn zone_key_uses_engine_eight_tile_zones() {
        assert_eq!(
            ZoneKey::of(Coord::new(15, 2, 31)),
            ZoneKey {
                x: 1,
                z: 3,
                level: 2
            }
        );
    }
}
