//! Release-time reachability atlas: movement components, closures, stands.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::time::Instant;

use api::obj_names::LocDefs;
use api::query::loc_approach::{
    loc_can_operate, loc_candidate_stands, loc_reach_strategy, LocOperateQuery, LocReachStrategy,
};
use api::snapshot::WorldTile;
use nav_planner::collision::{game_plane, WorldCollision};
use nav_planner::essence::{essence_session_for_wizard, ESSENCE_MINE_PORTALS};
use nav_planner::movement::step_ok;
use nav_planner::pack::{pack_tile_key, PackError};
use nav_planner::paint::{bake_reach, can_step_off};
use nav_planner::reachability::{
    chunk_cells, chunk_side, pack_indexes, palette_bit_width, CellRun, Chunk, ClosureRun,
    ConditionalJoin, CoverageRow, ExceptionalTransition, PlacementRow, ReachabilityAtlas,
    SessionTemplate, StandRow, TeleportTemplate, COVERAGE_EXCEPTIONAL_LANDING,
    COVERAGE_UNMATCHED_JOIN, COVERAGE_UNSUPPORTED_GEOMETRY, PLACEMENT_COMPLETE,
    PLACEMENT_UNSUPPORTED_GEOMETRY, SESSION_ESSENCE_EXIT, TRIGGER_OP,
};
use nav_planner::router::INTERACT_RADIUS;
use nav_planner::transport::{TransportEdge, TransportGraph};

use crate::pack::{parse_loc_fields, parse_map_line, section, SQUARE};
use crate::GenerationError;

const DIRS: [(i32, i32); 8] = [
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 0),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

type ClosureTables = (Box<[u32]>, Box<[ClosureRun]>);
type LabelTables = (Box<[Chunk]>, Box<[u32]>, Box<[u8]>);
type PlacementTables = (Box<[PlacementRow]>, Box<[StandRow]>);

/// Bake the complete v18 atlas for `collision` and `graph`.
pub fn bake_atlas(
    collision: &WorldCollision,
    graph: &TransportGraph,
    maps_dir: &Path,
    loc_defs: &LocDefs,
) -> Result<ReachabilityAtlas, GenerationError> {
    bake_atlas_inner(collision, graph, maps_dir, loc_defs)
        .map_err(|source| GenerationError::PackEncodeFailed { source })
}

