//! Content-derived navigation exclusion zones, their shared spatial index,
//! and the per-search exemption/activation filter.
//!
//! The wire table stores geometry and stable identities; the 8×8 bucket index,
//! presence bitset, and decode-time Wilderness overlap facts are rebuilt once
//! when a pack is loaded and shared with the `NavWorld`.

pub mod curated;

use std::collections::HashSet;
use std::fmt;
use std::mem::{align_of, size_of};
use std::slice;
use std::sync::Arc;

use api::snapshot::WorldTile;

use crate::router::AvoidRect;
use crate::transport::{WildernessRules, WildernessZone};

/// Sentinel for a zone which is not a member of a curated group.
pub const NO_GROUP: u16 = u16::MAX;
/// Sentinel for a zone represented by its complete bounding rectangle.
pub const NO_SHAPE: u16 = u16::MAX;
/// A single WalkTo may exempt at most this many named identities.
pub const MAX_ZONE_KEYS: usize = 8;
/// Frozen compat `avoidZones` catalog names, shared by script and host validation.
pub const AVOID_CATALOG_IDS: &[&str] = &["white-wolf-mountain", "draynor-jail-guards"];
const BUCKET_SIDE: i32 = 8;
const MAX_GRID_SIDE: u32 = 16_384;

/// Whether a zone always applies, or applies only below its combat threshold
/// outside the Wilderness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ZoneClass {
    Always = 0,
    LevelRule = 1,
}

/// Stable identity for one derived zone or a curated group of zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoneKey {
    Zone(u16),
    Group(u16),
}

/// Error returned when a request cannot be represented in the bounded key
/// set or the packed 15-bit index namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooMany;

impl fmt::Display for TooMany {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("too many or out-of-range zone keys")
    }
}

impl std::error::Error for TooMany {}

/// Per-walk opt-outs. Its compact, `Copy` representation is safe to carry in
/// stored route options; `all()` is a no-work bypass rather than a full mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneExempt {
    all: bool,
    len: u8,
    keys: [u16; MAX_ZONE_KEYS],
}

impl ZoneExempt {
    /// No zone identities are exempt.
    pub const NONE: Self = Self {
        all: false,
        len: 0,
        keys: [0; MAX_ZONE_KEYS],
    };

    /// Exempt every zone without constructing a table-sized mask.
    pub const fn all() -> Self {
        Self {
            all: true,
            len: 0,
            keys: [0; MAX_ZONE_KEYS],
        }
    }

    /// Exempt a bounded set of packed zone or group keys.
    pub fn named(keys: &[ZoneKey]) -> Result<Self, TooMany> {
        if keys.len() > MAX_ZONE_KEYS {
            return Err(TooMany);
        }
        let mut result = Self::NONE;
        for (slot, key) in keys.iter().copied().enumerate() {
            let index = match key {
                ZoneKey::Zone(index) if index < 0x8000 => index,
                ZoneKey::Group(index) if index < 0x8000 => index | 0x8000,
                _ => return Err(TooMany),
            };
            result.keys[slot] = index;
        }
        result.len = keys.len() as u8;
        Ok(result)
    }
    /// Combine request-local exemptions without allocating. `all` dominates;
    /// duplicate keys are removed in left-then-right request order.
    pub fn union(self, other: Self) -> Result<Self, TooMany> {
        if self.all || other.all {
            return Ok(Self::all());
        }
        let mut result = Self::NONE;
        for key in self.keys().iter().chain(other.keys()) {
            if result.keys[..usize::from(result.len)].contains(key) {
                continue;
            }
            let index = usize::from(result.len);
            if index == MAX_ZONE_KEYS {
                return Err(TooMany);
            }
            result.keys[index] = *key;
            result.len += 1;
        }
        Ok(result)
    }

    /// Whether this request bypasses all table work.
    pub const fn is_all(&self) -> bool {
        self.all
    }

    fn keys(&self) -> &[u16] {
        &self.keys[..usize::from(self.len)]
    }
}

impl Default for ZoneExempt {
    fn default() -> Self {
        Self::NONE
    }
}

/// One rectangular exclusion identity. A shape index, when present, stores a
/// row-major cell mask over the inclusive bounds; the bounds remain the
/// conservative index geometry. `spawn_x/z` preserve the actual spawn for
/// stable names even when a larger footprint makes the bounds asymmetric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone {
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
    pub spawn_x: i32,
    pub spawn_z: i32,
    pub level: u8,
    pub class: ZoneClass,
    /// `2 * vislevel` for `LevelRule`; `u16::MAX` for `Always`.
    pub cap: u16,
    /// Index into [`ZoneTable::kinds`].
    pub kind: u16,
    /// Index into [`ZoneTable::groups`], or [`NO_GROUP`].
    pub group: u16,
    /// Index into [`ZoneTable::shapes`], or [`NO_SHAPE`].
    pub shape: u16,
    /// Whether this zone's bounding rect intersects any packed Wilderness
    /// cell. This is conservative for shapes and carves.
    pub wild: bool,
}

