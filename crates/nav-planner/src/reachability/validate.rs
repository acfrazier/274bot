//! Invariants a reachability atlas keeps against the collision grid and the
//! transport graph it ships with. The baker checks its own atlas with
//! [`ReachabilityAtlas::validate`] before it publishes one, and both decoders
//! check the atlas they read. Every failure is a typed [`PackError`].
//!
//! One definition of each table invariant. The `check_*` functions below are
//! `pub(crate)`: the pack reader (`pack::reach`) calls the same functions
//! while it decodes each table that `validate` checks afterwards, so the
//! writer cannot accept a table the reader refuses. A new reader refusal goes
//! here, never into the reader alone.

use api::query::loc_approach::{loc_reach_strategy, LocReachStrategy};

use super::{
    chunk_cells, palette_bit_width, AtlasGeometry, CellRun, Chunk, ClosureRun, CoverageRow,
    PlacementRow, ReachabilityAtlas, StandRow, CHUNK_SIDE, COVERAGE_EXCEPTIONAL_LANDING,
    COVERAGE_UNKNOWN_CONTENT, COVERAGE_UNMATCHED_JOIN, PLACEMENT_COMPLETE,
    PLACEMENT_UNKNOWN_CONTENT, SESSION_ESSENCE_EXIT, TRIGGER_CONTENT_STATE,
};
use crate::collision::WorldCollision;
use crate::pack::{unpack_tile_key, PackError, KEY_LEVELS};
use crate::transport::TransportGraph;

fn bad(message: &str) -> PackError {
    PackError::BadLength(message.into())
}

/// The `row`-th slice of a CSR table, or a typed error when the offsets lie.
fn csr_row<'a, T>(offsets: &[u32], items: &'a [T], row: usize) -> Result<&'a [T], PackError> {
    let start = *offsets
        .get(row)
        .ok_or_else(|| bad("atlas CSR row is missing"))?;
    let end = *offsets
        .get(row + 1)
        .ok_or_else(|| bad("atlas CSR row is missing"))?;
    items
        .get(start as usize..end as usize)
        .ok_or_else(|| bad("atlas CSR row is out of bounds"))
}

/// CSR offsets: `rows + 1` of them, from zero, monotonic, ending at the item count.
pub(crate) fn check_csr_shape(
    offsets: &[u32],
    items: usize,
    rows: usize,
    what: &str,
) -> Result<(), PackError> {
    if offsets.len() != rows + 1
        || offsets.first() != Some(&0)
        || offsets.last().copied().map(|end| end as usize) != Some(items)
        || offsets.windows(2).any(|pair| pair[1] < pair[0])
    {
        return Err(PackError::BadLength(format!(
            "atlas {what} CSR offsets are not monotonic from zero to the table count"
        )));
    }
    Ok(())
}

/// Planes, chunk side and extent of the atlas geometry.
pub(crate) fn check_geometry_constants(g: &AtlasGeometry) -> Result<(), PackError> {
    if g.planes != KEY_LEVELS as u32 || g.chunk_side != CHUNK_SIDE {
        return Err(bad("atlas geometry planes must be 4 and chunk side 64"));
    }
    if g.width == 0 || g.height == 0 {
        return Err(bad("atlas geometry is empty"));
    }
    Ok(())
}

/// Planes, chunk side and extent, then exactly one directory row per plane and chunk.
pub(crate) fn check_geometry_shape(g: &AtlasGeometry, chunks: usize) -> Result<(), PackError> {
    check_geometry_constants(g)?;
    let side = u64::from(CHUNK_SIDE);
    let expected_chunks = KEY_LEVELS as u64
        * u64::from(g.height).div_ceil(side)
        * u64::from(g.width).div_ceil(side);
    if chunks as u64 != expected_chunks {
        return Err(bad("atlas chunk directory count does not match geometry"));
    }
    Ok(())
}

