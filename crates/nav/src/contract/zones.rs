//! Pack-bound zones, groups, bounded exemptions (`MAX_ZONE_KEYS=8`) and
//! wilderness membership/level. Does not expose table spatial storage,
//! unbound group/shape indices, `NO_SHAPE`, or wilderness grids.

use std::fmt;

use api::snapshot::WorldTile;

use crate::transport::WildernessRules;
use crate::zones::{ZoneKind, ZoneTable, NO_GROUP, NO_SHAPE};

use super::{ContractError, PackId, ZoneClass, ZoneExempt, ZoneKey};

/// A [`ZoneKey`] bound to the pack that minted it. Using it against another
/// pack is [`ContractError::PackMismatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BoundZoneKey {
    pack: PackId,
    key: ZoneKey,
}

impl BoundZoneKey {
    /// Originating pack.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Pack-local key.
    pub const fn key(self) -> ZoneKey {
        self.key
    }
}

/// Borrowed zone table of one pack.
#[derive(Clone, Copy)]
pub struct ZoneTableView<'a> {
    pack: PackId,
    table: &'a ZoneTable,
}

impl<'a> ZoneTableView<'a> {
    pub(super) fn new(pack: PackId, table: &'a ZoneTable) -> Self {
        Self { pack, table }
    }

    /// Originating pack.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Whether `key` names a row in this pack.
    pub fn contains_key(self, key: ZoneKey) -> bool {
        match key {
            ZoneKey::Zone(index) => self.table.zones().get(usize::from(index)).is_some(),
            ZoneKey::Group(index) => self.table.groups().get(usize::from(index)).is_some(),
        }
    }

    /// Bind `key` to this pack, or refuse unknown keys.
    pub fn bind(self, key: ZoneKey) -> Result<BoundZoneKey, ContractError> {
        if !self.contains_key(key) {
            return Err(ContractError::UnknownKey);
        }
        Ok(BoundZoneKey {
            pack: self.pack,
            key,
        })
    }

    /// Zone row for a bound key. Group keys yield [`ContractError::UnknownKey`]
    /// here; use [`Self::containing`] / [`Self::resolve`] for groups.
    pub fn zone(self, bound: BoundZoneKey) -> Result<ZoneView<'a>, ContractError> {
        if bound.pack != self.pack {
            return Err(ContractError::PackMismatch);
        }
        match bound.key {
            ZoneKey::Zone(index) if self.table.zones().get(usize::from(index)).is_some() => {
                Ok(ZoneView {
                    pack: self.pack,
                    table: self.table,
                    index,
                })
            }
            _ => Err(ContractError::UnknownKey),
        }
    }

    /// Kind facts for a bound zone key.
    pub fn kind(self, bound: BoundZoneKey) -> Result<ZoneKindView<'a>, ContractError> {
        Ok(self.zone(bound)?.kind())
    }

    /// Zones geometrically containing `tile` (inclusive plane-aware bounds,
    /// shapes and carves).
    pub fn containing(self, tile: WorldTile) -> impl Iterator<Item = ZoneView<'a>> + 'a {
        let pack = self.pack;
        let table = self.table;
        table
            .at(tile)
            .map(move |index| ZoneView { pack, table, index })
    }

    /// Resolve a catalog/group/hazard/spawn identity, or `None` (unknown).
    pub fn resolve(self, id: &str) -> Option<BoundZoneKey> {
        self.table.resolve(id).map(|key| BoundZoneKey {
            pack: self.pack,
            key,
        })
    }

    /// Canonical name for a bound key, or unknown refusal.
    pub fn name(self, bound: BoundZoneKey) -> Result<String, ContractError> {
        if bound.pack != self.pack {
            return Err(ContractError::PackMismatch);
        }
        if !self.contains_key(bound.key) {
            return Err(ContractError::UnknownKey);
        }
        Ok(self.table.name(bound.key))
    }

    /// Human-readable label for a bound key.
    pub fn label(self, bound: BoundZoneKey) -> Result<String, ContractError> {
        if bound.pack != self.pack {
            return Err(ContractError::PackMismatch);
        }
        if !self.contains_key(bound.key) {
            return Err(ContractError::UnknownKey);
        }
        Ok(self.table.label(bound.key))
    }

    /// Whether `exempt` covers this bound zone, checking pack identity.
    pub fn exemption_contains(
        self,
        exempt: ZoneExempt,
        bound: BoundZoneKey,
    ) -> Result<bool, ContractError> {
        if bound.pack != self.pack {
            return Err(ContractError::PackMismatch);
        }
        match bound.key {
            ZoneKey::Zone(index) if self.table.zones().get(usize::from(index)).is_some() => {
                Ok(exempt.contains_zone(index, self.table))
            }
            ZoneKey::Group(group) if self.table.groups().get(usize::from(group)).is_some() => {
                Ok(self.table.zones().iter().enumerate().any(|(i, zone)| {
                    zone.group == group
                        && exempt.contains_zone(u16::try_from(i).unwrap_or(u16::MAX), self.table)
                }))
            }
            _ => Err(ContractError::UnknownKey),
        }
    }
}