fn bake_atlas_inner(
    collision: &WorldCollision,
    graph: &TransportGraph,
    maps_dir: &Path,
    loc_defs: &LocDefs,
) -> Result<ReachabilityAtlas, PackError> {
    let plane = collision
        .width
        .checked_mul(collision.height)
        .ok_or_else(|| PackError::BadLength("atlas plane size overflows".into()))?;
    let n = plane
        .checked_mul(4)
        .ok_or_else(|| PackError::BadLength("atlas cell count overflows".into()))?;
    let idx = |tile: WorldTile| -> Option<usize> {
        if !(0..4).contains(&tile.level) {
            return None;
        }
        let x = tile.x - collision.origin.x;
        let z = tile.z - collision.origin.z;
        if x < 0 || z < 0 {
            return None;
        }
        let (x, z) = (x as usize, z as usize);
        if x >= collision.width || z >= collision.height {
            return None;
        }
        Some(tile.level as usize * plane + z * collision.width + x)
    };
    let at = |i: usize| -> WorldTile {
        let level = (i / plane) as i32;
        let rem = i % plane;
        WorldTile {
            x: collision.origin.x + (rem % collision.width) as i32,
            z: collision.origin.z + (rem / collision.width) as i32,
            level,
        }
    };

    let started = Instant::now();
    eprintln!("nav-pack: atlas: scanning {n} cells");
    let mut dense = vec![u32::MAX; n];
    let mut cells = Vec::new();
    for (i, slot) in dense.iter_mut().enumerate() {
        if collision.standable(at(i)) {
            *slot = cells.len() as u32;
            cells.push(i);
        }
    }
    eprintln!(
        "nav-pack: atlas: {} walkable tiles in {:?}",
        cells.len(),
        started.elapsed()
    );
    let labels = movement_labels(collision, &cells, &dense, &at, idx)?;
    let base_count = labels.iter().copied().max().unwrap_or(0);
    eprintln!(
        "nav-pack: atlas: {base_count} movement components in {:?}",
        started.elapsed()
    );
    let mut base_sizes = vec![0u32; base_count as usize];
    for &label in &labels {
        if label > 0 {
            base_sizes[label as usize - 1] += 1;
        }
    }

    let mut movement_adj = vec![BTreeSet::<u32>::new(); base_count as usize];
    for &cell in &cells {
        let cur = at(cell);
        let Some(src) = label_of(&labels, cell) else {
            continue;
        };
        for d in DIRS {
            if !step_ok(collision, cur, d) {
                continue;
            }
            let next = WorldTile {
                x: cur.x + d.0,
                z: cur.z + d.1,
                level: cur.level,
            };
            let Some(ni) = idx(next) else {
                continue;
            };
            let Some(dst) = label_of(&labels, ni) else {
                continue;
            };
            if dst != src {
                movement_adj[src as usize].insert(dst);
            }
        }
    }
    eprintln!(
        "nav-pack: atlas: computing movement closures for {} components after {:?}",
        movement_adj.len(),
        started.elapsed()
    );
    let (movement_offsets, movement_runs) = closures_from_adj(&movement_adj)?;
    eprintln!(
        "nav-pack: atlas: movement closures done in {:?}",
        started.elapsed()
    );

    let mut coverage = Vec::new();
    let mut exceptional = Vec::new();
    // Condensed components condense movement and unconditional transport arcs
    // together (contract "Graph payload"): a one-way step between two movement
    // components is a directed arc of the condensed level too.
    let mut uncond_adj = movement_adj.clone();
    let mut conditional = Vec::<(u32, ConditionalJoin)>::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        let joined = join_edge(
            collision,
            graph,
            edge_index,
            edge,
            &labels,
            idx,
            &mut exceptional,
        )?;
        match joined {
            JoinResult::Missing => {
                // No admissible takeoff reaches a labelled component. Name the
                // edge's landing as well as its anchor, so a consumer can limit
                // "no complete negative" to the goals the landing serves.
                let landing = edge.landing_from(edge.at).unwrap_or(edge.to);
                exceptional.push(ExceptionalTransition {
                    source: pack_tile_key(edge.at)?,
                    destination: pack_tile_key(landing)?,
                    edge: edge_index as u32,
                    reason: COVERAGE_UNMATCHED_JOIN,
                });
                coverage.push(CoverageRow {
                    tile: pack_tile_key(edge.at)?,
                    reason: COVERAGE_UNMATCHED_JOIN,
                });
            }
            JoinResult::Pairs(pairs) => {
                for (src, dst) in pairs {
                    if is_unconditional(edge) {
                        uncond_adj[src as usize].insert(dst);
                    } else {
                        conditional.push((
                            src,
                            ConditionalJoin {
                                destination: dst,
                                edge: edge_index as u32,
                            },
                        ));
                    }
                }
            }
        }
    }
    coverage.sort_by_key(|row| (row.tile, row.reason));
    coverage.dedup_by_key(|row| (row.tile, row.reason));

    let condensed = scc_ids(&uncond_adj);
    let condensed_count = condensed.iter().copied().max().map_or(0, |id| id + 1);
    let mut condensed_adj = vec![BTreeSet::<u32>::new(); condensed_count as usize];
    for (src, dests) in uncond_adj.iter().enumerate() {
        let s = condensed[src];
        for &dst in dests {
            let d = condensed[dst as usize];
            if s != d {
                condensed_adj[s as usize].insert(d);
            }
        }
    }
    eprintln!(
        "nav-pack: atlas: computing transport closures for {} condensed components after {:?}",
        condensed_adj.len(),
        started.elapsed()
    );
    let (transport_offsets, transport_runs) = closures_from_adj(&condensed_adj)?;
    eprintln!(
        "nav-pack: atlas: transport closures done in {:?}",
        started.elapsed()
    );
    let mut cond_by_src = vec![Vec::<ConditionalJoin>::new(); condensed_count as usize];
    for (src_base, join) in conditional {
        let src = condensed[src_base as usize];
        cond_by_src[src as usize].push(ConditionalJoin {
            destination: condensed[join.destination as usize],
            edge: join.edge,
        });
    }
    let mut conditional_offsets = vec![0u32];
    let mut conditional_joins = Vec::new();
    for joins in &mut cond_by_src {
        joins.sort_by_key(|j| (j.destination, j.edge));
        joins.dedup_by_key(|j| (j.destination, j.edge));
        conditional_joins.extend_from_slice(joins);
        conditional_offsets.push(conditional_joins.len() as u32);
    }

    // Templates name base components (`landing_base`, `source_base`), as the
    // contract's tables do. Consumers map them through the base-to-condensed
    // table.
    let mut teleports = Vec::new();
    for (i, edge) in graph.teleports.iter().enumerate() {
        let landing = edge.landing_from(edge.at).unwrap_or(edge.to);
        let (landing_base, reason) = match idx(landing).and_then(|i| label_of(&labels, i)) {
            Some(base) => (base, 0),
            None => (0, COVERAGE_UNMATCHED_JOIN),
        };
        teleports.push(TeleportTemplate {
            teleport: i as u32,
            landing_base,
            reason,
        });
    }

    // Each essence-mine portal is a session source (see `portal_sources`).
    let mut sessions = Vec::new();
    for &portal in ESSENCE_MINE_PORTALS {
        let sources = portal_sources(portal, &labels, &idx);
        for npc in [553, 300, 462, 844, 171] {
            let Some(session) = essence_session_for_wizard(npc) else {
                continue;
            };
            let landing_base = idx(session.return_tile).and_then(|i| label_of(&labels, i));
            for &source_base in &sources {
                let reason = if source_base.is_none() || landing_base.is_none() {
                    COVERAGE_UNMATCHED_JOIN
                } else {
                    0
                };
                sessions.push(SessionTemplate {
                    kind: SESSION_ESSENCE_EXIT,
                    source_base: source_base.unwrap_or(0),
                    landing_base: landing_base.unwrap_or(0),
                    source_tile: pack_tile_key(portal)?,
                    landing_tile: pack_tile_key(session.return_tile)?,
                    selector: npc as u32,
                    reason,
                });
            }
        }
    }

    eprintln!(
        "nav-pack: atlas: baking placements after {:?}",
        started.elapsed()
    );
    let (placements, stands) = bake_placements(collision, maps_dir, loc_defs, &labels, idx)?;
    eprintln!(
        "nav-pack: atlas: {} placements, {} stands in {:?}",
        placements.len(),
        stands.len(),
        started.elapsed()
    );

    let reach_bits = bake_reach(collision, graph);
    let legacy_paint = bit_runs(&reach_bits, n);
    let scatter = scatter_runs(collision, &reach_bits, n, &at);

    let (chunks, palette, indexes) = encode_labels(collision, &labels)?;
    let atlas = ReachabilityAtlas::from_bake(
        collision,
        chunks,
        palette,
        indexes,
        coverage.into_boxed_slice(),
        exceptional.into_boxed_slice(),
        legacy_paint.into_boxed_slice(),
        scatter.into_boxed_slice(),
        condensed.into_boxed_slice(),
        base_sizes.into_boxed_slice(),
        movement_offsets,
        movement_runs,
        transport_offsets,
        transport_runs,
        conditional_offsets.into_boxed_slice(),
        conditional_joins.into_boxed_slice(),
        teleports.into_boxed_slice(),
        sessions.into_boxed_slice(),
        placements,
        stands,
    )?;
    // The reader refuses an atlas that breaks these invariants, so the baker
    // refuses to publish one.
    atlas.validate(collision, graph)?;
    Ok(atlas)
}