/// One chunk against the palette and index tables: minimum bit width, palette
/// range in bounds and sorted unique, index range in bounds, exact index
/// length, and no stray bits past the last cell.
pub(crate) fn check_chunk(chunk: &Chunk, palette: &[u32], indexes: &[u8]) -> Result<(), PackError> {
    let want = palette_bit_width(chunk.palette_count)?;
    if chunk.bit_width != want {
        return Err(bad("atlas chunk bit width is not the minimum for its palette"));
    }
    let pal_end = chunk
        .palette_start
        .checked_add(chunk.palette_count)
        .ok_or_else(|| bad("atlas palette range overflows"))?;
    if pal_end as usize > palette.len() {
        return Err(bad("atlas chunk palette range is out of bounds"));
    }
    if chunk.palette_count > 1 {
        let slice = &palette[chunk.palette_start as usize..pal_end as usize];
        if slice.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(bad("atlas chunk palette is not sorted unique"));
        }
    }
    let index_end = chunk
        .index_start
        .checked_add(chunk.index_len)
        .ok_or_else(|| bad("atlas index range overflows"))?;
    if index_end as usize > indexes.len() {
        return Err(bad("atlas chunk index range is out of bounds"));
    }
    if chunk.bit_width == 0 {
        if chunk.index_len != 0 {
            return Err(bad("uniform atlas chunk must have empty index payload"));
        }
    } else {
        let bits = chunk_cells()
            .checked_mul(usize::from(chunk.bit_width))
            .ok_or_else(|| bad("atlas index bits overflow"))?;
        if chunk.index_len as usize != bits.div_ceil(8) {
            return Err(bad("atlas chunk index length is not exact for its bit width"));
        }
        let unused = chunk.index_len as usize * 8 - bits;
        if unused > 0 {
            let last = indexes[index_end as usize - 1];
            let mask = !0u8 << (8 - unused);
            if last & mask != 0 {
                return Err(bad("atlas index payload has unused high bits set"));
            }
        }
    }
    Ok(())
}

/// Cell runs (legacy paint, scatter): nonempty, sorted and disjoint.
pub(crate) fn check_cell_runs(runs: &[CellRun]) -> Result<(), PackError> {
    let mut next = 0u64;
    for run in runs {
        if run.count == 0 {
            return Err(bad("atlas run count is zero"));
        }
        if u64::from(run.first) < next {
            return Err(bad("atlas runs must be sorted disjoint maximal"));
        }
        next = u64::from(run.first) + u64::from(run.count);
    }
    Ok(())
}

/// A closure run names at least one component.
pub(crate) fn check_closure_run(run: &ClosureRun) -> Result<(), PackError> {
    if run.count == 0 {
        return Err(bad("atlas closure run is empty"));
    }
    Ok(())
}

/// The component map and the size table describe the same base components.
pub(crate) fn check_base_tables(
    base_to_condensed: &[u32],
    base_sizes: &[u32],
) -> Result<(), PackError> {
    if base_to_condensed.len() != base_sizes.len() {
        return Err(bad("atlas base map and size tables differ in length"));
    }
    Ok(())
}

/// Coverage rows: valid tile keys, known reasons, sorted unique tile/reason pairs.
pub(crate) fn check_coverage_rows(rows: &[CoverageRow]) -> Result<(), PackError> {
    let mut previous = None;
    for row in rows {
        unpack_tile_key(row.tile)?;
        if !known_reason(row.reason) {
            return Err(bad("atlas coverage row has an unknown reason"));
        }
        if previous.is_some_and(|prev| (row.tile, row.reason) <= prev) {
            return Err(bad(
                "atlas coverage rows must be sorted unique tile/reason pairs",
            ));
        }
        previous = Some((row.tile, row.reason));
    }
    Ok(())
}

/// Placement rows: valid origin keys, angles 0..3, sorted unique by
/// `(plane, x, z, loc id, shape, angle)`.
pub(crate) fn check_placement_rows(rows: &[PlacementRow]) -> Result<(), PackError> {
    let mut last = None;
    for row in rows {
        let tile = unpack_tile_key(row.origin)?;
        if row.angle > 3 {
            return Err(bad("placement angle is outside 0..3"));
        }
        let key = (
            tile.level, tile.x, tile.z, row.loc_id, row.shape, row.angle,
        );
        if last.is_some_and(|prev| key <= prev) {
            return Err(bad("placements must be sorted unique"));
        }
        last = Some(key);
    }
    Ok(())
}