impl Zone {
    /// Create a square NPC zone centered on its actual spawn tile.
    pub fn npc(spawn: WorldTile, radius: u8, class: ZoneClass, cap: u16, kind: u16) -> Self {
        let level = match spawn.level {
            0..=3 => spawn.level as u8,
            _ => u8::MAX,
        };
        let radius = i32::from(radius);
        Self {
            min_x: spawn.x.saturating_sub(radius),
            min_z: spawn.z.saturating_sub(radius),
            max_x: spawn.x.saturating_add(radius),
            max_z: spawn.z.saturating_add(radius),
            spawn_x: spawn.x,
            spawn_z: spawn.z,
            level,
            class,
            cap,
            kind,
            group: NO_GROUP,
            shape: NO_SHAPE,
            wild: false,
        }
    }

    /// Create a stationary melee NPC zone. `width`/`height` describe the
    /// footprint; the bounding rectangle is footprint ± one tile. The mask
    /// index is into the table's row-major `shapes` array.
    pub fn shaped_npc(
        spawn: WorldTile,
        width: u8,
        height: u8,
        class: ZoneClass,
        cap: u16,
        kind: u16,
        shape: u16,
    ) -> Self {
        let level = match spawn.level {
            0..=3 => spawn.level as u8,
            _ => u8::MAX,
        };
        Self {
            min_x: spawn.x.saturating_sub(1),
            min_z: spawn.z.saturating_sub(1),
            max_x: spawn.x.saturating_add(i32::from(width)),
            max_z: spawn.z.saturating_add(i32::from(height)),
            spawn_x: spawn.x,
            spawn_z: spawn.z,
            level,
            class,
            cap,
            kind,
            group: NO_GROUP,
            shape,
            wild: false,
        }
    }

    /// Create a curated rectangular hazard zone.
    pub fn hazard(rect: AvoidRect, level: u8, kind: u16) -> Self {
        Self {
            min_x: rect.min_x,
            min_z: rect.min_z,
            max_x: rect.max_x,
            max_z: rect.max_z,
            spawn_x: rect.min_x,
            spawn_z: rect.min_z,
            level,
            class: ZoneClass::Always,
            cap: u16::MAX,
            kind,
            group: NO_GROUP,
            shape: NO_SHAPE,
            wild: false,
        }
    }
}

/// Packed descriptive facts for an NPC kind or curated hazard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneKind {
    pub id: Arc<str>,
    pub label: Arc<str>,
    /// `-1` for a non-NPC hazard.
    pub npc_id: i32,
    pub vislevel: u16,
    pub ap: bool,
    pub vis_off: bool,
}

impl ZoneKind {
    pub fn new(
        id: impl Into<Arc<str>>,
        label: impl Into<Arc<str>>,
        npc_id: i32,
        vislevel: u16,
        ap: bool,
        vis_off: bool,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            npc_id,
            vislevel,
            ap,
            vis_off,
        }
    }
}

/// Curated grouping over selected derived-zone indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneGroup {
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub rect: AvoidRect,
    pub members: Box<[u16]>,
}

impl ZoneGroup {
    pub fn new(
        id: impl Into<Arc<str>>,
        label: impl Into<Arc<str>>,
        rect: AvoidRect,
        members: impl Into<Box<[u16]>>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            rect,
            members: members.into(),
        }
    }
}
/// Exact shared index capacities and occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneIndexMetrics {
    pub total_buckets: usize,
    pub occupied_buckets: usize,
    pub entries: usize,
    pub presence_bytes: usize,
    pub bucket_bytes: usize,
    pub entry_bytes: usize,
}

/// A validated, immutable zone table and its shared spatial index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneTable {
    pub(crate) zones: Box<[Zone]>,
    pub(crate) kinds: Box<[ZoneKind]>,
    pub(crate) groups: Box<[ZoneGroup]>,
    pub(crate) carves: Box<[(u16, AvoidRect)]>,
    pub(crate) shapes: Box<[u64]>,
    present: Box<[u64]>,
    buckets: Box<[(u32, u32)]>,
    entries: Box<[u16]>,
    origin: WorldTile,
    cols: u32,
    rows: u32,
}

/// Malformed or internally inconsistent zone-table input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneTableError(&'static str);

impl fmt::Display for ZoneTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ZoneTableError {}