/// The base component of a dense cell: label 0 is "no component", label `n`
/// names component `n - 1`. Label 0 must not reach the subtraction: the
/// dev profile checks overflow, and the host-play build script bakes in it.
fn label_of(labels: &[u32], cell: usize) -> Option<u32> {
    labels.get(cell)?.checked_sub(1)
}

/// Components a portal can be operated from. The portal loc blocks its own
/// tile, so that tile has no label. The router relaxes the hop from every
/// standable tile within `INTERACT_RADIUS` of the portal
/// (`ReverseClosure::admit_takeoffs`), so the sources are the distinct
/// labelled components among those tiles. `[None]` means no neighbour is
/// labelled: the template is then an unmatched session, not a source of 0.
fn portal_sources(
    portal: WorldTile,
    labels: &[u32],
    idx: &impl Fn(WorldTile) -> Option<usize>,
) -> Vec<Option<u32>> {
    let mut sources = BTreeSet::new();
    for dx in -INTERACT_RADIUS..=INTERACT_RADIUS {
        for dz in -INTERACT_RADIUS..=INTERACT_RADIUS {
            let near = WorldTile {
                x: portal.x + dx,
                z: portal.z + dz,
                level: portal.level,
            };
            if let Some(base) = idx(near).and_then(|i| label_of(labels, i)) {
                sources.insert(base);
            }
        }
    }
    if sources.is_empty() {
        vec![None]
    } else {
        sources.into_iter().map(Some).collect()
    }
}

fn is_unconditional(edge: &TransportEdge) -> bool {
    edge.skill_req.is_empty()
        && edge.item_req.is_empty()
        && edge.consumed_req.is_empty()
        && edge.item_returns.is_empty()
        && edge.quest_req.is_empty()
        && edge.varp_req.is_empty()
        && edge.worn_req.is_empty()
        && edge.worn_all_req.is_empty()
        && edge.forbidden_held.is_empty()
        && edge.forbidden_worn.is_empty()
        && !edge.members_req
        && edge.wildy_cap.is_none()
        && edge.quest_gates.is_none()
}

enum JoinResult {
    Missing,
    Pairs(Vec<(u32, u32)>),
}

fn join_edge(
    collision: &WorldCollision,
    graph: &TransportGraph,
    edge_index: usize,
    edge: &TransportEdge,
    labels: &[u32],
    idx: impl Fn(WorldTile) -> Option<usize>,
    exceptional: &mut Vec<ExceptionalTransition>,
) -> Result<JoinResult, PackError> {
    let (min, max) = graph.takeoff_bounds(edge_index);
    let mut pairs = Vec::new();
    for x in min.x..=max.x {
        for z in min.z..=max.z {
            let from = WorldTile {
                x,
                z,
                level: min.level,
            };
            if !graph.admissible_from(collision, edge_index, from) {
                continue;
            }
            let Some(to) = edge.landing_from(from) else {
                continue;
            };
            let Some(fi) = idx(from) else {
                continue;
            };
            let Some(src) = label_of(labels, fi) else {
                continue;
            };
            match idx(to).and_then(|ti| label_of(labels, ti)) {
                Some(dst) => pairs.push((src, dst)),
                None => {
                    exceptional.push(ExceptionalTransition {
                        source: pack_tile_key(from)?,
                        destination: pack_tile_key(to)?,
                        edge: edge_index as u32,
                        reason: COVERAGE_EXCEPTIONAL_LANDING,
                    });
                }
            }
        }
    }
    if pairs.is_empty() && exceptional.iter().all(|row| row.edge != edge_index as u32) {
        Ok(JoinResult::Missing)
    } else {
        Ok(JoinResult::Pairs(pairs))
    }
}