/// Stand rows: valid tile keys, each naming an existing base component.
pub(crate) fn check_stand_rows(rows: &[StandRow], base_count: usize) -> Result<(), PackError> {
    for row in rows {
        unpack_tile_key(row.tile)?;
        if row.base as usize >= base_count {
            return Err(bad("stand component is out of range"));
        }
    }
    Ok(())
}

/// The stands of one placement are sorted by packed tile and duplicate-free.
pub(crate) fn check_stand_order(stands: &[StandRow]) -> Result<(), PackError> {
    if stands.windows(2).any(|pair| pair[0].tile >= pair[1].tile) {
        return Err(bad("placement stands must be sorted unique tiles"));
    }
    Ok(())
}

/// Closure runs: nonempty rows of sorted, disjoint, maximal runs in bounds,
/// each row including its owner.
fn check_closures(
    offsets: &[u32],
    runs: &[ClosureRun],
    rows: usize,
    what: &'static str,
) -> Result<(), PackError> {
    check_csr_shape(offsets, runs.len(), rows, what)?;
    for owner in 0..rows {
        let row = csr_row(offsets, runs, owner)?;
        let mut previous_end: Option<u64> = None;
        let mut includes_owner = false;
        for run in row {
            check_closure_run(run)?;
            let first = u64::from(run.first);
            let end = first + u64::from(run.count);
            if end > rows as u64 {
                return Err(bad("atlas closure run is out of bounds"));
            }
            // A run that touches the previous one is not maximal.
            if previous_end.is_some_and(|prev| first <= prev) {
                return Err(bad(
                    "atlas closure runs are not sorted, disjoint and maximal",
                ));
            }
            previous_end = Some(end);
            includes_owner |= first <= owner as u64 && (owner as u64) < end;
        }
        if !includes_owner {
            return Err(bad("atlas closure does not include its own component"));
        }
    }
    Ok(())
}

impl ReachabilityAtlas {
    /// Check the atlas against the collision grid and graph it belongs to.
    ///
    /// An atlas with no components (synthetic worlds) passes the graph
    /// checks trivially. A labelled atlas must match the grid geometry, keep
    /// every table inside its bounds and order, and name only transport edges
    /// and teleports the graph has.
    pub fn validate(
        &self,
        collision: &WorldCollision,
        graph: &TransportGraph,
    ) -> Result<(), PackError> {
        self.check_geometry(collision)?;
        check_base_tables(&self.base_to_condensed, &self.base_sizes)?;
        let base_count = self.base_to_condensed.len();
        let condensed = self.condensed_count() as usize;
        self.check_labels(base_count)?;
        check_closures(
            &self.movement_offsets,
            &self.movement_runs,
            base_count,
            "movement",
        )?;
        check_closures(
            &self.transport_offsets,
            &self.transport_runs,
            condensed,
            "transport",
        )?;
        self.check_joins(graph, condensed)?;
        self.check_rows(graph, base_count)?;
        check_placement_rows(&self.placements)?;
        check_stand_rows(&self.stands, base_count)?;
        self.check_placements(base_count)
    }

    fn check_geometry(&self, collision: &WorldCollision) -> Result<(), PackError> {
        let g = &self.geometry;
        if g.origin_x != collision.origin.x
            || g.origin_z != collision.origin.z
            || u64::from(g.width) != collision.width as u64
            || u64::from(g.height) != collision.height as u64
        {
            return Err(bad(
                "atlas geometry does not match the collision grid it ships with",
            ));
        }
        check_geometry_shape(g, self.chunks.len())?;
        check_cell_runs(&self.legacy_paint)?;
        check_cell_runs(&self.scatter)?;
        let plane = u64::from(g.width) * u64::from(g.height);
        for run in self.legacy_paint.iter() {
            if u64::from(run.first) + u64::from(run.count) > plane * 4 {
                return Err(bad("atlas legacy paint run is outside the grid"));
            }
        }
        for run in self.scatter.iter() {
            if u64::from(run.first) + u64::from(run.count) > plane {
                return Err(bad("atlas scatter run is outside level zero"));
            }
        }
        Ok(())
    }

