//! Immutable packed reachability atlas. Construction lives in `nav-bake`.
//! Runtime code borrows compressed labels and tables; it never expands a
//! dense component array.

mod validate;

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;
use crate::pack::{pack_tile_key, unpack_tile_key, PackError, KEY_LEVELS};

/// Coverage reasons stored in the labels coverage table.
pub const COVERAGE_UNKNOWN_CONTENT: u32 = 1;
pub const COVERAGE_UNMATCHED_JOIN: u32 = 2;
pub const COVERAGE_UNSUPPORTED_GEOMETRY: u32 = 3;
pub const COVERAGE_UNSUPPORTED_TRIGGER: u32 = 4;
pub const COVERAGE_EXCEPTIONAL_LANDING: u32 = 5;

/// Placement coverage: complete, unsupported geometry, unsupported trigger,
/// or unknown content state.
pub const PLACEMENT_COMPLETE: u8 = 0;
pub const PLACEMENT_UNSUPPORTED_GEOMETRY: u8 = 1;
pub const PLACEMENT_UNSUPPORTED_TRIGGER: u8 = 2;
pub const PLACEMENT_UNKNOWN_CONTENT: u8 = 3;

/// Placement trigger: op, ap, or a content-state template.
pub const TRIGGER_OP: u8 = 0;
pub const TRIGGER_AP: u8 = 1;
pub const TRIGGER_CONTENT_STATE: u8 = 2;

/// Session template kind: essence-mine exit.
pub const SESSION_ESSENCE_EXIT: u32 = 1;

const CHUNK_SIDE: u32 = 64;
const CHUNK_CELLS: usize = 64 * 64;