impl ZoneTable {
    /// Validate the content-derived rows and build the 8×8 bucket index.
    /// `wild` on every row is recomputed from the supplied packed Wilderness
    /// rectangles and the row's bounding geometry. Shape indices are
    /// canonicalized to zone order so the v12 wire has a stable row order.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        mut zones: Vec<Zone>,
        kinds: Vec<ZoneKind>,
        groups: Vec<ZoneGroup>,
        mut carves: Vec<(u16, AvoidRect)>,
        mut shapes: Vec<u64>,
        origin: WorldTile,
        cols: u32,
        rows: u32,
        wilderness: &WildernessRules,
    ) -> Result<Self, ZoneTableError> {
        if origin.level != 0
            || cols == 0
            || rows == 0
            || cols > MAX_GRID_SIDE
            || rows > MAX_GRID_SIDE
            || i64::from(origin.x) + i64::from(cols) > i64::from(i32::MAX) + 1
            || i64::from(origin.z) + i64::from(rows) > i64::from(i32::MAX) + 1
        {
            return Err(ZoneTableError("invalid zone index bounds"));
        }
        validate_table_parts(&zones, &kinds, &groups, &carves, &shapes)?;
        let mut expected = 0usize;
        let canonical_shape_order =
            zones
                .iter()
                .filter(|zone| zone.shape != NO_SHAPE)
                .all(|zone| {
                    let canonical = usize::from(zone.shape) == expected;
                    expected += 1;
                    canonical
                })
                && expected == shapes.len();
        if !canonical_shape_order {
            let mut ordered = Vec::with_capacity(shapes.len());
            for zone in &mut zones {
                if zone.shape == NO_SHAPE {
                    continue;
                }
                let Some(bits) = shapes.get(usize::from(zone.shape)).copied() else {
                    return Err(ZoneTableError("zone shape index is out of range"));
                };
                let shape_index = u16::try_from(ordered.len())
                    .map_err(|_| ZoneTableError("zone shape count exceeds packed limit"))?;
                zone.shape = shape_index;
                ordered.push(bits);
            }
            if ordered.len() != shapes.len() {
                return Err(ZoneTableError("unreferenced zone shape"));
            }
            shapes = ordered;
        }

        for zone in &mut zones {
            zone.wild = rect_meets_wilderness(zone, &wilderness.zones);
        }
        carves.sort_unstable_by_key(|(zone, rect)| {
            (*zone, rect.min_z, rect.min_x, rect.max_z, rect.max_x)
        });
        let BucketIndex {
            present,
            buckets,
            entries,
        } = build_index(&zones, origin, cols, rows)?;
        Ok(Self {
            zones: zones.into_boxed_slice(),
            kinds: kinds.into_boxed_slice(),
            groups: groups.into_boxed_slice(),
            carves: carves.into_boxed_slice(),
            shapes: shapes.into_boxed_slice(),
            present: present.into_boxed_slice(),
            buckets: buckets.into_boxed_slice(),
            entries: entries.into_boxed_slice(),
            origin,
            cols,
            rows,
        })
    }

    /// Zone rows in deterministic pack order.
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    /// Packed kind descriptions.
    pub fn kinds(&self) -> &[ZoneKind] {
        &self.kinds
    }

    /// Curated group descriptions.
    pub fn groups(&self) -> &[ZoneGroup] {
        &self.groups
    }

    /// Subtracted rectangles, retained for pack round-trips.
    pub fn carves(&self) -> &[(u16, AvoidRect)] {
        &self.carves
    }

    /// Row-major cell masks, one u64 per shaped zone.
    pub fn shapes(&self) -> &[u64] {
        &self.shapes
    }

    /// Grid origin and dimensions used by the bucket index.
    pub fn bounds(&self) -> (WorldTile, u32, u32) {
        (self.origin, self.cols, self.rows)
    }

    /// Every shaped/carve-adjusted zone whose identity geometrically contains
    /// `tile`, regardless of combat activation or exemptions.
    pub fn at(&self, tile: WorldTile) -> ZoneAt<'_> {
        let candidates = self.bucket_entries(tile);
        ZoneAt {
            table: self,
            tile,
            candidates: candidates.iter(),
        }
    }

    /// Resolve a group id, hazard id, or exact NPC spawn identity.
    pub fn resolve(&self, id: &str) -> Option<ZoneKey> {
        if let Some(index) = self.groups.iter().position(|group| group.id.as_ref() == id) {
            return u16::try_from(index).ok().map(ZoneKey::Group);
        }
        let spawn = id.rsplit_once('@').and_then(|(kind, coordinates)| {
            let parse = |value: &str| {
                let digits = value.strip_prefix('-').unwrap_or(value);
                if digits.is_empty()
                    || (digits.starts_with('0') && (digits.len() != 1 || value.starts_with('-')))
                    || !digits.bytes().all(|digit| digit.is_ascii_digit())
                {
                    return None;
                }
                value.parse::<i32>().ok()
            };
            let mut parts = coordinates.split(',');
            let x = parse(parts.next()?)?;
            let z = parse(parts.next()?)?;
            let level = parse(parts.next()?)?;
            parts.next().is_none().then_some((kind, x, z, level))
        });
        self.zones.iter().enumerate().find_map(|(index, zone)| {
            let kind = self.kinds.get(usize::from(zone.kind))?;
            let matches = if kind.npc_id < 0 {
                kind.id.as_ref() == id
            } else {
                spawn.is_some_and(|(id, x, z, level)| {
                    kind.id.as_ref() == id
                        && zone.spawn_x == x
                        && zone.spawn_z == z
                        && i32::from(zone.level) == level
                })
            };
            matches
                .then(|| u16::try_from(index).ok())
                .flatten()
                .map(|index| {
                    if zone.group == NO_GROUP {
                        ZoneKey::Zone(index)
                    } else {
                        ZoneKey::Group(zone.group)
                    }
                })
        })
    }

    /// Canonical opt-out identity for a zone or group key.
    pub fn name(&self, key: ZoneKey) -> String {
        match key {
            ZoneKey::Group(index) => self
                .groups
                .get(usize::from(index))
                .map_or_else(String::new, |group| group.id.to_string()),
            ZoneKey::Zone(index) => {
                self.zones
                    .get(usize::from(index))
                    .map_or_else(String::new, |zone| {
                        let Some(kind) = self.kinds.get(usize::from(zone.kind)) else {
                            return String::new();
                        };
                        if kind.npc_id < 0 {
                            kind.id.to_string()
                        } else {
                            format!(
                                "{}@{},{},{}",
                                kind.id, zone.spawn_x, zone.spawn_z, zone.level
                            )
                        }
                    })
            }
        }
    }

    /// Human-readable zone or group label for refusal diagnostics.
    pub fn label(&self, key: ZoneKey) -> String {
        match key {
            ZoneKey::Group(index) => self
                .groups
                .get(usize::from(index))
                .map_or_else(String::new, |group| group.label.to_string()),
            ZoneKey::Zone(index) => {
                self.zones
                    .get(usize::from(index))
                    .map_or_else(String::new, |zone| {
                        let Some(kind) = self.kinds.get(usize::from(zone.kind)) else {
                            return String::new();
                        };
                        if kind.npc_id < 0 {
                            kind.label.to_string()
                        } else {
                            format!(
                                "{} at {},{} level {}",
                                kind.label, zone.spawn_x, zone.spawn_z, zone.level
                            )
                        }
                    })
            }
        }
    }

    /// The key that represents a zone in diagnostics. A grouped member maps
    /// to its group key; otherwise the key is the zone index.
    pub fn key(&self, index: u16) -> ZoneKey {
        self.zones
            .get(usize::from(index))
            .filter(|zone| zone.group != NO_GROUP)
            .map_or(ZoneKey::Zone(index), |zone| ZoneKey::Group(zone.group))
    }

    /// Exact payload bytes owned by this table: all resident fixed-size rows,
    /// boxed index/slice capacities, group-member indices, and Arc string
    /// allocations (reference-count headers and contents). Allocator
    /// bookkeeping and the stack-owned `ZoneTable` value itself are excluded.
    pub fn resident_heap_payload_bytes(&self) -> usize {
        let fixed = self.zones.len() * size_of::<Zone>()
            + self.kinds.len() * size_of::<ZoneKind>()
            + self.groups.len() * size_of::<ZoneGroup>()
            + self.carves.len() * size_of::<(u16, AvoidRect)>()
            + self.shapes.len() * size_of::<u64>()
            + self.present.len() * size_of::<u64>()
            + self.buckets.len() * size_of::<(u32, u32)>()
            + self.entries.len() * size_of::<u16>()
            + self
                .groups
                .iter()
                .map(|group| group.members.len() * size_of::<u16>())
                .sum::<usize>();
        let mut strings = HashSet::new();
        let mut string_bytes = 0usize;
        for text in self
            .kinds
            .iter()
            .flat_map(|kind| [&kind.id, &kind.label])
            .chain(
                self.groups
                    .iter()
                    .flat_map(|group| [&group.id, &group.label]),
            )
        {
            let allocation = Arc::as_ptr(text) as *const () as usize;
            if strings.insert(allocation) {
                let bytes = 2 * size_of::<usize>() + text.len();
                string_bytes += bytes.div_ceil(align_of::<usize>()) * align_of::<usize>();
            }
        }
        fixed + string_bytes
    }

    /// Exact shared index capacities and occupancy. `entries` counts
    /// `(bucket, zone)` references, not unique map tiles.
    pub fn index_metrics(&self) -> ZoneIndexMetrics {
        let bucket_cols = self.cols.div_ceil(BUCKET_SIDE as u32) as usize;
        let bucket_rows = self.rows.div_ceil(BUCKET_SIDE as u32) as usize;
        ZoneIndexMetrics {
            total_buckets: bucket_cols * bucket_rows * 4,
            occupied_buckets: self.buckets.len(),
            entries: self.entries.len(),
            presence_bytes: self.present.len() * size_of::<u64>(),
            bucket_bytes: self.buckets.len() * size_of::<(u32, u32)>(),
            entry_bytes: self.entries.len() * size_of::<u16>(),
        }
    }

    fn bucket_entries(&self, tile: WorldTile) -> &[u16] {
        let Some(key) = self.bucket_key(tile) else {
            return &[];
        };
        let word = usize::try_from(key / 64).expect("bucket word index fits usize");
        if self
            .present
            .get(word)
            .is_none_or(|bits| bits & (1 << (key & 63)) == 0)
        {
            return &[];
        }
        let Ok(position) = self
            .buckets
            .binary_search_by_key(&key, |(bucket, _)| *bucket)
        else {
            return &[];
        };
        let start = self.buckets[position].1 as usize;
        let end = self
            .buckets
            .get(position + 1)
            .map_or(self.entries.len(), |(_, start)| *start as usize);
        &self.entries[start..end]
    }

    fn bucket_key(&self, tile: WorldTile) -> Option<u32> {
        if !(0..4).contains(&tile.level) {
            return None;
        }
        let x = tile.x.checked_sub(self.origin.x)?;
        let z = tile.z.checked_sub(self.origin.z)?;
        if x < 0 || z < 0 || x as u32 >= self.cols || z as u32 >= self.rows {
            return None;
        }
        let bucket_cols = self.cols.div_ceil(BUCKET_SIDE as u32);
        let bucket_rows = self.rows.div_ceil(BUCKET_SIDE as u32);
        let per_level = bucket_cols.checked_mul(bucket_rows)?;
        let bx = x as u32 / BUCKET_SIDE as u32;
        let bz = z as u32 / BUCKET_SIDE as u32;
        (tile.level as u32)
            .checked_mul(per_level)?
            .checked_add(bz.checked_mul(bucket_cols)?)?
            .checked_add(bx)
    }

    fn zone_contains(&self, index: u16, tile: WorldTile) -> bool {
        let zone = &self.zones[usize::from(index)];
        if i32::from(zone.level) != tile.level
            || tile.x < zone.min_x
            || tile.x > zone.max_x
            || tile.z < zone.min_z
            || tile.z > zone.max_z
        {
            return false;
        }
        if zone.shape != NO_SHAPE {
            let width = (i64::from(zone.max_x) - i64::from(zone.min_x) + 1) as u32;
            let dx = (tile.x - zone.min_x) as u32;
            let dz = (tile.z - zone.min_z) as u32;
            let bit = dz * width + dx;
            if self.shapes[usize::from(zone.shape)] & (1u64 << bit) == 0 {
                return false;
            }
        }
        !self.carved(index, tile)
    }

    fn carved(&self, index: u16, tile: WorldTile) -> bool {
        let start = self.carves.partition_point(|(zone, _)| *zone < index);
        self.carves[start..]
            .iter()
            .take_while(|(zone, _)| *zone == index)
            .any(|(_, rect)| rect.contains(tile))
    }
}

