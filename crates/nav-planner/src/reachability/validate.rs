//! Invariants a reachability atlas keeps against the collision grid and the
//! transport graph it ships with. The baker checks its own atlas with
//! [`ReachabilityAtlas::validate`] before it publishes one, and both decoders
//! check the atlas they read. Every failure is a typed [`PackError`].

use api::query::loc_approach::{loc_reach_strategy, LocReachStrategy};

use super::{
    ClosureRun, ReachabilityAtlas, COVERAGE_EXCEPTIONAL_LANDING, COVERAGE_UNKNOWN_CONTENT,
    COVERAGE_UNMATCHED_JOIN, PLACEMENT_COMPLETE, PLACEMENT_UNKNOWN_CONTENT, SESSION_ESSENCE_EXIT,
    TRIGGER_CONTENT_STATE,
};
use crate::collision::WorldCollision;
use crate::pack::{unpack_tile_key, PackError};
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

fn check_csr_shape(
    offsets: &[u32],
    items: usize,
    rows: usize,
    what: &'static str,
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
            if run.count == 0 {
                return Err(bad("atlas closure run is empty"));
            }
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
        let known_reason = |reason: u32| {
            (COVERAGE_UNKNOWN_CONTENT..=COVERAGE_EXCEPTIONAL_LANDING).contains(&reason)
        };
        let mut previous = None;
        for row in self.coverage.iter() {
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
            } else if !known_detail(row.detail) {
                return Err(bad("atlas incomplete placement has no coverage detail"));
            }
            for stand in self.stands_for(row)? {
                let tile = unpack_tile_key(stand.tile)?;
                if stand.base as usize >= base_count || self.base_at(tile)? != Some(stand.base) {
                    return Err(bad("atlas stand component does not match the labels"));
                }
            }
        }
        Ok(())
    }
}

fn known_detail(detail: u32) -> bool {
    (COVERAGE_UNKNOWN_CONTENT..=COVERAGE_EXCEPTIONAL_LANDING).contains(&detail)
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