/// Compressed atlas retained by [`crate::world::NavWorld`]. Not `Clone`.
#[derive(Debug)]
pub struct ReachabilityAtlas {
    pub(crate) geometry: AtlasGeometry,
    pub(crate) chunks: Box<[Chunk]>,
    pub(crate) palette: Box<[u32]>,
    pub(crate) indexes: Box<[u8]>,
    pub(crate) coverage: Box<[CoverageRow]>,
    pub(crate) exceptional: Box<[ExceptionalTransition]>,
    pub(crate) legacy_paint: Box<[CellRun]>,
    pub(crate) scatter: Box<[CellRun]>,
    pub(crate) base_to_condensed: Box<[u32]>,
    pub(crate) base_sizes: Box<[u32]>,
    pub(crate) movement_offsets: Box<[u32]>,
    pub(crate) movement_runs: Box<[ClosureRun]>,
    pub(crate) transport_offsets: Box<[u32]>,
    pub(crate) transport_runs: Box<[ClosureRun]>,
    pub(crate) conditional_offsets: Box<[u32]>,
    pub(crate) conditional_joins: Box<[ConditionalJoin]>,
    pub(crate) teleports: Box<[TeleportTemplate]>,
    pub(crate) sessions: Box<[SessionTemplate]>,
    pub(crate) placements: Box<[PlacementRow]>,
    pub(crate) stands: Box<[StandRow]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasGeometry {
    pub origin_x: i32,
    pub origin_z: i32,
    pub width: u32,
    pub height: u32,
    pub planes: u32,
    pub chunk_side: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Chunk {
    pub palette_start: u32,
    pub palette_count: u32,
    pub index_start: u32,
    pub index_len: u32,
    pub bit_width: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct CoverageRow {
    pub tile: u32,
    pub reason: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ExceptionalTransition {
    pub source: u32,
    pub destination: u32,
    pub edge: u32,
    pub reason: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct CellRun {
    pub first: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ClosureRun {
    pub first: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ConditionalJoin {
    pub destination: u32,
    pub edge: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct TeleportTemplate {
    pub teleport: u32,
    pub landing_base: u32,
    pub reason: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct SessionTemplate {
    pub kind: u32,
    pub source_base: u32,
    pub landing_base: u32,
    pub source_tile: u32,
    pub landing_tile: u32,
    pub selector: u32,
    pub reason: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct PlacementRow {
    pub origin: u32,
    pub loc_id: i32,
    pub shape: u8,
    pub angle: u8,
    pub coverage: u8,
    pub trigger: u8,
    pub stand_start: u32,
    pub stand_count: u32,
    pub detail: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct StandRow {
    pub tile: u32,
    pub base: u32,
}

impl ReachabilityAtlas {
    /// Assemble a baked atlas onto collision geometry.
    #[expect(
        clippy::too_many_arguments,
        reason = "The baker supplies every v18 table in one publication step."
    )]
    pub fn from_bake(
        collision: &WorldCollision,
        chunks: Box<[Chunk]>,
        palette: Box<[u32]>,
        indexes: Box<[u8]>,
        coverage: Box<[CoverageRow]>,
        exceptional: Box<[ExceptionalTransition]>,
        legacy_paint: Box<[CellRun]>,
        scatter: Box<[CellRun]>,
        base_to_condensed: Box<[u32]>,
        base_sizes: Box<[u32]>,
        movement_offsets: Box<[u32]>,
        movement_runs: Box<[ClosureRun]>,
        transport_offsets: Box<[u32]>,
        transport_runs: Box<[ClosureRun]>,
        conditional_offsets: Box<[u32]>,
        conditional_joins: Box<[ConditionalJoin]>,
        teleports: Box<[TeleportTemplate]>,
        sessions: Box<[SessionTemplate]>,
        placements: Box<[PlacementRow]>,
        stands: Box<[StandRow]>,
    ) -> Result<Self, PackError> {
        let mut atlas = Self::unlabeled(collision)?;
        atlas.chunks = chunks;
        atlas.palette = palette;
        atlas.indexes = indexes;
        atlas.coverage = coverage;
        atlas.exceptional = exceptional;
        atlas.legacy_paint = legacy_paint;
        atlas.scatter = scatter;
        atlas.base_to_condensed = base_to_condensed;
        atlas.base_sizes = base_sizes;
        atlas.movement_offsets = movement_offsets;
        atlas.movement_runs = movement_runs;
        atlas.transport_offsets = transport_offsets;
        atlas.transport_runs = transport_runs;
        atlas.conditional_offsets = conditional_offsets;
        atlas.conditional_joins = conditional_joins;
        atlas.teleports = teleports;
        atlas.sessions = sessions;
        atlas.placements = placements;
        atlas.stands = stands;
        Ok(atlas)
    }

    /// Atlas with matching geometry and no covered tiles, for synthetic worlds
    /// and tests that do not bake components.
    pub fn unlabeled(collision: &WorldCollision) -> Result<Self, PackError> {
        let width = u32::try_from(collision.width)
            .map_err(|_| PackError::BadLength("atlas width exceeds u32".into()))?;
        let height = u32::try_from(collision.height)
            .map_err(|_| PackError::BadLength("atlas height exceeds u32".into()))?;
        if width == 0 || height == 0 {
            return Err(PackError::BadLength("atlas geometry is empty".into()));
        }
        let chunk_cols = width.div_ceil(CHUNK_SIDE);
        let chunk_rows = height.div_ceil(CHUNK_SIDE);
        let n_chunks = (KEY_LEVELS as u32)
            .checked_mul(chunk_rows)
            .and_then(|n| n.checked_mul(chunk_cols))
            .ok_or_else(|| PackError::BadLength("atlas chunk count overflows".into()))?;
        let chunks = (0..n_chunks)
            .map(|_| Chunk {
                palette_start: 0,
                palette_count: 1,
                index_start: 0,
                index_len: 0,
                bit_width: 0,
            })
            .collect();
        Ok(Self {
            geometry: AtlasGeometry {
                origin_x: collision.origin.x,
                origin_z: collision.origin.z,
                width,
                height,
                planes: KEY_LEVELS as u32,
                chunk_side: CHUNK_SIDE,
            },
            chunks,
            palette: Box::from([0]),
            indexes: Box::from([]),
            coverage: Box::from([]),
            exceptional: Box::from([]),
            legacy_paint: Box::from([]),
            scatter: Box::from([]),
            base_to_condensed: Box::from([]),
            base_sizes: Box::from([]),
            movement_offsets: Box::from([0]),
            movement_runs: Box::from([]),
            transport_offsets: Box::from([0]),
            transport_runs: Box::from([]),
            conditional_offsets: Box::from([0]),
            conditional_joins: Box::from([]),
            teleports: Box::from([]),
            sessions: Box::from([]),
            placements: Box::from([]),
            stands: Box::from([]),
        })
    }

    /// Base component of `tile`, or `None` when the tile is uncovered.
    pub fn base_at(&self, tile: WorldTile) -> Result<Option<u32>, PackError> {
        if tile.level < 0 || tile.level >= KEY_LEVELS {
            return Ok(None);
        }
        let lx = tile.x.checked_sub(self.geometry.origin_x);
        let lz = tile.z.checked_sub(self.geometry.origin_z);
        let (Some(lx), Some(lz)) = (lx, lz) else {
            return Ok(None);
        };
        if lx < 0 || lz < 0 {
            return Ok(None);
        }
        let (lx, lz) = (lx as u32, lz as u32);
        if lx >= self.geometry.width || lz >= self.geometry.height {
            return Ok(None);
        }
        let chunk_cols = self.geometry.width.div_ceil(CHUNK_SIDE);
        let chunk_rows = self.geometry.height.div_ceil(CHUNK_SIDE);
        let cx = lx / CHUNK_SIDE;
        let cz = lz / CHUNK_SIDE;
        let local_x = lx % CHUNK_SIDE;
        let local_z = lz % CHUNK_SIDE;
        let chunk_index = (tile.level as u32)
            .checked_mul(chunk_rows)
            .and_then(|n| n.checked_mul(chunk_cols))
            .and_then(|n| n.checked_add(cz.checked_mul(chunk_cols)?))
            .and_then(|n| n.checked_add(cx))
            .ok_or_else(|| PackError::BadLength("atlas chunk index overflows".into()))?;
        let chunk = self
            .chunks
            .get(chunk_index as usize)
            .ok_or_else(|| PackError::BadLength("atlas chunk index is out of range".into()))?;
        let cell = local_z
            .checked_mul(CHUNK_SIDE)
            .and_then(|n| n.checked_add(local_x))
            .ok_or_else(|| PackError::BadLength("atlas cell index overflows".into()))?
            as usize;
        let pal_idx = if chunk.bit_width == 0 {
            0
        } else {
            extract_index(&self.indexes, chunk, cell)?
        };
        if pal_idx >= chunk.palette_count {
            return Err(PackError::BadLength(
                "atlas palette index is out of range".into(),
            ));
        }
        let start = chunk.palette_start as usize;
        let label = *self
            .palette
            .get(start + pal_idx as usize)
            .ok_or_else(|| PackError::BadLength("atlas palette start is out of range".into()))?;
        if label == 0 {
            return Ok(None);
        }
        Ok(Some(label - 1))
    }

    pub fn base_count(&self) -> u32 {
        u32::try_from(self.base_to_condensed.len()).unwrap_or(u32::MAX)
    }

    pub fn condensed_count(&self) -> u32 {
        self.base_to_condensed
            .iter()
            .copied()
            .max()
            .map_or(0, |id| id.saturating_add(1))
    }

    /// Condensed component that holds `base`, or `None` past the base count.
    pub fn condensed_of(&self, base: u32) -> Option<u32> {
        self.base_to_condensed.get(base as usize).copied()
    }

    /// Base components reachable from `base` by directed movement, itself
    /// included, in ascending order. `None` past the base count.
    pub fn movement_closure(&self, base: u32) -> Option<impl Iterator<Item = u32> + '_> {
        closure_ids(&self.movement_offsets, &self.movement_runs, base)
    }

    /// Condensed components reachable from `condensed` by movement and
    /// unconditional transport arcs, itself included, in ascending order.
    /// `None` past the condensed count.
    pub fn transport_closure(&self, condensed: u32) -> Option<impl Iterator<Item = u32> + '_> {
        closure_ids(&self.transport_offsets, &self.transport_runs, condensed)
    }

    /// Bytes the atlas tables hold in memory. The row strides equal the wire
    /// strides, so this tracks the serialized payload without the section
    /// directories. One atlas lives once per loaded pack, never per bot.
    pub fn resident_bytes(&self) -> usize {
        use std::mem::size_of_val;
        size_of_val(&*self.chunks)
            + size_of_val(&*self.palette)
            + size_of_val(&*self.indexes)
            + size_of_val(&*self.coverage)
            + size_of_val(&*self.exceptional)
            + size_of_val(&*self.legacy_paint)
            + size_of_val(&*self.scatter)
            + size_of_val(&*self.base_to_condensed)
            + size_of_val(&*self.base_sizes)
            + size_of_val(&*self.movement_offsets)
            + size_of_val(&*self.movement_runs)
            + size_of_val(&*self.transport_offsets)
            + size_of_val(&*self.transport_runs)
            + size_of_val(&*self.conditional_offsets)
            + size_of_val(&*self.conditional_joins)
            + size_of_val(&*self.teleports)
            + size_of_val(&*self.sessions)
            + size_of_val(&*self.placements)
            + size_of_val(&*self.stands)
    }

    /// Transport edges that never joined a labelled component.
    pub fn unmatched_join_count(&self) -> usize {
        self.coverage
            .iter()
            .filter(|row| row.reason == COVERAGE_UNMATCHED_JOIN)
            .count()
    }

    /// Transport edges with no admissible takeoff in a labelled component, as
    /// `(anchor, landing, local edge index)`. The landing names the goals the
    /// edge would have served: a negative proof for a goal near it is not
    /// complete.
    pub fn unmatched_joins(&self) -> impl Iterator<Item = (WorldTile, WorldTile, u32)> + '_ {
        self.exceptional
            .iter()
            .filter(|row| row.reason == COVERAGE_UNMATCHED_JOIN)
            .filter_map(|row| {
                Some((
                    unpack_tile_key(row.source).ok()?,
                    unpack_tile_key(row.destination).ok()?,
                    row.edge,
                ))
            })
    }

    /// Joined takeoffs whose landings sit off the labelled walk surface.
    pub fn exceptional_landing_count(&self) -> usize {
        self.exceptional
            .iter()
            .filter(|row| row.reason == COVERAGE_EXCEPTIONAL_LANDING)
            .count()
    }

    /// Teleport templates that never joined a labelled landing.
    pub fn unmatched_teleport_count(&self) -> usize {
        self.teleports
            .iter()
            .filter(|row| row.reason == COVERAGE_UNMATCHED_JOIN)
            .count()
    }

    /// Session templates that never joined a labelled source or landing.
    pub fn unmatched_session_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|row| row.reason == COVERAGE_UNMATCHED_JOIN)
            .count()
    }

    pub fn placement(
        &self,
        origin: WorldTile,
        loc_id: i32,
        shape: u8,
        angle: u8,
    ) -> Option<&PlacementRow> {
        let key = pack_tile_key(origin).ok()?;
        self.placements
            .binary_search_by(|row| {
                row.origin
                    .cmp(&key)
                    .then(row.loc_id.cmp(&loc_id))
                    .then(row.shape.cmp(&shape))
                    .then(row.angle.cmp(&angle))
            })
            .ok()
            .and_then(|i| self.placements.get(i))
    }

    pub fn stands_for(&self, row: &PlacementRow) -> Result<&[StandRow], PackError> {
        let start = row.stand_start as usize;
        let end = start
            .checked_add(row.stand_count as usize)
            .ok_or_else(|| PackError::BadLength("placement stand range overflows".into()))?;
        self.stands
            .get(start..end)
            .ok_or_else(|| PackError::BadLength("placement stand range is out of bounds".into()))
    }
}

/// Component ids named by one CSR row of closure runs.
fn closure_ids<'a>(
    offsets: &'a [u32],
    runs: &'a [ClosureRun],
    row: u32,
) -> Option<impl Iterator<Item = u32> + 'a> {
    let start = *offsets.get(row as usize)? as usize;
    let end = *offsets.get(row as usize + 1)? as usize;
    Some(
        runs.get(start..end)?
            .iter()
            .flat_map(|run| run.first..run.first.saturating_add(run.count)),
    )
}

fn extract_index(indexes: &[u8], chunk: &Chunk, cell: usize) -> Result<u32, PackError> {
    let width = usize::from(chunk.bit_width);
    let bit = cell
        .checked_mul(width)
        .ok_or_else(|| PackError::BadLength("atlas index bit offset overflows".into()))?;
    let mut value = 0u32;
    for b in 0..width {
        let at = bit + b;
        let byte = indexes
            .get(chunk.index_start as usize + at / 8)
            .copied()
            .ok_or_else(|| PackError::BadLength("atlas index payload is short".into()))?;
        if byte & (1 << (at % 8)) != 0 {
            value |= 1 << b;
        }
    }
    Ok(value)
}

/// Pack LSB-first indexes for one chunk.
pub fn pack_indexes(indexes: &[u32], bit_width: u8) -> Result<Vec<u8>, PackError> {
    if bit_width == 0 {
        return Ok(Vec::new());
    }
    let width = usize::from(bit_width);
    let bits = indexes
        .len()
        .checked_mul(width)
        .ok_or_else(|| PackError::BadLength("atlas index bit count overflows".into()))?;
    let mut out = vec![0u8; bits.div_ceil(8)];
    for (cell, &idx) in indexes.iter().enumerate() {
        if idx >= 1 << bit_width {
            return Err(PackError::BadLength(
                "atlas index does not fit the chunk bit width".into(),
            ));
        }
        let bit = cell * width;
        for b in 0..width {
            if idx & (1 << b) != 0 {
                let at = bit + b;
                out[at / 8] |= 1 << (at % 8);
            }
        }
    }
    Ok(out)
}

/// Minimum bit width for `palette_count` distinct labels.
pub fn palette_bit_width(palette_count: u32) -> Result<u8, PackError> {
    if palette_count <= 1 {
        return Ok(0);
    }
    Ok((u32::BITS - (palette_count - 1).leading_zeros()) as u8)
}

pub fn unpack_stand_tile(key: u32) -> Result<WorldTile, PackError> {
    unpack_tile_key(key)
}

pub const fn chunk_side() -> u32 {
    CHUNK_SIDE
}

pub const fn chunk_cells() -> usize {
    CHUNK_CELLS
}