/// Iterator over the zones geometrically containing one tile.
pub struct ZoneAt<'a> {
    table: &'a ZoneTable,
    tile: WorldTile,
    candidates: slice::Iter<'a, u16>,
}

impl Iterator for ZoneAt<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        self.candidates
            .by_ref()
            .copied()
            .find(|index| self.table.zone_contains(*index, self.tile))
    }
}

/// Per-search named exemptions and endpoint context for the original activity predicate.
pub struct ZoneFilter<'a> {
    table: &'a ZoneTable,
    combat: i32,
    mask: Box<[u64]>,
    origin: Option<WorldTile>,
    destination: Option<WorldTile>,
    all: bool,
}

impl<'a> ZoneFilter<'a> {
    /// Build named whole-walk exemptions and retain the selected endpoints.
    /// Origin permission is continuous and one-way. A destination's active
    /// zones permit entry and movement to the goal, but not exit for transit.
    pub fn new(
        table: &'a ZoneTable,
        combat_level: Option<i32>,
        endpoints: &[WorldTile],
        exempt: &ZoneExempt,
    ) -> Self {
        if exempt.is_all() {
            return Self {
                table,
                combat: combat_level.unwrap_or(0),
                mask: Box::new([]),
                origin: endpoints.first().copied(),
                destination: endpoints.get(1).copied(),
                all: true,
            };
        }
        let mut mask = vec![0u64; table.zones.len().div_ceil(64)];
        for packed in exempt.keys() {
            if packed & 0x8000 != 0 {
                let group = usize::from(packed & 0x7fff);
                if let Some(group) = table.groups.get(group) {
                    for zone in group.members.iter().copied() {
                        set_mask(&mut mask, zone);
                    }
                }
            } else {
                set_mask(&mut mask, *packed);
            }
        }
        Self {
            table,
            combat: combat_level.unwrap_or(0),
            mask: mask.into_boxed_slice(),
            origin: endpoints.first().copied(),
            destination: endpoints.get(1).copied(),
            all: false,
        }
    }