    fn check_labels(&self, base_count: usize) -> Result<(), PackError> {
        for chunk in self.chunks.iter() {
            check_chunk(chunk, &self.palette, &self.indexes)?;
        }
        if self
            .palette
            .iter()
            .any(|&label| label as usize > base_count)
        {
            return Err(bad("atlas palette names a component past the base count"));
        }
        Ok(())
    }

    fn check_joins(&self, graph: &TransportGraph, condensed: usize) -> Result<(), PackError> {
        check_csr_shape(
            &self.conditional_offsets,
            self.conditional_joins.len(),
            condensed,
            "conditional",
        )?;
        for source in 0..condensed {
            let row = csr_row(&self.conditional_offsets, &self.conditional_joins, source)?;
            for join in row {
                if join.destination as usize >= condensed {
                    return Err(bad("atlas conditional join names a missing component"));
                }
                if join.edge as usize >= graph.edges.len() {
                    return Err(bad("atlas conditional join names a missing transport edge"));
                }
            }
            if row.windows(2).any(|pair| {
                (pair[0].destination, pair[0].edge) >= (pair[1].destination, pair[1].edge)
            }) {
                return Err(bad("atlas conditional joins must be sorted unique"));
            }
        }
        Ok(())
    }

    fn check_rows(&self, graph: &TransportGraph, base_count: usize) -> Result<(), PackError> {
        check_coverage_rows(&self.coverage)?;
        for row in self.exceptional.iter() {
            unpack_tile_key(row.source)?;
            unpack_tile_key(row.destination)?;
            if row.edge as usize >= graph.edges.len() {
                return Err(bad("atlas exceptional transition names a missing edge"));
            }
            if row.reason != COVERAGE_UNMATCHED_JOIN && row.reason != COVERAGE_EXCEPTIONAL_LANDING {
                return Err(bad("atlas exceptional transition has an unknown reason"));
            }
        }
        let mut previous = None;
        for row in self.teleports.iter() {
            if row.teleport as usize >= graph.teleports.len() {
                return Err(bad("atlas teleport template names a missing teleport"));
            }
            if previous.is_some_and(|prev| row.teleport <= prev) {
                return Err(bad("atlas teleport templates must be sorted unique"));
            }
            previous = Some(row.teleport);
            match row.reason {
                0 if (row.landing_base as usize) < base_count => {}
                COVERAGE_UNMATCHED_JOIN => {}
                _ => return Err(bad("atlas teleport template has a bad landing or reason")),
            }
        }
        for row in self.sessions.iter() {
            unpack_tile_key(row.source_tile)?;
            unpack_tile_key(row.landing_tile)?;
            if row.kind != SESSION_ESSENCE_EXIT {
                return Err(bad("atlas session template has an unknown kind"));
            }
            match row.reason {
                0 if (row.source_base as usize) < base_count
                    && (row.landing_base as usize) < base_count => {}
                COVERAGE_UNMATCHED_JOIN => {}
                _ => return Err(bad("atlas session template has a bad component or reason")),
            }
        }
        Ok(())
    }

    fn check_placements(&self, base_count: usize) -> Result<(), PackError> {
        for row in self.placements.iter() {
            if row.coverage > PLACEMENT_UNKNOWN_CONTENT || row.trigger > TRIGGER_CONTENT_STATE {
                return Err(bad("atlas placement coverage or trigger is out of range"));
            }
            let unsupported = loc_reach_strategy(row.shape) == LocReachStrategy::Unsupported;
            if row.coverage == PLACEMENT_COMPLETE {
                if row.detail != 0 || unsupported {
                    return Err(bad(
                        "atlas complete placement has a coverage detail or an unsupported shape",
                    ));
                }
            } else if !known_reason(row.detail) {
                return Err(bad("atlas incomplete placement has no coverage detail"));
            }
            let stands = self.stands_for(row)?;
            check_stand_order(stands)?;
            for stand in stands {
                let tile = unpack_tile_key(stand.tile)?;
                if self.base_at(tile)? != Some(stand.base) {
                    return Err(bad("atlas stand component does not match the labels"));
                }
            }
        }
        Ok(())
    }
}