fn movement_labels(
    collision: &WorldCollision,
    cells: &[usize],
    dense: &[u32],
    at: &dyn Fn(usize) -> WorldTile,
    idx: impl Fn(WorldTile) -> Option<usize>,
) -> Result<Vec<u32>, PackError> {
    let m = cells.len();
    let neighbor = |node: usize, reverse: bool| -> Vec<usize> {
        let cur = at(cells[node]);
        let mut out = Vec::new();
        for d in DIRS {
            // Reverse edges are incoming steps: the other tile is `cur + d`,
            // walking `-d` onto `cur`. Using `dest = from + step` here
            // collapses that other tile back to `cur` and makes every
            // standable tile its own component.
            let (from, step, other) = if reverse {
                let pred = WorldTile {
                    x: cur.x + d.0,
                    z: cur.z + d.1,
                    level: cur.level,
                };
                (pred, (-d.0, -d.1), pred)
            } else {
                let dest = WorldTile {
                    x: cur.x + d.0,
                    z: cur.z + d.1,
                    level: cur.level,
                };
                (cur, d, dest)
            };
            if !step_ok(collision, from, step) {
                continue;
            }
            let Some(di) = idx(other) else {
                continue;
            };
            let id = dense[di];
            if id != u32::MAX {
                out.push(id as usize);
            }
        }
        out
    };
    let mut seen = vec![false; m];
    let mut order = Vec::with_capacity(m);
    for seed in 0..m {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        let mut stack = vec![(seed, 0usize)];
        let mut adj = neighbor(seed, false);
        while let Some((node, next)) = stack.last_mut() {
            if *next < adj.len() {
                let n = adj[*next];
                *next += 1;
                if !seen[n] {
                    seen[n] = true;
                    stack.push((n, 0));
                    adj = neighbor(n, false);
                }
            } else {
                order.push(*node);
                stack.pop();
                if let Some((prev, _)) = stack.last() {
                    adj = neighbor(*prev, false);
                }
            }
        }
    }
    let mut scc = vec![u32::MAX; m];
    let mut component = 0u32;
    for seed in order.into_iter().rev() {
        if scc[seed] != u32::MAX {
            continue;
        }
        component = component
            .checked_add(1)
            .ok_or_else(|| PackError::BadLength("movement component count overflows".into()))?;
        let mut stack = vec![seed];
        scc[seed] = component;
        while let Some(node) = stack.pop() {
            for n in neighbor(node, true) {
                if scc[n] == u32::MAX {
                    scc[n] = component;
                    stack.push(n);
                }
            }
        }
    }
    let mut labels = vec![0u32; dense.len()];
    for (dense_id, &cell) in cells.iter().enumerate() {
        labels[cell] = scc[dense_id];
    }
    Ok(labels)
}