    pub(crate) fn select_destination(&mut self, goal: Option<WorldTile>) {
        self.destination = goal;
    }

    /// All matching zones active under the route predicate, applying named
    /// whole-walk exemptions only. Endpoint rules belong to transitions.
    /// Named exemptions apply throughout the route; endpoint exemptions
    /// require transition context and are not applied here.
    #[inline]
    pub fn blocking_at<'b>(
        &'b self,
        wilderness: &'b WildernessRules,
        tile: WorldTile,
    ) -> impl Iterator<Item = u16> + 'b {
        self.table
            .at(tile)
            .filter(move |&index| !self.whole_masked(index) && self.active(index, wilderness, tile))
    }

    /// Zones that block entering `tile` from `previous`, or leaving an active
    /// destination zone. Origin escape is continuous and cannot be reentered.
    pub(crate) fn blocking_transition_at<'b>(
        &'b self,
        wilderness: &'b WildernessRules,
        previous: WorldTile,
        tile: WorldTile,
        is_goal: bool,
    ) -> impl Iterator<Item = u16> + 'b {
        let entering = self.table.at(tile).filter(move |&index| {
            if self.whole_masked(index) || !self.active(index, wilderness, tile) {
                return false;
            }
            if self.origin_active(wilderness, index) {
                return !(self.table.zone_contains(index, previous)
                    && self.active(index, wilderness, previous));
            }
            !self.destination_active(wilderness, index) && !is_goal
        });
        let leaving = self.table.at(previous).filter(move |&index| {
            !self.whole_masked(index)
                && self.active(index, wilderness, previous)
                && self.destination_active(wilderness, index)
                && (!self.table.zone_contains(index, tile) || !self.active(index, wilderness, tile))
        });
        entering.chain(leaving)
    }

    /// A safe-pass goal reached only through its exact-tile permission is
    /// terminal. Completion passes may continue within their selected zone.
    pub(crate) fn destination_only_at(
        &self,
        wilderness: &WildernessRules,
        tile: WorldTile,
    ) -> bool {
        self.table.at(tile).any(|index| {
            !self.whole_masked(index)
                && self.active(index, wilderness, tile)
                && !self.origin_active(wilderness, index)
                && !self.destination_active(wilderness, index)
        })
    }

    pub(crate) fn origin_active(&self, wilderness: &WildernessRules, index: u16) -> bool {
        self.origin.is_some_and(|origin| {
            self.table.zone_contains(index, origin) && self.active(index, wilderness, origin)
        })
    }

    fn destination_active(&self, wilderness: &WildernessRules, index: u16) -> bool {
        self.destination.is_some_and(|goal| {
            self.table.zone_contains(index, goal) && self.active(index, wilderness, goal)
        })
    }

    /// Number of allocated 64-bit words in the named exemption mask.
    pub fn mask_words(&self) -> usize {
        self.mask.len()
    }

    #[inline]
    fn whole_masked(&self, zone: u16) -> bool {
        if self.all {
            return true;
        }
        let index = usize::from(zone);
        self.mask
            .get(index >> 6)
            .is_some_and(|word| word & (1u64 << (index & 63)) != 0)
    }

    #[inline]
    fn active(&self, index: u16, wilderness: &WildernessRules, tile: WorldTile) -> bool {
        let zone = &self.table.zones[usize::from(index)];
        zone.class == ZoneClass::Always
            || self.combat <= i32::from(zone.cap)
            || wilderness.contains(tile)
    }
}