impl fmt::Debug for ZoneTableView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZoneTableView")
            .field("pack", &self.pack)
            .finish_non_exhaustive()
    }
}

/// One pack-bound zone row.
#[derive(Clone, Copy)]
pub struct ZoneView<'a> {
    pack: PackId,
    table: &'a ZoneTable,
    index: u16,
}

impl<'a> ZoneView<'a> {
    /// Bound key for this row (group if the zone is grouped).
    pub fn key(self) -> BoundZoneKey {
        BoundZoneKey {
            pack: self.pack,
            key: self.table.key(self.index),
        }
    }

    /// Inclusive bounds `(min_x, min_z, max_x, max_z)`.
    pub fn bounds(self) -> (i32, i32, i32, i32) {
        let zone = &self.table.zones()[usize::from(self.index)];
        (zone.min_x, zone.min_z, zone.max_x, zone.max_z)
    }

    /// Spawn tile xz.
    pub fn spawn(self) -> (i32, i32) {
        let zone = &self.table.zones()[usize::from(self.index)];
        (zone.spawn_x, zone.spawn_z)
    }

    /// Plane.
    pub fn plane(self) -> u8 {
        self.table.zones()[usize::from(self.index)].level
    }

    /// Always vs level-rule.
    pub fn class(self) -> ZoneClass {
        self.table.zones()[usize::from(self.index)].class
    }

    /// Combat cap (`u16::MAX` for Always).
    pub fn cap(self) -> u16 {
        self.table.zones()[usize::from(self.index)].cap
    }

    /// Whether the zone has a shape mask (replaces importing `NO_SHAPE`).
    pub fn has_shape(self) -> bool {
        self.table.zones()[usize::from(self.index)].shape != NO_SHAPE
    }

    /// Inclusive plane-aware membership, honouring shapes and carves.
    pub fn contains(self, tile: WorldTile) -> bool {
        self.table.at(tile).any(|index| index == self.index)
    }

    /// Kind facts.
    pub fn kind(self) -> ZoneKindView<'a> {
        let zone = &self.table.zones()[usize::from(self.index)];
        ZoneKindView {
            kind: &self.table.kinds()[usize::from(zone.kind)],
        }
    }

    /// Group identity, if any.
    pub fn group(self) -> Option<BoundZoneKey> {
        let zone = &self.table.zones()[usize::from(self.index)];
        if zone.group == NO_GROUP {
            None
        } else {
            Some(BoundZoneKey {
                pack: self.pack,
                key: ZoneKey::Group(zone.group),
            })
        }
    }
}

impl fmt::Debug for ZoneView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZoneView")
            .field("pack", &self.pack)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

/// Borrowed kind/hazard facts used by risk.
#[derive(Clone, Copy, Debug)]
pub struct ZoneKindView<'a> {
    kind: &'a ZoneKind,
}

impl<'a> ZoneKindView<'a> {
    /// Stable kind id.
    pub fn id(self) -> &'a str {
        self.kind.id.as_ref()
    }

    /// Display label.
    pub fn label(self) -> &'a str {
        self.kind.label.as_ref()
    }

    /// NPC id, or `-1` for a non-NPC hazard.
    pub fn npc_id(self) -> i32 {
        self.kind.npc_id
    }

    /// Visibility level.
    pub fn vislevel(self) -> u16 {
        self.kind.vislevel
    }

    /// Aggressive.
    pub fn aggressive(self) -> bool {
        self.kind.ap
    }

    /// Visibility off.
    pub fn vis_off(self) -> bool {
        self.kind.vis_off
    }

    /// Curated hazard damage, unknown when absent.
    pub fn hazard_damage(self) -> Option<u8> {
        crate::zones::curated::hazard_damage(self.kind.id.as_ref())
    }
}

/// Wilderness membership and level. No public Vec/grid/index fields.
#[derive(Clone, Copy, Debug)]
pub struct WildernessView<'a> {
    rules: &'a WildernessRules,
}

impl<'a> WildernessView<'a> {
    pub(super) fn new(rules: &'a WildernessRules) -> Self {
        Self { rules }
    }

    /// Inclusive `inzone` membership.
    pub fn contains(self, tile: WorldTile) -> bool {
        self.rules.contains(tile)
    }

    /// Content `~wilderness_level(coord)`: 0 outside every zone.
    pub fn level(self, tile: WorldTile) -> i32 {
        self.rules.level(tile)
    }
}