fn known_reason(reason: u32) -> bool {
    (COVERAGE_UNKNOWN_CONTENT..=COVERAGE_EXCEPTIONAL_LANDING).contains(&reason)
}

#[cfg(test)]
mod tests {
    use api::snapshot::WorldTile;

    use super::*;
    use crate::collision::pack_walk;
    use crate::reachability::{
        Chunk, ConditionalJoin, CoverageRow, PlacementRow, SessionTemplate, StandRow,
        TeleportTemplate, PLACEMENT_UNSUPPORTED_GEOMETRY, TRIGGER_OP,
    };

    fn collision(width: usize) -> WorldCollision {
        let (walk, blocked) = pack_walk(&vec![0; width * 4]);
        WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width,
            height: 1,
            walk,
            blocked,
            flags: None,
        }
    }

    /// Two tiles on one row: (3200,3200) is component 0, (3201,3200) is not
    /// covered. One placement has one stand on component 0.
    fn labelled() -> (WorldCollision, TransportGraph, ReachabilityAtlas) {
        let collision = collision(2);
        let key = crate::pack::pack_tile_key(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        })
        .unwrap();
        let mut atlas = ReachabilityAtlas::unlabeled(&collision).unwrap();
        atlas.palette = Box::from([0, 1]);
        atlas.chunks[0] = Chunk {
            palette_start: 0,
            palette_count: 2,
            index_start: 0,
            index_len: 512,
            bit_width: 1,
        };
        let mut indexes = vec![0u8; 512];
        indexes[0] = 1;
        atlas.indexes = indexes.into_boxed_slice();
        atlas.base_to_condensed = Box::from([0]);
        atlas.base_sizes = Box::from([1]);
        atlas.movement_offsets = Box::from([0, 1]);
        atlas.movement_runs = Box::from([ClosureRun { first: 0, count: 1 }]);
        atlas.transport_offsets = Box::from([0, 1]);
        atlas.transport_runs = Box::from([ClosureRun { first: 0, count: 1 }]);
        atlas.conditional_offsets = Box::from([0, 0]);
        atlas.placements = Box::from([PlacementRow {
            origin: key,
            loc_id: 42,
            shape: 0,
            angle: 0,
            coverage: PLACEMENT_COMPLETE,
            trigger: TRIGGER_OP,
            stand_start: 0,
            stand_count: 1,
            detail: 0,
        }]);
        atlas.stands = Box::from([StandRow { tile: key, base: 0 }]);
        (collision, TransportGraph::default(), atlas)
    }

    fn refused(mutate: impl FnOnce(&mut ReachabilityAtlas)) -> String {
        let (collision, graph, mut atlas) = labelled();
        mutate(&mut atlas);
        match atlas.validate(&collision, &graph) {
            Err(PackError::BadLength(message)) => message,
            other => panic!("expected a typed refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_consistent_atlas_passes() {
        let (collision, graph, atlas) = labelled();
        atlas.validate(&collision, &graph).unwrap();
    }

    #[test]
    fn an_atlas_for_another_grid_is_refused() {
        let (_, graph, atlas) = labelled();
        let wider = collision(3);
        assert!(matches!(
            atlas.validate(&wider, &graph),
            Err(PackError::BadLength(message)) if message.contains("geometry")
        ));
    }

    /// The reader wants one directory row per plane and chunk, so a baked
    /// atlas with none (or with too few) is a pack its own reader refuses.
    #[test]
    fn an_empty_or_short_chunk_directory_is_refused() {
        assert!(refused(|a| a.chunks = Box::from([])).contains("chunk directory"));
        let (_, _, whole) = labelled();
        let mut short = whole.chunks.to_vec();
        short.pop();
        assert!(refused(|a| a.chunks = short.into_boxed_slice()).contains("chunk directory"));
        let mut long = whole.chunks.to_vec();
        long.push(long[0]);
        assert!(refused(|a| a.chunks = long.into_boxed_slice()).contains("chunk directory"));
    }

    #[test]
    fn a_wrong_plane_count_or_chunk_side_is_refused() {
        assert!(refused(|a| a.geometry.planes = 3).contains("planes"));
        assert!(refused(|a| a.geometry.chunk_side = 32).contains("chunk side"));
    }

    #[test]
    fn labels_closures_and_placements_are_bounds_checked() {
        assert!(refused(|a| a.palette = Box::from([0, 2])).contains("palette"));
        assert!(
            refused(|a| a.movement_runs = Box::from([ClosureRun { first: 0, count: 2 }]))
                .contains("out of bounds")
        );
        assert!(
            refused(|a| a.transport_runs = Box::from([ClosureRun { first: 1, count: 0 }]))
                .contains("empty")
        );
        assert!(refused(|a| a.movement_offsets = Box::from([0, 0])).contains("CSR"));
        assert!(refused(|a| a.placements[0].coverage = 9).contains("coverage or trigger"));
        assert!(refused(|a| a.placements[0].trigger = 9).contains("coverage or trigger"));
        assert!(refused(|a| a.placements[0].detail = 3).contains("coverage detail"));
        assert!(refused(|a| a.placements[0].shape = 12).contains("unsupported shape"));
        assert!(refused(|a| {
            a.placements[0].coverage = PLACEMENT_UNSUPPORTED_GEOMETRY;
            a.placements[0].detail = 0;
        })
        .contains("no coverage detail"));
        assert!(refused(|a| a.stands[0].base = 1).contains("stand component"));
    }

    #[test]
    fn closure_runs_must_be_maximal_and_include_their_owner() {
        let two = |a: &mut ReachabilityAtlas| {
            a.base_to_condensed = Box::from([0, 1]);
            a.base_sizes = Box::from([1, 1]);
            a.movement_offsets = Box::from([0, 1, 2]);
            a.transport_offsets = Box::from([0, 1, 2]);
            a.transport_runs = Box::from([
                ClosureRun { first: 0, count: 1 },
                ClosureRun { first: 1, count: 1 },
            ]);
            a.conditional_offsets = Box::from([0, 0, 0]);
        };
        // Owner 1 lists only component 0.
        assert!(refused(|a| {
            two(a);
            a.movement_runs = Box::from([
                ClosureRun { first: 0, count: 1 },
                ClosureRun { first: 0, count: 1 },
            ]);
        })
        .contains("own component"));
        // Owner 0 lists two touching runs: not maximal.
        assert!(refused(|a| {
            two(a);
            a.movement_offsets = Box::from([0, 2, 3]);
            a.movement_runs = Box::from([
                ClosureRun { first: 0, count: 1 },
                ClosureRun { first: 1, count: 1 },
                ClosureRun { first: 1, count: 1 },
            ]);
        })
        .contains("maximal"));
    }

    #[test]
    fn joins_teleports_sessions_and_coverage_name_real_rows() {
        let key = crate::pack::pack_tile_key(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        })
        .unwrap();
        assert!(refused(|a| {
            a.conditional_joins = Box::from([ConditionalJoin {
                destination: 0,
                edge: 0,
            }]);
            a.conditional_offsets = Box::from([0, 1]);
        })
        .contains("missing transport edge"));
        assert!(refused(|a| {
            a.teleports = Box::from([TeleportTemplate {
                teleport: 0,
                landing_base: 0,
                reason: 0,
            }]);
        })
        .contains("missing teleport"));
        assert!(refused(|a| {
            a.sessions = Box::from([SessionTemplate {
                kind: 9,
                source_base: 0,
                landing_base: 0,
                source_tile: key,
                landing_tile: key,
                selector: 553,
                reason: 0,
            }]);
        })
        .contains("unknown kind"));
        assert!(refused(|a| {
            a.coverage = Box::from([
                CoverageRow {
                    tile: key,
                    reason: 3,
                },
                CoverageRow {
                    tile: key,
                    reason: 2,
                },
            ]);
        })
        .contains("sorted unique"));
        let (collision, graph, mut atlas) = labelled();
        atlas.coverage = Box::from([
            CoverageRow {
                tile: key,
                reason: 2,
            },
            CoverageRow {
                tile: key,
                reason: 5,
            },
        ]);
        atlas
            .validate(&collision, &graph)
            .expect("one tile may carry two reasons");
    }
}