fn set_mask(mask: &mut [u64], zone: u16) {
    let index = usize::from(zone);
    if let Some(word) = mask.get_mut(index >> 6) {
        *word |= 1u64 << (index & 63);
    }
}

fn validate_table_parts(
    zones: &[Zone],
    kinds: &[ZoneKind],
    groups: &[ZoneGroup],
    carves: &[(u16, AvoidRect)],
    shapes: &[u64],
) -> Result<(), ZoneTableError> {
    if zones.len() > 32_767
        || groups.len() > 32_767
        || kinds.len() > u16::MAX as usize
        || shapes.len() > u16::MAX as usize
    {
        return Err(ZoneTableError(
            "zone, group, kind, or shape count exceeds packed limit",
        ));
    }
    if kinds
        .iter()
        .any(|kind| kind.id.trim().is_empty() || kind.label.trim().is_empty() || kind.npc_id < -1)
    {
        return Err(ZoneTableError("malformed zone kind identity"));
    }
    let mut identities = HashSet::with_capacity(groups.len() + zones.len());
    for group in groups {
        if group.id.trim().is_empty()
            || group.label.trim().is_empty()
            || !identities.insert(group.id.to_string())
            || group.members.is_empty()
            || !valid_rect(
                group.rect.min_x,
                group.rect.min_z,
                group.rect.max_x,
                group.rect.max_z,
            )
            || !(-1..=3).contains(&group.rect.level.unwrap_or(-1))
        {
            return Err(ZoneTableError("malformed or empty zone group"));
        }
    }
    let mut kind_ids = HashSet::with_capacity(kinds.len());
    let mut npc_ids = HashSet::with_capacity(kinds.len());
    for kind in kinds {
        if !kind_ids.insert(kind.id.as_ref()) || (kind.npc_id >= 0 && !npc_ids.insert(kind.npc_id))
        {
            return Err(ZoneTableError("duplicate kind identity"));
        }
    }
    let mut seen_shapes = vec![false; shapes.len()];
    for zone in zones {
        if !valid_rect(zone.min_x, zone.min_z, zone.max_x, zone.max_z)
            || zone.level > 3
            || usize::from(zone.kind) >= kinds.len()
        {
            return Err(ZoneTableError("malformed zone geometry or kind index"));
        }
        if zone.class == ZoneClass::Always && zone.cap != u16::MAX {
            return Err(ZoneTableError("Always zone has a non-sentinel cap"));
        }
        let kind = &kinds[usize::from(zone.kind)];
        if zone.class == ZoneClass::LevelRule {
            let cap = kind
                .vislevel
                .checked_mul(2)
                .ok_or(ZoneTableError("zone kind combat cap overflows"))?;
            if zone.cap != cap {
                return Err(ZoneTableError("LevelRule zone cap differs from its kind"));
            }
        }
        if kind.npc_id >= 0 {
            let west = i64::from(zone.spawn_x) - i64::from(zone.min_x);
            let east = i64::from(zone.max_x) - i64::from(zone.spawn_x);
            let south = i64::from(zone.spawn_z) - i64::from(zone.min_z);
            let north = i64::from(zone.max_z) - i64::from(zone.spawn_z);
            if zone.shape == NO_SHAPE
                && (west != east
                    || west != south
                    || west != north
                    || !(0..=u8::MAX as i64).contains(&west))
            {
                return Err(ZoneTableError(
                    "unshaped NPC zone bounds do not match its radius",
                ));
            }
            if zone.shape != NO_SHAPE && (!(1..=6).contains(&east) || !(1..=6).contains(&north)) {
                return Err(ZoneTableError("shaped NPC extent exceeds pack bounds"));
            }
        }
        if kind.npc_id == -1 {
            if zone.class != ZoneClass::Always || zone.shape != NO_SHAPE || zone.group != NO_GROUP {
                return Err(ZoneTableError("malformed hazard zone"));
            }
            if !identities.insert(kind.id.to_string()) {
                return Err(ZoneTableError("duplicate zone identity"));
            }
        } else {
            let id = format!(
                "{}@{},{},{}",
                kind.id, zone.spawn_x, zone.spawn_z, zone.level
            );
            if kind.id.contains('@') || !identities.insert(id) {
                return Err(ZoneTableError("malformed or duplicate NPC zone identity"));
            }
        }
        if zone.group != NO_GROUP && usize::from(zone.group) >= groups.len() {
            return Err(ZoneTableError("zone group index is out of range"));
        }
        if zone.shape != NO_SHAPE {
            let shape_index = usize::from(zone.shape);
            if kind.npc_id < 0 || shape_index >= shapes.len() || seen_shapes[shape_index] {
                return Err(ZoneTableError(
                    "malformed, duplicate, or non-NPC shape index",
                ));
            }
            seen_shapes[shape_index] = true;
            let width = i64::from(zone.max_x) - i64::from(zone.min_x) + 1;
            let height = i64::from(zone.max_z) - i64::from(zone.min_z) + 1;
            if !(1..=8).contains(&width) || !(1..=8).contains(&height) {
                return Err(ZoneTableError("zone shape exceeds 8x8 bounds"));
            }
            if zone.min_x != zone.spawn_x.saturating_sub(1)
                || zone.min_z != zone.spawn_z.saturating_sub(1)
                || zone.max_x <= zone.spawn_x
                || zone.max_z <= zone.spawn_z
            {
                return Err(ZoneTableError(
                    "shape bounds do not preserve the spawn footprint",
                ));
            }
            let cells = (width * height) as u32;
            let bits = shapes[shape_index];
            if bits == 0 || (cells < 64 && bits >> cells != 0) {
                return Err(ZoneTableError("malformed zone shape bits"));
            }
        }
    }
    if seen_shapes.iter().any(|seen| !seen) {
        return Err(ZoneTableError("unreferenced zone shape"));
    }
    for (group_index, group) in groups.iter().enumerate() {
        let mut members = HashSet::with_capacity(group.members.len());
        for member in group.members.iter().copied() {
            let Some(zone) = zones.get(usize::from(member)) else {
                return Err(ZoneTableError("zone group member index is out of range"));
            };
            if !members.insert(member)
                || usize::from(zone.group) != group_index
                || zone.spawn_x < group.rect.min_x
                || zone.spawn_x > group.rect.max_x
                || zone.spawn_z < group.rect.min_z
                || zone.spawn_z > group.rect.max_z
                || group
                    .rect
                    .level
                    .is_some_and(|level| i32::from(zone.level) != level)
            {
                return Err(ZoneTableError("malformed zone group membership"));
            }
        }
    }
    for (index, zone) in zones.iter().enumerate() {
        if zone.group != NO_GROUP
            && !groups[usize::from(zone.group)]
                .members
                .contains(&(index as u16))
        {
            return Err(ZoneTableError("zone is missing from its group members"));
        }
    }
    for (zone, rect) in carves {
        let Some(owner) = zones.get(usize::from(*zone)) else {
            return Err(ZoneTableError("carve zone index is out of range"));
        };
        if !valid_rect(rect.min_x, rect.min_z, rect.max_x, rect.max_z)
            || rect.min_x < owner.min_x
            || rect.max_x > owner.max_x
            || rect.min_z < owner.min_z
            || rect.max_z > owner.max_z
            || rect
                .level
                .is_some_and(|level| level != i32::from(owner.level))
        {
            return Err(ZoneTableError("malformed zone carve"));
        }
    }
    Ok(())
}