fn scc_ids(adj: &[BTreeSet<u32>]) -> Vec<u32> {
    let n = adj.len();
    let mut rev = vec![Vec::new(); n];
    for (src, dests) in adj.iter().enumerate() {
        for &dst in dests {
            rev[dst as usize].push(src);
        }
    }
    let mut seen = vec![false; n];
    let mut order = Vec::new();
    for seed in 0..n {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        let mut stack = vec![(
            seed,
            adj[seed].iter().map(|&d| d as usize).collect::<Vec<_>>(),
            0usize,
        )];
        while let Some((node, dests, next)) = stack.last_mut() {
            if *next < dests.len() {
                let n = dests[*next];
                *next += 1;
                if !seen[n] {
                    seen[n] = true;
                    let child = adj[n].iter().map(|&d| d as usize).collect::<Vec<_>>();
                    stack.push((n, child, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    let mut scc = vec![0u32; n];
    let mut id = 0u32;
    let mut assigned = vec![false; n];
    for seed in order.into_iter().rev() {
        if assigned[seed] {
            continue;
        }
        let mut stack = vec![seed];
        assigned[seed] = true;
        scc[seed] = id;
        while let Some(node) = stack.pop() {
            for &p in &rev[node] {
                if !assigned[p] {
                    assigned[p] = true;
                    scc[p] = id;
                    stack.push(p);
                }
            }
        }
        id += 1;
    }
    scc
}

/// n² bitsets are only for a small component DAG. Millions of isolated
/// stands would try to allocate terabytes.
const MAX_BITSET_BYTES: usize = 512 * 1024 * 1024;

fn closures_from_adj(adj: &[BTreeSet<u32>]) -> Result<ClosureTables, PackError> {
    closures_from_adj_within(adj, MAX_BITSET_BYTES)
}

/// Closures by bitset when the n² table fits `max_bitset_bytes`, else by one
/// bounded walk per component. Both forms produce the same tables.
fn closures_from_adj_within(
    adj: &[BTreeSet<u32>],
    max_bitset_bytes: usize,
) -> Result<ClosureTables, PackError> {
    let n = adj.len();
    if n == 0 {
        return Ok((Box::from([0u32]), Box::from([])));
    }
    let words = n.div_ceil(64);
    let cells = n
        .checked_mul(words)
        .ok_or_else(|| PackError::BadLength("atlas closure bitset overflows".into()))?;
    if cells
        .checked_mul(8)
        .is_none_or(|bytes| bytes > max_bitset_bytes)
    {
        return closures_from_adj_sparse(adj);
    }
    let mut reach = vec![0u64; cells];
    match dag_reverse_topo(adj) {
        Some(order) => {
            for u in order {
                let base = u * words;
                reach[base + u / 64] |= 1u64 << (u % 64);
                for &v in &adj[u] {
                    let vb = v as usize * words;
                    for w in 0..words {
                        reach[base + w] |= reach[vb + w];
                    }
                }
            }
        }
        None => {
            for seed in 0..n {
                let base = seed * words;
                let mut stack = vec![seed];
                reach[base + seed / 64] |= 1u64 << (seed % 64);
                while let Some(node) = stack.pop() {
                    for &next in &adj[node] {
                        let next = next as usize;
                        let word = &mut reach[base + next / 64];
                        let bit = 1u64 << (next % 64);
                        if *word & bit == 0 {
                            *word |= bit;
                            stack.push(next);
                        }
                    }
                }
            }
        }
    }
    bitsets_to_closure_runs(&reach, n, words)
}

fn closures_from_adj_sparse(adj: &[BTreeSet<u32>]) -> Result<ClosureTables, PackError> {
    let n = adj.len() as u32;
    let mut offsets = vec![0u32];
    let mut runs = Vec::new();
    for seed in 0..n {
        let mut reachable = BTreeSet::from([seed]);
        let mut stack = vec![seed];
        while let Some(node) = stack.pop() {
            for &next in &adj[node as usize] {
                if reachable.insert(next) {
                    stack.push(next);
                }
            }
        }
        let mut first = None;
        let mut count = 0u32;
        for id in reachable {
            match first {
                Some(start) if start + count == id => count += 1,
                Some(start) => {
                    runs.push(ClosureRun {
                        first: start,
                        count,
                    });
                    first = Some(id);
                    count = 1;
                }
                None => {
                    first = Some(id);
                    count = 1;
                }
            }
        }
        if let Some(start) = first {
            runs.push(ClosureRun {
                first: start,
                count,
            });
        }
        offsets.push(
            u32::try_from(runs.len())
                .map_err(|_| PackError::BadLength("atlas closure run count exceeds u32".into()))?,
        );
    }
    Ok((offsets.into_boxed_slice(), runs.into_boxed_slice()))
}

fn dag_reverse_topo(adj: &[BTreeSet<u32>]) -> Option<Vec<usize>> {
    let n = adj.len();
    let mut indeg = vec![0usize; n];
    for dests in adj {
        for &dst in dests {
            indeg[dst as usize] += 1;
        }
    }
    let mut ready: Vec<usize> = indeg
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| (d == 0).then_some(i))
        .collect();
    let mut topo = Vec::with_capacity(n);
    while let Some(u) = ready.pop() {
        topo.push(u);
        for &v in &adj[u] {
            let v = v as usize;
            indeg[v] -= 1;
            if indeg[v] == 0 {
                ready.push(v);
            }
        }
    }
    if topo.len() != n {
        return None;
    }
    topo.reverse();
    Some(topo)
}

fn bitsets_to_closure_runs(
    reach: &[u64],
    n: usize,
    words: usize,
) -> Result<ClosureTables, PackError> {
    let mut offsets = vec![0u32];
    let mut runs = Vec::new();
    for u in 0..n {
        let base = u * words;
        let mut first = None;
        let mut count = 0u32;
        for w in 0..words {
            let mut word = reach[base + w];
            while word != 0 {
                let bit = word.trailing_zeros();
                word &= word - 1;
                let id = (w * 64 + bit as usize) as u32;
                if id as usize >= n {
                    continue;
                }
                match first {
                    Some(start) if start + count == id => count += 1,
                    Some(start) => {
                        runs.push(ClosureRun {
                            first: start,
                            count,
                        });
                        first = Some(id);
                        count = 1;
                    }
                    None => {
                        first = Some(id);
                        count = 1;
                    }
                }
            }
        }
        if let Some(start) = first {
            runs.push(ClosureRun {
                first: start,
                count,
            });
        }
        offsets.push(
            u32::try_from(runs.len())
                .map_err(|_| PackError::BadLength("atlas closure run count exceeds u32".into()))?,
        );
    }
    Ok((offsets.into_boxed_slice(), runs.into_boxed_slice()))
}

fn encode_labels(collision: &WorldCollision, labels: &[u32]) -> Result<LabelTables, PackError> {
    let side = chunk_side() as usize;
    let chunk_cols = collision.width.div_ceil(side);
    let chunk_rows = collision.height.div_ceil(side);
    let plane = collision.width * collision.height;
    let mut chunks = Vec::new();
    let mut palette = Vec::new();
    let mut indexes = Vec::new();
    for level in 0..4 {
        for cz in 0..chunk_rows {
            for cx in 0..chunk_cols {
                let mut cells = vec![0u32; chunk_cells()];
                for lz in 0..side {
                    for lx in 0..side {
                        let x = cx * side + lx;
                        let z = cz * side + lz;
                        if x < collision.width && z < collision.height {
                            cells[lz * side + lx] = labels[level * plane + z * collision.width + x];
                        }
                    }
                }
                let unique: Vec<u32> = cells
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let bit_width = palette_bit_width(unique.len() as u32)?;
                let pal_start = u32::try_from(palette.len())
                    .map_err(|_| PackError::BadLength("atlas palette start exceeds u32".into()))?;
                palette.extend_from_slice(&unique);
                let mut pal_index = Vec::with_capacity(cells.len());
                for &label in &cells {
                    let i = unique.binary_search(&label).unwrap_or(0) as u32;
                    pal_index.push(i);
                }
                let packed = pack_indexes(&pal_index, bit_width)?;
                let index_start = u32::try_from(indexes.len())
                    .map_err(|_| PackError::BadLength("atlas index start exceeds u32".into()))?;
                let index_len = u32::try_from(packed.len())
                    .map_err(|_| PackError::BadLength("atlas index length exceeds u32".into()))?;
                indexes.extend_from_slice(&packed);
                chunks.push(Chunk {
                    palette_start: pal_start,
                    palette_count: unique.len() as u32,
                    index_start,
                    index_len,
                    bit_width,
                });
            }
        }
    }
    Ok((
        chunks.into_boxed_slice(),
        palette.into_boxed_slice(),
        indexes.into_boxed_slice(),
    ))
}

fn bake_placements(
    collision: &WorldCollision,
    maps_dir: &Path,
    loc_defs: &LocDefs,
    labels: &[u32],
    idx: impl Fn(WorldTile) -> Option<usize>,
) -> Result<PlacementTables, PackError> {
    let mut files: Vec<_> = std::fs::read_dir(maps_dir)
        .map_err(PackError::Io)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jm2"))
        .collect();
    files.sort();
    let mut placements = Vec::new();
    let mut stands = Vec::new();
    for path in files {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| PackError::BadLength("mapsquare name is not utf-8".into()))?;
        let Some((sx, sz)) = name.strip_prefix('m').and_then(|rest| rest.split_once('_')) else {
            continue;
        };
        let sx: i32 = sx
            .parse()
            .map_err(|_| PackError::BadLength("mapsquare x is not an integer".into()))?;
        let sz: i32 = sz
            .parse()
            .map_err(|_| PackError::BadLength("mapsquare z is not an integer".into()))?;
        let text = std::fs::read_to_string(&path).map_err(PackError::Io)?;
        let mut in_map = false;
        let mut link_below = HashSet::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = section(line) {
                in_map = name == "MAP";
                continue;
            }
            if in_map {
                if let Some((level, x, z, flags)) = parse_map_line(line) {
                    if level == 1 && flags & 2 != 0 {
                        link_below.insert((x, z));
                    }
                }
            }
        }
        let mut in_loc = false;
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = section(line) {
                in_loc = name == "LOC";
                continue;
            }
            if !in_loc {
                continue;
            }
            let Some(loc) = parse_loc_fields(line) else {
                continue;
            };
            let Some(true_level) = game_plane(loc.level, link_below.contains(&(loc.x, loc.z)))
            else {
                continue;
            };
            let origin = WorldTile {
                x: sx * SQUARE as i32 + loc.x as i32,
                z: sz * SQUARE as i32 + loc.z as i32,
                level: true_level,
            };
            let Some(def) = loc_defs.loc(loc.loc_id) else {
                continue;
            };
            if def.ops.is_empty() {
                continue;
            }
            let shape = u8::try_from(loc.shape).unwrap_or(255);
            let angle = u8::try_from(loc.angle).unwrap_or(255);
            if angle > 3 {
                continue;
            }
            let strategy = loc_reach_strategy(shape);
            let stand_start = stands.len() as u32;
            let mut coverage = PLACEMENT_COMPLETE;
            if strategy == LocReachStrategy::Unsupported {
                coverage = PLACEMENT_UNSUPPORTED_GEOMETRY;
            } else if let Some(candidates) =
                loc_candidate_stands(origin, shape, angle, def.width, def.length)
            {
                let mut local = Vec::new();
                for from in candidates {
                    if !collision.standable(from) {
                        continue;
                    }
                    let flags = collision.walkable_word(from.x, from.z, from.level) as i32;
                    match loc_can_operate(LocOperateQuery {
                        origin,
                        from,
                        shape,
                        angle,
                        width: def.width,
                        length: def.length,
                        force_approach: def.force_approach,
                        flags,
                    }) {
                        None => coverage = PLACEMENT_UNSUPPORTED_GEOMETRY,
                        Some(false) => {}
                        Some(true) => {
                            if let Some(base) = idx(from).and_then(|i| label_of(labels, i)) {
                                local.push(StandRow {
                                    tile: pack_tile_key(from)?,
                                    base,
                                });
                            }
                        }
                    }
                }
                local.sort_by_key(|row| row.tile);
                local.dedup_by_key(|row| row.tile);
                stands.extend(local);
            }
            let detail = if coverage == PLACEMENT_UNSUPPORTED_GEOMETRY {
                COVERAGE_UNSUPPORTED_GEOMETRY
            } else {
                0
            };
            placements.push(PlacementRow {
                origin: pack_tile_key(origin)?,
                loc_id: loc.loc_id,
                shape,
                angle,
                coverage,
                trigger: TRIGGER_OP,
                stand_start,
                stand_count: stands.len() as u32 - stand_start,
                detail,
            });
        }
    }
    placements.sort_by(|a, b| {
        let ta = nav_planner::pack::unpack_tile_key(a.origin).unwrap_or(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        });
        let tb = nav_planner::pack::unpack_tile_key(b.origin).unwrap_or(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        });
        (ta.level, ta.x, ta.z, a.loc_id, a.shape, a.angle)
            .cmp(&(tb.level, tb.x, tb.z, b.loc_id, b.shape, b.angle))
    });
    Ok((placements.into_boxed_slice(), stands.into_boxed_slice()))
}