fn valid_rect(min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
    min_x <= max_x && min_z <= max_z
}

fn rect_meets_wilderness(zone: &Zone, wilderness: &[WildernessZone]) -> bool {
    wilderness.iter().any(|wild| {
        wild.x1 <= wild.x2
            && wild.z1 <= wild.z2
            && wild.level1 <= wild.level2
            && i32::from(zone.level) >= wild.level1
            && i32::from(zone.level) <= wild.level2
            && zone.min_x <= wild.x2
            && zone.max_x >= wild.x1
            && zone.min_z <= wild.z2
            && zone.max_z >= wild.z1
    })
}

#[derive(Default)]
struct BucketIndex {
    present: Vec<u64>,
    buckets: Vec<(u32, u32)>,
    entries: Vec<u16>,
}

fn build_index(
    zones: &[Zone],
    origin: WorldTile,
    cols: u32,
    rows: u32,
) -> Result<BucketIndex, ZoneTableError> {
    let bucket_cols = cols.div_ceil(BUCKET_SIDE as u32);
    let bucket_rows = rows.div_ceil(BUCKET_SIDE as u32);
    let per_level = bucket_cols
        .checked_mul(bucket_rows)
        .ok_or(ZoneTableError("zone bucket count overflows"))?;
    let bucket_count = per_level
        .checked_mul(4)
        .ok_or(ZoneTableError("zone bucket count overflows"))?;
    if zones.is_empty() {
        return Ok(BucketIndex::default());
    }
    let mut present = vec![0u64; bucket_count.div_ceil(64) as usize];
    let mut pairs = Vec::new();
    for (index, zone) in zones.iter().enumerate() {
        let index = u16::try_from(index).map_err(|_| ZoneTableError("too many zones"))?;
        // Keep the full geometric identity, but index only routable cells.
        // Hunter reach can overhang the baked map's outer boundary.
        let min_x = i64::from(zone.min_x).max(i64::from(origin.x));
        let min_z = i64::from(zone.min_z).max(i64::from(origin.z));
        let max_x = i64::from(zone.max_x).min(i64::from(origin.x) + i64::from(cols) - 1);
        let max_z = i64::from(zone.max_z).min(i64::from(origin.z) + i64::from(rows) - 1);
        if min_x > max_x || min_z > max_z {
            continue;
        }
        let x0 = ((min_x - i64::from(origin.x)) as u32) / BUCKET_SIDE as u32;
        let x1 = ((max_x - i64::from(origin.x)) as u32) / BUCKET_SIDE as u32;
        let z0 = ((min_z - i64::from(origin.z)) as u32) / BUCKET_SIDE as u32;
        let z1 = ((max_z - i64::from(origin.z)) as u32) / BUCKET_SIDE as u32;
        let base = u32::from(zone.level) * per_level;
        for bz in z0..=z1 {
            for bx in x0..=x1 {
                let key = base + bz * bucket_cols + bx;
                pairs.push((key, index));
            }
        }
    }
    pairs.sort_unstable();
    let mut buckets = Vec::new();
    let mut entries = Vec::with_capacity(pairs.len());
    let mut previous = None;
    for (bucket, zone) in pairs {
        if previous != Some(bucket) {
            buckets.push((bucket, entries.len() as u32));
            previous = Some(bucket);
            present[(bucket / 64) as usize] |= 1u64 << (bucket & 63);
        }
        entries.push(zone);
    }
    Ok(BucketIndex {
        present,
        buckets,
        entries,
    })
}

#[cfg(test)]
#[path = "zones_tests.rs"]
mod tests;