fn bit_runs(bits: &[u64], n: usize) -> Vec<CellRun> {
    let mut runs = Vec::new();
    let mut i = 0usize;
    while i < n {
        let word = bits.get(i / 64).copied().unwrap_or(0);
        if (word >> (i % 64)) & 1 == 0 {
            i += 1;
            continue;
        }
        let first = i as u32;
        i += 1;
        while i < n {
            let word = bits.get(i / 64).copied().unwrap_or(0);
            if (word >> (i % 64)) & 1 == 0 {
                break;
            }
            i += 1;
        }
        runs.push(CellRun {
            first,
            count: i as u32 - first,
        });
    }
    runs
}

fn scatter_runs(
    collision: &WorldCollision,
    bits: &[u64],
    n: usize,
    at: &dyn Fn(usize) -> WorldTile,
) -> Vec<CellRun> {
    let plane = collision.width * collision.height;
    let mut marked = vec![false; n];
    for (i, slot) in marked.iter_mut().enumerate().take(plane.min(n)) {
        let tile = at(i);
        if !collision.standable(tile) {
            continue;
        }
        let word = bits.get(i / 64).copied().unwrap_or(0);
        if (word >> (i % 64)) & 1 == 0 {
            continue;
        }
        if can_step_off(collision, tile) {
            *slot = true;
        }
    }
    let words: Vec<u64> = marked
        .chunks(64)
        .map(|chunk| {
            let mut word = 0u64;
            for (b, &on) in chunk.iter().enumerate() {
                if on {
                    word |= 1 << b;
                }
            }
            word
        })
        .collect();
    bit_runs(&words, plane)
}

#[cfg(test)]
mod closure_tests {
    use super::*;

    fn runs_for(adj: Vec<BTreeSet<u32>>) -> Vec<Vec<(u32, u32)>> {
        let (offsets, runs) = closures_from_adj(&adj).unwrap();
        (0..adj.len())
            .map(|i| {
                let start = offsets[i] as usize;
                let end = offsets[i + 1] as usize;
                runs[start..end]
                    .iter()
                    .map(|run| (run.first, run.count))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn empty_graph_has_no_closure_runs() {
        let (offsets, runs) = closures_from_adj(&[]).unwrap();
        assert_eq!(&*offsets, &[0]);
        assert!(runs.is_empty());
    }

    #[test]
    fn chain_closure_covers_self_and_descendants() {
        let mut adj = vec![BTreeSet::new(); 3];
        adj[0].insert(1);
        adj[1].insert(2);
        assert_eq!(
            runs_for(adj),
            vec![vec![(0, 3)], vec![(1, 2)], vec![(2, 1)]]
        );
    }

    #[test]
    fn disconnected_ids_encode_separate_runs() {
        let mut adj = vec![BTreeSet::new(); 3];
        adj[0].insert(2);
        assert_eq!(
            runs_for(adj),
            vec![vec![(0, 1), (2, 1)], vec![(1, 1)], vec![(2, 1)]]
        );
    }

    use nav_planner::collision::pack_walk;
    use nav_planner::transport::TransportKind;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const BLOCKED: u32 = client::dash3d::CollisionFlag::SQ_BLOCKED as u32;

    fn tile(x: i32) -> WorldTile {
        WorldTile {
            x,
            z: 3200,
            level: 0,
        }
    }

    /// One row of tiles on plane 0; planes 1 to 3 are open rows.
    fn row_collision(plane0: &[u32]) -> WorldCollision {
        let width = plane0.len();
        let mut flags = vec![0u32; width * 4];
        flags[..width].copy_from_slice(plane0);
        let (walk, blocked) = pack_walk(&flags);
        WorldCollision {
            origin: tile(3200),
            width,
            height: 1,
            walk,
            blocked,
            flags: Some(flags),
        }
    }

    fn door(at: i32, to: i32) -> TransportEdge {
        TransportEdge {
            takeoff: None,
            worn_all_req: Vec::new(),
            forbidden_held: Vec::new(),
            forbidden_worn: Vec::new(),
            kind: TransportKind::Door,
            player_delta: None,
            at: tile(at),
            to: tile(to),
            loc_id: 1,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: Vec::new(),
            item_req: Vec::new(),
            consumed_req: Vec::new(),
            item_returns: Vec::new(),
            quest_req: Vec::new(),
            varp_req: Vec::new(),
            worn_req: Vec::new(),
            members_req: false,
            wildy_cap: None,
            quest_gates: None,
        }
    }

    fn bake(collision: &WorldCollision, edges: Vec<TransportEdge>) -> ReachabilityAtlas {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "atlas-bake-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let graph = TransportGraph {
            edges,
            ..TransportGraph::default()
        };
        let atlas = bake_atlas(collision, &graph, &dir, &LocDefs::default());
        let _ = std::fs::remove_dir_all(&dir);
        atlas.unwrap()
    }

    #[test]
    fn adjacent_walkable_tiles_share_one_movement_component() {
        let atlas = bake(&row_collision(&[0, 0]), Vec::new());
        let left = atlas.base_at(tile(3200)).unwrap();
        let right = atlas.base_at(tile(3201)).unwrap();
        assert_eq!(left, right);
        assert!(left.is_some());
        assert_eq!(
            atlas.base_count(),
            4,
            "each of the four open planes is one component"
        );
    }

    /// Label 0 is "no component". The dev profile checks overflow and the
    /// host-play build script bakes in it, so `label - 1` on label 0 aborted
    /// the default bake (`BOT_NAV_BUILD=require`).
    #[test]
    fn label_zero_is_no_component() {
        assert_eq!(label_of(&[0, 1, 5], 0), None);
        assert_eq!(label_of(&[0, 1, 5], 1), Some(0));
        assert_eq!(label_of(&[0, 1, 5], 2), Some(4));
        assert_eq!(label_of(&[0, 1, 5], 3), None);
    }

    /// The landing tile is blocked, so it has label 0. The dev-profile bake
    /// records an exceptional transition. It does not panic.
    #[test]
    fn a_landing_on_a_blocked_tile_is_an_exceptional_transition() {
        let atlas = bake(&row_collision(&[0, BLOCKED, 0]), vec![door(3200, 3201)]);
        assert_eq!(atlas.exceptional_landing_count(), 1);
        assert_eq!(atlas.unmatched_join_count(), 0);
        assert_eq!(atlas.base_at(tile(3201)).unwrap(), None);
    }

    /// Both takeoff tiles are blocked, so no admissible takeoff joins a
    /// component. The row names the landing as well as the anchor.
    #[test]
    fn an_unmatched_join_names_its_landing() {
        let atlas = bake(&row_collision(&[BLOCKED, BLOCKED, 0]), vec![door(3200, 3202)]);
        assert_eq!(atlas.unmatched_join_count(), 1);
        let joins: Vec<_> = atlas.unmatched_joins().collect();
        assert_eq!(joins, vec![(tile(3200), tile(3202), 0)]);
        assert!(atlas.base_at(joins[0].1).unwrap().is_some());
    }

    /// Tile 3201 has a wall face on its west side: a step east into it is
    /// refused, a step west out of it is allowed. The two movement components
    /// are joined by one directed arc, and that arc is part of the condensed
    /// closure: from the one-way side the other side is reachable.
    #[test]
    fn a_one_way_movement_step_is_a_condensed_closure_arc() {
        let atlas = bake(&row_collision(&[0, client::dash3d::CollisionFlag::W_W as u32]), Vec::new());
        let west = atlas.base_at(tile(3200)).unwrap().unwrap();
        let east = atlas.base_at(tile(3201)).unwrap().unwrap();
        assert_ne!(west, east, "the one-way step separates the components");
        let movement_from_east: Vec<u32> = atlas.movement_closure(east).unwrap().collect();
        let movement_from_west: Vec<u32> = atlas.movement_closure(west).unwrap().collect();
        assert!(movement_from_east.contains(&west));
        assert!(!movement_from_west.contains(&east));
        let east_condensed = atlas.condensed_of(east).unwrap();
        let west_condensed = atlas.condensed_of(west).unwrap();
        let transport_from_east: Vec<u32> =
            atlas.transport_closure(east_condensed).unwrap().collect();
        let transport_from_west: Vec<u32> =
            atlas.transport_closure(west_condensed).unwrap().collect();
        assert!(transport_from_east.contains(&west_condensed));
        assert!(!transport_from_west.contains(&east_condensed));
    }

    #[test]
    fn portal_sources_are_the_neighbour_components_not_the_blocked_portal_tile() {
        // Portal at 3201 (blocked); 3200 and 3202 are open and separate
        // components because the portal row between them is blocked.
        let collision = row_collision(&[0, BLOCKED, 0]);
        let atlas = bake(&collision, Vec::new());
        let labels_of = |x: i32| atlas.base_at(tile(x)).unwrap().map(|base| base + 1);
        let cells = [
            labels_of(3200).unwrap_or(0),
            labels_of(3201).unwrap_or(0),
            labels_of(3202).unwrap_or(0),
        ];
        let idx = |t: WorldTile| {
            (t.level == 0 && t.z == 3200 && (3200..3203).contains(&t.x))
                .then(|| (t.x - 3200) as usize)
        };
        let sources = portal_sources(tile(3201), &cells, &idx);
        let west = atlas.base_at(tile(3200)).unwrap().unwrap();
        let east = atlas.base_at(tile(3202)).unwrap().unwrap();
        assert_ne!(west, east);
        assert_eq!(sources, vec![Some(west), Some(east)]);
        let none = portal_sources(tile(3301), &cells, &idx);
        assert_eq!(none, vec![None]);
    }

    fn closure_tables(adj: &[BTreeSet<u32>], limit: usize) -> Vec<Vec<(u32, u32)>> {
        let (offsets, runs) = closures_from_adj_within(adj, limit).unwrap();
        (0..adj.len())
            .map(|i| {
                runs[offsets[i] as usize..offsets[i + 1] as usize]
                    .iter()
                    .map(|run| (run.first, run.count))
                    .collect()
            })
            .collect()
    }

    /// The per-seed walk that replaces the n² bitsets above the memory limit
    /// gives the tables the bitset form gives: a DAG, a cycle, and scattered
    /// arcs. A broken labelling would reach it with millions of singleton
    /// components, so the equality is what keeps its answers honest.
    #[test]
    fn sparse_closures_equal_bitset_closures() {
        let mut graphs: Vec<Vec<BTreeSet<u32>>> = Vec::new();
        let mut chain = vec![BTreeSet::new(); 70];
        for i in 0..69 {
            chain[i].insert(i as u32 + 1);
        }
        graphs.push(chain);
        let mut cycle = vec![BTreeSet::new(); 5];
        for i in 0..5 {
            cycle[i].insert((i as u32 + 1) % 5);
        }
        graphs.push(cycle);
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut scattered = vec![BTreeSet::new(); 150];
        for _ in 0..400 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let from = (state >> 33) as usize % 150;
            let to = (state >> 13) as u32 % 150;
            scattered[from].insert(to);
        }
        graphs.push(scattered);
        graphs.push(vec![BTreeSet::new(); 3]);
        for adj in &graphs {
            assert_eq!(
                closure_tables(adj, usize::MAX),
                closure_tables(adj, 0),
                "{} components",
                adj.len()
            );
        }
    }
}
