//! Dijkstra routing over the whole-world collision bake (`WorldCollision`)
//! and the content-derived transport graph (`TransportGraph`), plus the
//! legacy step-grid A* kept under [`find_on_grid`] for the traveller and
//! live harnesses until the collision+transport router replaces them.
//!
//! One Dijkstra minimizes **total ticks**: a walk step costs the search's
//! run rate ([`CostModel::running`]: 0.5 ticks/tile, 2 tiles/tick) and a
//! transport edge costs its `ticks` (OP_BASE + duration). Costs are `f64`
//! — 0.5 and every reachable sum are exact — ordered with `total_cmp` in
//! the heap so equal costs tie-break on tile coordinates. A step into a
//! neighbour is allowed only when the neighbour's derived walkable word
//! passes the client's directional movement test (the `PL_WALK_*` masks in
//! `tryMove`), never the blanket `walkable()` check.
//!
//! [`find`] and [`find_with_model`] never see teleports; the any-tile
//! teleport layer ([`TransportGraph::teleports`]) only joins the search
//! through [`find_allow_teleports`]/[`find_allow_teleports_with_model`].
//! Wilderness tiles ([`TransportGraph::wilderness`]) are refused unless
//! the search's [`FindOptions::allow_wilderness`] is set; every option'd
//! entry point is [`find_with`]. A graph with no packed zones (a legacy
//! 274N grid) gates nothing. Every transport edge — walked or teleported
//! — is additionally gated by the search's [`WorldState`]: an edge whose
//! requirements the state cannot prove is never relaxed (missing facts
//! fail closed). A quest-stage gate the state's evidence leaves `Unknown`
//! is such a requirement; [`find_unresolved_quest_gates`] names the gates
//! fresh evidence would have to decide, it never crosses them for a route.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use api::identity::Cancellation;
use api::selected::{QuestGate, Truth};
use api::snapshot::WorldTile;
use api::work::{Checkpoint, StepContext, WorkPhase};
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::contract::{ContractError, NavLease, PackId, RouteView};
use crate::essence::{EssenceSession, ESSENCE_MINE_EXIT_TICKS, ESSENCE_MINE_PORTALS};
use crate::quest_gates::QuestFamilyMismatch;
use crate::transport::{select_web_action, TransportEdge, TransportGraph};
use crate::world_state::WorldState;
use crate::zones::{ZoneExempt, ZoneFilter, ZoneFilterOwned, ZoneKey, ZoneTable};

mod resources;
#[cfg(unix)]
mod zeroed_buffer;
use resources::{
    BacktraceStep, Budget, PackedLookup, PackedMinMaxHeap, PackedSearchSlotExt, PackedSearchTable,
    Predecessors, ResourceBudget, SearchKey, Unmetered, PACKED_ROOT_ID,
};

thread_local! {
    // Only the synchronous driver suppresses cooperative suspension. Cursor
    // polls keep one Pending result per bounded work unit.
    static SYNCHRONOUS_WORK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct SynchronousWorkGuard(bool);

impl Drop for SynchronousWorkGuard {
    fn drop(&mut self) {
        SYNCHRONOUS_WORK.set(self.0);
    }
}

struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

fn work_unit() -> YieldOnce {
    YieldOnce(SYNCHRONOUS_WORK.get())
}

async fn sift_down_by<T>(
    values: &mut [T],
    mut root: usize,
    end: usize,
    compare: &impl Fn(&T, &T) -> Ordering,
) {
    loop {
        let Some(child) = root.checked_mul(2).and_then(|child| child.checked_add(1)) else {
            return;
        };
        if child >= end {
            return;
        }
        let child = if child + 1 < end && compare(&values[child], &values[child + 1]).is_lt() {
            child + 1
        } else {
            child
        };
        if !compare(&values[root], &values[child]).is_lt() {
            return;
        }
        values.swap(root, child);
        root = child;
    }
}

async fn sort_unstable_by_cooperative<T>(values: &mut [T], compare: impl Fn(&T, &T) -> Ordering) {
    let len = values.len();
    for root in (0..len / 2).rev() {
        sift_down_by(values, root, len, &compare).await;
    }
    for end in (1..len).rev() {
        values.swap(0, end);
        sift_down_by(values, 0, end, &compare).await;
    }
}

async fn dedup_sorted_by_cooperative<T: PartialEq>(values: &mut Vec<T>) {
    let mut kept = 0;
    let mut scanned = 0;
    while scanned < values.len() {
        work_unit().await;
        if kept == 0 || values[kept - 1] != values[scanned] {
            if kept != scanned {
                values.swap(kept, scanned);
            }
            kept += 1;
        }
        scanned += 1;
    }
    while values.len() > kept {
        work_unit().await;
        drop(values.pop());
    }
}

async fn reverse_cooperative<T>(values: &mut [T]) {
    let len = values.len();
    for left in 0..len / 2 {
        work_unit().await;
        values.swap(left, len - left - 1);
    }
}

fn drive_to_completion<F: Future>(future: F) -> F::Output {
    let _synchronous_work = SynchronousWorkGuard(SYNCHRONOUS_WORK.replace(true));
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

/// One leg of a route: a walk run or one transport crossing. Consecutive
/// walk tiles collapse into a single `Walk` leg; each transport edge is
/// its own `Transport` leg.
/// Transport payloads are boxed only when reconstructing a route, keeping
/// ordinary walk legs small as transport facts grow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leg {
    Walk { tiles: Vec<WorldTile> },
    Transport { edge: Box<TransportEdge> },
}

/// A route from the `find` origin to `dest`. `ticks` is the total tick
/// cost of the whole route: walk steps at the search's run rate (0.5 per
/// tile, exact in f64) plus every transport edge's `ticks`. Half-ticks are
/// never truncated.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub legs: Vec<Leg>,
    pub dest: WorldTile,
    pub ticks: f64,
}
/// Route custody stored without a per-hop `TransportEdge` clone. Walk runs
/// borrow slices from `walk_tiles`; transport legs name one immutable graph
/// edge and resolve their actual landing from the preceding route tile.
pub struct CompactRoute {
    origin: WorldTile,
    dest: WorldTile,
    ticks: f64,
    walk_tiles: Box<[WorldTile]>,
    /// Two-bit transition tags: walk, transport, teleport, essence return.
    tags: Box<[u8]>,
    transition_count: u32,
    /// Legacy leg count (walk gaps and transports alternate).
    leg_count: u32,
    edge_ids: Box<[u32]>,
    essence: Option<EssenceSession>,
    checkpoints: Box<[CompactCheckpoint]>,
    lease: Option<crate::contract::NavLease>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompactCheckpoint {
    pub(crate) transition: u32,
    pub(crate) walk_tile: u32,
    pub(crate) edge: u32,
    pub(crate) tile: WorldTile,
}

/// One borrowed leg of a compact route.
#[derive(Clone, Copy, Debug)]
pub enum CompactRouteLeg<'a> {
    /// Consecutive walk tiles, including the run's starting tile.
    Walk(&'a [WorldTile]),
    /// A transport edge or session-only essence return.
    Transport(CompactTransportStep<'a>),
}

/// Borrowed transport facts plus the exact route-relative landing.
#[derive(Clone, Copy, Debug)]
pub struct CompactTransportStep<'a> {
    edge_index: Option<u32>,
    portal_index: Option<u8>,
    edge: Option<&'a TransportEdge>,
    kind: crate::transport::TransportKind,
    loc_id: i32,
    anchor: WorldTile,
    from: WorldTile,
    to: WorldTile,
    option: i32,
    ticks: i32,
}

impl<'a> CompactTransportStep<'a> {
    /// Packed edge index; `None` only for the session-synthesized essence return.
    pub const fn edge_index(self) -> Option<u32> {
        self.edge_index
    }

    /// Borrowed packed edge, or `None` for the synthetic essence return.
    pub const fn source(self) -> Option<&'a TransportEdge> {
        self.edge
    }

    /// Transport kind.
    pub const fn kind(self) -> crate::transport::TransportKind {
        self.kind
    }

    /// Interacted loc/NPC/object id.
    pub const fn loc_id(self) -> i32 {
        self.loc_id
    }

    /// Packed interaction anchor; for a teleport this is the packed value.
    pub const fn anchor(self) -> WorldTile {
        self.anchor
    }

    /// Actual tile the search departed from.
    pub const fn actual_from(self) -> WorldTile {
        self.from
    }

    /// Actual landing, including player-relative displacement.
    pub const fn actual_to(self) -> WorldTile {
        self.to
    }

    /// Interaction option.
    pub const fn option(self) -> i32 {
        self.option
    }

    /// Exact transport tick cost.
    pub const fn ticks(self) -> i32 {
        self.ticks
    }
}

impl CompactRoute {
    pub(crate) fn retain_lease(&mut self, lease: crate::contract::NavLease) {
        self.lease = Some(lease);
    }

    pub(crate) fn pack(&self) -> Option<PackId> {
        self.lease.as_ref().map(|lease| lease.identity().pack())
    }

    pub(crate) fn graph(&self) -> Option<&TransportGraph> {
        self.lease.as_ref().map(crate::contract::NavLease::graph)
    }
}

impl std::fmt::Debug for CompactRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some(graph) = self.graph() else {
            return f
                .debug_struct("CompactRoute")
                .field("dest", &self.dest)
                .field("ticks", &self.ticks)
                .field("leg_count", &self.leg_count)
                .finish_non_exhaustive();
        };
        std::fmt::Debug::fmt(&CompactRouteDebug { route: self, graph }, f)
    }
}

struct CompactRouteDebug<'a> {
    route: &'a CompactRoute,
    graph: &'a TransportGraph,
}

impl std::fmt::Debug for CompactRouteDebug<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field(
                "legs",
                &CompactDebugLegs {
                    route: self.route,
                    graph: self.graph,
                },
            )
            .field("dest", &self.route.dest)
            .field("ticks", &self.route.ticks)
            .finish()
    }
}

struct CompactDebugLegs<'a> {
    route: &'a CompactRoute,
    graph: &'a TransportGraph,
}

impl std::fmt::Debug for CompactDebugLegs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(CompactDebugIter {
                cursor: self.route.cursor(self.graph),
            })
            .finish()
    }
}

struct CompactDebugIter<'a> {
    cursor: CompactRouteCursor<'a>,
}

impl<'a> std::iter::Iterator for CompactDebugIter<'a> {
    type Item = CompactDebugLeg<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next_leg().map(|leg| match leg {
            CompactRouteLeg::Walk(tiles) => CompactDebugLeg::Walk(tiles),
            CompactRouteLeg::Transport(step) => CompactDebugLeg::Transport(step),
        })
    }
}

enum CompactDebugLeg<'a> {
    Walk(&'a [WorldTile]),
    Transport(CompactTransportStep<'a>),
}

impl std::fmt::Debug for CompactDebugLeg<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Walk(tiles) => f.debug_struct("Walk").field("tiles", tiles).finish(),
            Self::Transport(step) => f
                .debug_struct("Transport")
                .field("edge", &CompactEdgeDebug(*step))
                .finish(),
        }
    }
}

struct CompactEdgeDebug<'a>(CompactTransportStep<'a>);

impl std::fmt::Debug for CompactEdgeDebug<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let step = self.0;
        if let Some(edge) = step.source() {
            let to = if edge.player_delta.is_some() {
                step.actual_to()
            } else {
                edge.to
            };
            return f
                .debug_struct("TransportEdge")
                .field("kind", &edge.kind)
                .field("at", &edge.at)
                .field("to", &to)
                .field("takeoff", &edge.takeoff)
                .field("player_delta", &edge.player_delta)
                .field("loc_id", &edge.loc_id)
                .field("option", &edge.option)
                .field("ticks", &edge.ticks)
                .field("dir", &edge.dir)
                .field("open_loc_id", &edge.open_loc_id)
                .field("skill_req", &edge.skill_req)
                .field("item_req", &edge.item_req)
                .field("consumed_req", &edge.consumed_req)
                .field("item_returns", &edge.item_returns)
                .field("quest_req", &edge.quest_req)
                .field("varp_req", &edge.varp_req)
                .field("worn_req", &edge.worn_req)
                .field("worn_all_req", &edge.worn_all_req)
                .field("members_req", &edge.members_req)
                .field("wildy_cap", &edge.wildy_cap)
                .field("quest_gates", &edge.quest_gates)
                .finish();
        }
        let empty_pairs: &[(i32, i32)] = &[];
        let empty_quests: &[String] = &[];
        let empty_ids: &[i32] = &[];
        f.debug_struct("TransportEdge")
            .field("kind", &crate::transport::TransportKind::EssenceExit)
            .field("at", &step.anchor())
            .field("to", &step.actual_to())
            .field("takeoff", &Option::<WorldTile>::None)
            .field("player_delta", &Option::<WorldTile>::None)
            .field("loc_id", &crate::essence::ESSENCE_MINE_PORTAL_LOC_ID)
            .field("option", &1)
            .field("ticks", &ESSENCE_MINE_EXIT_TICKS)
            .field("dir", &Option::<crate::transport::DoorDir>::None)
            .field("open_loc_id", &Option::<i32>::None)
            .field("skill_req", &empty_pairs)
            .field("item_req", &empty_pairs)
            .field("consumed_req", &empty_pairs)
            .field("item_returns", &empty_pairs)
            .field("quest_req", &empty_quests)
            .field("varp_req", &empty_pairs)
            .field("worn_req", &empty_ids)
            .field("worn_all_req", &empty_ids)
            .field("members_req", &false)
            .field("wildy_cap", &Option::<i32>::None)
            .field(
                "quest_gates",
                &Option::<crate::quest_gates::QuestGates>::None,
            )
            .finish()
    }
}

impl CompactRoute {
    /// Exact destination.
    pub const fn destination(&self) -> WorldTile {
        self.dest
    }

    /// Exact legacy route tick sum.
    pub const fn ticks(&self) -> f64 {
        self.ticks
    }

    /// Number of compact route legs.
    pub const fn leg_count(&self) -> usize {
        self.leg_count as usize
    }

    /// Checked retained bytes for the owned route buffers.
    pub fn try_retained_bytes(&self) -> Option<usize> {
        self.walk_tiles
            .len()
            .checked_mul(std::mem::size_of::<WorldTile>())?
            .checked_add(self.tags.len())?
            .checked_add(
                self.edge_ids
                    .len()
                    .checked_mul(std::mem::size_of::<u32>())?,
            )?
            .checked_add(
                self.checkpoints
                    .len()
                    .checked_mul(std::mem::size_of::<CompactCheckpoint>())?,
            )
    }
    /// Checked maximum retained bytes for an owned compact route built from
    /// at most `max_settles` predecessor transitions. Includes the inline
    /// route header and all backing buffers. Cursor reservations account
    /// separately for temporary materialization and conversion overlap.
    pub fn storage_upper_bound(max_settles: usize) -> Option<usize> {
        route_result_bytes(max_settles)
    }

    /// Bytes retained by this compact plan's owned buffers.
    ///
    /// Use [`Self::try_retained_bytes`] for admission and accounting.
    pub fn retained_bytes(&self) -> usize {
        self.try_retained_bytes().unwrap_or(usize::MAX)
    }

    /// Borrow the legs while resolving only the graph references they name.
    pub fn cursor<'a>(&'a self, graph: &'a TransportGraph) -> CompactRouteCursor<'a> {
        CompactRouteCursor {
            route: self,
            graph,
            leg: 0,
            transition: 0,
            walk_tile: 0,
            edge: 0,
            tile: self.origin,
        }
    }

    /// Bounded random-access view. The nearest checkpoint is at most 63 legs
    /// away; no route-wide or graph-wide scan is performed.
    pub fn leg_at<'a>(
        &'a self,
        graph: &'a TransportGraph,
        index: usize,
    ) -> Option<CompactRouteLeg<'a>> {
        if index >= self.leg_count as usize {
            return None;
        }
        let checkpoint_index = index / 64;
        let checkpoint = self.checkpoints.get(checkpoint_index)?;
        let checkpoint_leg = checkpoint_index * 64;
        let mut cursor = CompactRouteCursor {
            route: self,
            graph,
            leg: checkpoint_leg,
            transition: checkpoint.transition as usize,
            walk_tile: checkpoint.walk_tile as usize,
            edge: checkpoint.edge as usize,
            tile: checkpoint.tile,
        };
        while cursor.leg < index {
            cursor.next_leg()?;
        }
        cursor.next_leg()
    }

    /// Materialize one leg for the existing Traveller state machine or its
    /// legacy callback. Compact walk legs are otherwise borrowed directly.
    pub(crate) fn owned_leg(&self, graph: &TransportGraph, index: usize) -> Option<Leg> {
        Some(match self.leg_at(graph, index)? {
            CompactRouteLeg::Walk(tiles) => Leg::Walk {
                tiles: tiles.to_vec(),
            },
            CompactRouteLeg::Transport(step) => {
                let edge = if let Some(edge) = step.source() {
                    let mut edge = edge.clone();
                    if edge.player_delta.is_some() {
                        edge.to = step.actual_to();
                    }
                    edge
                } else {
                    crate::essence::essence_return_edge(
                        *ESSENCE_MINE_PORTALS.get(usize::from(step.portal_index?))?,
                        self.essence.as_ref()?,
                    )
                };
                Leg::Transport {
                    edge: Box::new(edge),
                }
            }
        })
    }

    /// Materialize the legacy public Vec/Box route for synchronous callers.
    /// Native cooperative jobs must retain this compact representation.
    pub fn to_route(&self, graph: &TransportGraph) -> Route {
        let mut legs = Vec::with_capacity(self.leg_count as usize);
        for leg in self.cursor(graph) {
            match leg {
                CompactRouteLeg::Walk(tiles) => legs.push(Leg::Walk {
                    tiles: tiles.to_vec(),
                }),
                CompactRouteLeg::Transport(step) => {
                    let edge = if let Some(edge) = step.source() {
                        let mut edge = edge.clone();
                        if edge.player_delta.is_some() {
                            edge.to = step.actual_to();
                        }
                        edge
                    } else {
                        crate::essence::essence_return_edge(
                            ESSENCE_MINE_PORTALS[usize::from(
                                step.portal_index
                                    .expect("essence return carries its portal index"),
                            )],
                            &self.essence.expect("essence route retains its session"),
                        )
                    };
                    legs.push(Leg::Transport {
                        edge: Box::new(edge),
                    });
                }
            }
        }
        Route {
            legs,
            dest: self.dest,
            ticks: self.ticks,
        }
    }

    fn tag(&self, index: usize) -> u8 {
        (self.tags[index >> 2] >> ((index & 3) * 2)) & 0b11
    }
}

/// Allocation-free sequential compact route traversal.
pub struct CompactRouteCursor<'a> {
    route: &'a CompactRoute,
    graph: &'a TransportGraph,
    leg: usize,
    transition: usize,
    walk_tile: usize,
    edge: usize,
    tile: WorldTile,
}

impl<'a> CompactRouteCursor<'a> {
    /// Advance exactly one compact leg without allocating.
    pub fn next_leg(&mut self) -> Option<CompactRouteLeg<'a>> {
        if self.leg >= self.route.leg_count as usize {
            return None;
        }
        if self.leg & 1 == 0 {
            let start = self.walk_tile;
            self.walk_tile += 1;
            while self.transition < self.route.transition_count as usize
                && self.route.tag(self.transition) == 0
            {
                self.transition += 1;
                self.walk_tile += 1;
            }
            let tiles = self.route.walk_tiles.get(start..self.walk_tile)?;
            self.tile = *tiles.last()?;
            self.leg += 1;
            return Some(CompactRouteLeg::Walk(tiles));
        }

        let tag = self.route.tag(self.transition);
        self.transition += 1;
        self.leg += 1;
        let index = *self.route.edge_ids.get(self.edge)?;
        self.edge += 1;
        match tag {
            1 | 2 => {
                let (edge_index, edge) = if tag == 1 {
                    let edge = self.graph.edges.get(index as usize)?;
                    (Some(index), edge)
                } else {
                    let edge = self.graph.teleports.get(index as usize)?;
                    (Some(index), edge)
                };
                let from = self.tile;
                let to = edge
                    .landing_from(from)
                    .expect("compact route records an admissible transport landing");
                self.tile = to;
                Some(CompactRouteLeg::Transport(CompactTransportStep {
                    edge_index,
                    portal_index: None,
                    edge: Some(edge),
                    kind: edge.kind,
                    loc_id: edge.loc_id,
                    anchor: edge.at,
                    from,
                    to,
                    option: edge.option,
                    ticks: edge.ticks,
                }))
            }
            _ => {
                let portal = u8::try_from(index).ok()?;
                let at = *ESSENCE_MINE_PORTALS.get(usize::from(portal))?;
                let session = self.route.essence?;
                let from = self.tile;
                let to = session.return_tile;
                self.tile = to;
                Some(CompactRouteLeg::Transport(CompactTransportStep {
                    edge_index: None,
                    portal_index: Some(portal),
                    edge: None,
                    kind: crate::transport::TransportKind::EssenceExit,
                    loc_id: crate::essence::ESSENCE_MINE_PORTAL_LOC_ID,
                    anchor: at,
                    from,
                    to,
                    option: 1,
                    ticks: ESSENCE_MINE_EXIT_TICKS,
                }))
            }
        }
    }
}

impl<'a> Iterator for CompactRouteCursor<'a> {
    type Item = CompactRouteLeg<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_leg()
    }
}

impl CompactRoute {
    #[expect(
        clippy::too_many_arguments,
        reason = "the producer transfers each compact buffer without an intermediate allocation"
    )]
    pub(crate) fn from_compact_parts(
        origin: WorldTile,
        dest: WorldTile,
        ticks: f64,
        walk_tiles: Box<[WorldTile]>,
        tags: Box<[u8]>,
        transition_count: u32,
        leg_count: u32,
        edge_ids: Box<[u32]>,
        essence: Option<EssenceSession>,
        checkpoints: Box<[CompactCheckpoint]>,
    ) -> Self {
        Self {
            origin,
            dest,
            ticks,
            walk_tiles,
            tags,
            transition_count,
            leg_count,
            edge_ids,
            essence,
            checkpoints,
            lease: None,
        }
    }

    async fn build_checkpoints(&mut self, graph: &TransportGraph) -> Result<(), SearchCursorError> {
        let checkpoint_count = (self.leg_count as usize).div_ceil(64);
        let mut checkpoints = Vec::new();
        checkpoints
            .try_reserve_exact(checkpoint_count)
            .map_err(|_| SearchCursorError::Allocation)?;
        let mut cursor = self.cursor(graph);
        for leg in 0..self.leg_count as usize {
            if leg & 63 == 0 {
                checkpoints.push(CompactCheckpoint {
                    transition: u32::try_from(cursor.transition)
                        .map_err(|_| SearchCursorError::ResultNotRepresentable)?,
                    walk_tile: u32::try_from(cursor.walk_tile)
                        .map_err(|_| SearchCursorError::ResultNotRepresentable)?,
                    edge: u32::try_from(cursor.edge)
                        .map_err(|_| SearchCursorError::ResultNotRepresentable)?,
                    tile: cursor.tile,
                });
            }
            if leg & 1 == 0 {
                cursor.walk_tile = cursor
                    .walk_tile
                    .checked_add(1)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                while cursor.transition < self.transition_count as usize
                    && self.tag(cursor.transition) == 0
                {
                    work_unit().await;
                    cursor.transition += 1;
                    cursor.walk_tile += 1;
                }
                cursor.tile = *self
                    .walk_tiles
                    .get(
                        cursor
                            .walk_tile
                            .checked_sub(1)
                            .ok_or(SearchCursorError::ResultNotRepresentable)?,
                    )
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
            } else {
                work_unit().await;
                let tag = self.tag(cursor.transition);
                cursor.transition += 1;
                cursor.leg += 1;
                let index = *self
                    .edge_ids
                    .get(cursor.edge)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                cursor.edge += 1;
                cursor.tile = match tag {
                    1 => graph
                        .edges
                        .get(index as usize)
                        .and_then(|edge| edge.landing_from(cursor.tile)),
                    2 => graph.teleports.get(index as usize).map(|edge| edge.to),
                    3 => self.essence.as_ref().map(|session| session.return_tile),
                    _ => None,
                }
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            }
            if leg & 1 == 0 {
                cursor.leg += 1;
            }
        }
        if cursor.transition != self.transition_count as usize
            || cursor.walk_tile != self.walk_tiles.len()
            || cursor.edge != self.edge_ids.len()
            || checkpoints.len() != checkpoint_count
        {
            return Err(SearchCursorError::ResultNotRepresentable);
        }
        self.checkpoints = checkpoints.into_boxed_slice();
        Ok(())
    }
}

/// Run pace: 2 tiles per tick, 0.5 ticks per tile.
pub const PER_STEP_RUN: f64 = 0.5;
/// Walk pace: 1 tile per tick, 1 tick per tile.
pub const PER_STEP_WALK: f64 = 1.0;

/// The per-search walking rate. This task's searches walk at the fixed
/// running pace ([`CostModel::running`]); the agility/regen energy-aware
/// fallback will flip to the walk pace when run energy can't sustain
/// running.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostModel {
    /// Ticks per walked tile at the running pace (0.5).
    pub run_per_step: f64,
    /// Ticks per walked tile at the walking pace (1.0).
    pub walk_per_step: f64,
}

impl CostModel {
    /// The fixed running rate this task searches with.
    pub fn running() -> Self {
        CostModel {
            run_per_step: PER_STEP_RUN,
            walk_per_step: PER_STEP_WALK,
        }
    }
}

/// Per-search opt-ins, all default off so [`find`] keeps the safe
/// defaults. `allow_teleports` unions the any-tile teleport layer in;
/// `allow_wilderness` lets the search step into (or land in) packed
/// [`TransportGraph::wilderness`] zones. A graph with no packed zones
/// gates nothing.
/// `allow_bank_fetch` is the BankBudget opt-in: on its own it never
/// inserts a bank leg or relaxes an item req — an edge stays unusable
/// unless the search's [`WorldState`] already proves it. The
/// fetch-and-wear session lives outside the router
/// ([`crate::bank_fetch::plan_bank_fetch`], whose diagnosis arm is
/// [`find_missing_item_reqs`]); a caller that sets the flag but plans no
/// session gets exactly the fail-closed search. `essence` is the
/// per-slot Rune Essence mine latch
/// ([`EssenceSession`]): when the player stands inside the enclosed mine
/// the search relaxes the exit portal's return hop to the entry wizard's
/// overworld anchor. `None` (the default) keeps the mine a sealed dead
/// end — the pack carries no return edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FindOptions {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    /// For bank access goals, break equal path costs by input target order.
    /// Compare safe arrivals with each destination's own-zone completion.
    /// Keep transit grants as fallback permissions and the full strict budget
    /// while a goal remains unproven. Non-first-goal searches ignore this option.
    pub prefer_target_order: bool,
    pub essence: Option<EssenceSession>,
    /// Named whole-walk zone opt-outs. The route origin may exit its danger
    /// zones and the selected destination may be reached inside its own zones.
    pub zones: ZoneExempt,
}

/// Axis-aligned avoidance rectangle for inspect-local routing. Bounds are
/// inclusive on both axes. When [`AvoidRect::level`] is [`None`], the rect
/// applies on every plane; otherwise only that level matches.
///
/// Callers must supply valid geometry (`min_x <= max_x`, `min_z <= max_z`).
/// Invalid rects are not validated here and do not produce a router error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvoidRect {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    /// `None` = every level.
    pub level: Option<i32>,
}

impl AvoidRect {
    /// Whether `tile` sits inside this rect (inclusive bounds, optional level).
    pub fn contains(self, tile: WorldTile) -> bool {
        if let Some(lvl) = self.level {
            if tile.level != lvl {
                return false;
            }
        }
        tile.x >= self.min_x && tile.x <= self.max_x && tile.z >= self.min_z && tile.z <= self.max_z
    }
}

/// Which blocker-attribution rule a cooperative diagnosis uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockerDiagnosis {
    /// Attribute only zones crossed on the best route to one target.
    SingleTarget,
    /// Attribute the reachable frontier for the selected target set.
    FirstGoal,
}

/// Owned request for one cooperative route search or route diagnosis. Input
/// arrays and compact routes move into the cursor without cloning.
pub enum SearchRequest {
    Single {
        from: WorldTile,
        to: WorldTile,
        options: FindOptions,
        avoid: Box<[AvoidRect]>,
        max_settles: usize,
    },
    Many {
        from: WorldTile,
        targets: Box<[WorldTile]>,
        options: FindOptions,
        avoid: Box<[AvoidRect]>,
        max_settles: usize,
        reconstruct_best: bool,
    },
    First {
        from: WorldTile,
        targets: Box<[WorldTile]>,
        fallback: Box<[WorldTile]>,
        options: FindOptions,
        avoid: Box<[AvoidRect]>,
        max_settles: usize,
    },
    MissingItemReqs {
        from: WorldTile,
        targets: Box<[WorldTile]>,
        fallback: Box<[WorldTile]>,
        options: FindOptions,
        avoid: Box<[AvoidRect]>,
        max_settles: usize,
    },
    /// Diagnose carry/wear requirements on this exact owned route, without
    /// running another route search.
    MissingItemReqsForRoute { route: CompactRoute },
    BlockingZones {
        diagnosis: BlockerDiagnosis,
        from: WorldTile,
        targets: Box<[WorldTile]>,
        options: FindOptions,
        avoid: Box<[AvoidRect]>,
        max_settles: usize,
        zone_filter: ZoneFilterOwned,
    },
    UnresolvedQuestGates {
        from: WorldTile,
        targets: Box<[WorldTile]>,
        options: FindOptions,
        avoid: Box<[AvoidRect]>,
        max_settles: usize,
    },
}
/// Allocation shape used by router producers before materializing target
/// arrays. This is an implementation helper, not part of `nav::contract`.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchRequestShape {
    Single {
        options: FindOptions,
        avoid_capacity: usize,
        max_settles: usize,
    },
    Many {
        target_capacity: usize,
        options: FindOptions,
        avoid_capacity: usize,
        max_settles: usize,
        reconstruct_best: bool,
    },
    First {
        target_capacity: usize,
        fallback_capacity: usize,
        options: FindOptions,
        avoid_capacity: usize,
        max_settles: usize,
    },
    MissingItemReqs {
        target_capacity: usize,
        fallback_capacity: usize,
        options: FindOptions,
        avoid_capacity: usize,
        max_settles: usize,
    },
    MissingItemReqsForRoute {
        route_bytes: usize,
        edge_capacity: usize,
        route_pack: PackId,
    },
    BlockingZones {
        diagnosis: BlockerDiagnosis,
        target_capacity: usize,
        options: FindOptions,
        avoid_capacity: usize,
        zone_filter_bytes: usize,
        max_settles: usize,
    },
    UnresolvedQuestGates {
        target_capacity: usize,
        options: FindOptions,
        avoid_capacity: usize,
        max_settles: usize,
    },
}

impl SearchRequestShape {
    fn from_request(request: &SearchRequest) -> Option<Self> {
        let shape = match request {
            SearchRequest::Single {
                options,
                avoid,
                max_settles,
                ..
            } => Self::Single {
                options: *options,
                avoid_capacity: avoid.len(),
                max_settles: *max_settles,
            },
            SearchRequest::Many {
                targets,
                options,
                avoid,
                max_settles,
                reconstruct_best,
                ..
            } => Self::Many {
                target_capacity: targets.len(),
                options: *options,
                avoid_capacity: avoid.len(),
                max_settles: *max_settles,
                reconstruct_best: *reconstruct_best,
            },
            SearchRequest::First {
                targets,
                fallback,
                options,
                avoid,
                max_settles,
                ..
            } => Self::First {
                target_capacity: targets.len(),
                fallback_capacity: fallback.len(),
                options: *options,
                avoid_capacity: avoid.len(),
                max_settles: *max_settles,
            },
            SearchRequest::MissingItemReqs {
                targets,
                fallback,
                options,
                avoid,
                max_settles,
                ..
            } => Self::MissingItemReqs {
                target_capacity: targets.len(),
                fallback_capacity: fallback.len(),
                options: *options,
                avoid_capacity: avoid.len(),
                max_settles: *max_settles,
            },
            SearchRequest::MissingItemReqsForRoute { route } => Self::MissingItemReqsForRoute {
                route_bytes: route.try_retained_bytes()?,
                edge_capacity: route.edge_ids.len(),
                route_pack: route.lease.as_ref()?.identity().pack(),
            },
            SearchRequest::BlockingZones {
                diagnosis,
                targets,
                options,
                avoid,
                max_settles,
                zone_filter,
                ..
            } => Self::BlockingZones {
                diagnosis: *diagnosis,
                target_capacity: targets.len(),
                options: *options,
                avoid_capacity: avoid.len(),
                zone_filter_bytes: zone_filter.retained_bytes()?,
                max_settles: *max_settles,
            },
            SearchRequest::UnresolvedQuestGates {
                targets,
                options,
                avoid,
                max_settles,
                ..
            } => Self::UnresolvedQuestGates {
                target_capacity: targets.len(),
                options: *options,
                avoid_capacity: avoid.len(),
                max_settles: *max_settles,
            },
        };
        shape.is_consistent().then_some(shape)
    }

    fn is_consistent(self) -> bool {
        !matches!(
            self,
            Self::BlockingZones {
                diagnosis: BlockerDiagnosis::SingleTarget,
                target_capacity,
                ..
            } if target_capacity != 1
        )
    }

    fn options(self) -> FindOptions {
        match self {
            Self::MissingItemReqsForRoute { .. } => FindOptions::default(),
            Self::Single { options, .. }
            | Self::Many { options, .. }
            | Self::First { options, .. }
            | Self::MissingItemReqs { options, .. }
            | Self::BlockingZones { options, .. }
            | Self::UnresolvedQuestGates { options, .. } => options,
        }
    }

    const fn max_settles(self) -> usize {
        match self {
            Self::Single { max_settles, .. }
            | Self::Many { max_settles, .. }
            | Self::First { max_settles, .. }
            | Self::MissingItemReqs { max_settles, .. }
            | Self::BlockingZones { max_settles, .. }
            | Self::UnresolvedQuestGates { max_settles, .. } => max_settles,
            Self::MissingItemReqsForRoute { .. } => 0,
        }
    }

    fn covers(self, actual: Self) -> bool {
        match (self, actual) {
            (
                Self::Single {
                    options,
                    avoid_capacity,
                    max_settles,
                },
                Self::Single {
                    options: actual_options,
                    avoid_capacity: actual_avoid,
                    max_settles: actual_settles,
                },
            ) => {
                options == actual_options
                    && avoid_capacity >= actual_avoid
                    && max_settles >= actual_settles
            }
            (
                Self::Many {
                    target_capacity,
                    options,
                    avoid_capacity,
                    max_settles,
                    reconstruct_best,
                },
                Self::Many {
                    target_capacity: actual_targets,
                    options: actual_options,
                    avoid_capacity: actual_avoid,
                    max_settles: actual_settles,
                    reconstruct_best: actual_reconstruct_best,
                },
            ) => {
                options == actual_options
                    && target_capacity >= actual_targets
                    && avoid_capacity >= actual_avoid
                    && max_settles >= actual_settles
                    && reconstruct_best == actual_reconstruct_best
            }
            (
                Self::First {
                    target_capacity,
                    fallback_capacity,
                    options,
                    avoid_capacity,
                    max_settles,
                },
                Self::First {
                    target_capacity: actual_targets,
                    fallback_capacity: actual_fallback,
                    options: actual_options,
                    avoid_capacity: actual_avoid,
                    max_settles: actual_settles,
                },
            )
            | (
                Self::MissingItemReqs {
                    target_capacity,
                    fallback_capacity,
                    options,
                    avoid_capacity,
                    max_settles,
                },
                Self::MissingItemReqs {
                    target_capacity: actual_targets,
                    fallback_capacity: actual_fallback,
                    options: actual_options,
                    avoid_capacity: actual_avoid,
                    max_settles: actual_settles,
                },
            ) => {
                options == actual_options
                    && target_capacity >= actual_targets
                    && fallback_capacity >= actual_fallback
                    && avoid_capacity >= actual_avoid
                    && max_settles >= actual_settles
            }
            (
                Self::BlockingZones {
                    diagnosis,
                    target_capacity,
                    options,
                    avoid_capacity,
                    zone_filter_bytes,
                    max_settles,
                },
                Self::BlockingZones {
                    diagnosis: actual_diagnosis,
                    target_capacity: actual_targets,
                    options: actual_options,
                    avoid_capacity: actual_avoid,
                    zone_filter_bytes: actual_zone_filter_bytes,
                    max_settles: actual_settles,
                },
            ) => {
                diagnosis == actual_diagnosis
                    && options == actual_options
                    && target_capacity >= actual_targets
                    && avoid_capacity >= actual_avoid
                    && zone_filter_bytes >= actual_zone_filter_bytes
                    && max_settles >= actual_settles
            }
            (
                Self::UnresolvedQuestGates {
                    target_capacity,
                    options,
                    avoid_capacity,
                    max_settles,
                },
                Self::UnresolvedQuestGates {
                    target_capacity: actual_targets,
                    options: actual_options,
                    avoid_capacity: actual_avoid,
                    max_settles: actual_settles,
                },
            ) => {
                options == actual_options
                    && target_capacity >= actual_targets
                    && avoid_capacity >= actual_avoid
                    && max_settles >= actual_settles
            }
            (
                Self::MissingItemReqsForRoute {
                    route_bytes,
                    edge_capacity,
                    route_pack,
                },
                Self::MissingItemReqsForRoute {
                    route_bytes: actual_route_bytes,
                    edge_capacity: actual_edges,
                    route_pack: actual_route_pack,
                },
            ) => {
                route_pack == actual_route_pack
                    && route_bytes >= actual_route_bytes
                    && edge_capacity >= actual_edges
            }
            _ => false,
        }
    }

    fn matches_pack(self, pack: PackId) -> bool {
        match self {
            Self::MissingItemReqsForRoute { route_pack, .. } => route_pack == pack,
            _ => true,
        }
    }

    fn target_count(self) -> Option<usize> {
        match self {
            Self::Single { .. } => Some(1),
            Self::Many {
                target_capacity, ..
            }
            | Self::BlockingZones {
                target_capacity, ..
            }
            | Self::UnresolvedQuestGates {
                target_capacity, ..
            } => Some(target_capacity),
            Self::First {
                target_capacity,
                fallback_capacity,
                ..
            }
            | Self::MissingItemReqs {
                target_capacity,
                fallback_capacity,
                ..
            } => target_capacity.checked_add(fallback_capacity),
            Self::MissingItemReqsForRoute { .. } => Some(0),
        }
    }

    fn retained_input_bytes(self) -> Option<usize> {
        let (targets, fallback, avoid, zone_filter) = match self {
            Self::Single { avoid_capacity, .. } => (0, 0, avoid_capacity, 0),
            Self::Many {
                target_capacity,
                avoid_capacity,
                ..
            }
            | Self::UnresolvedQuestGates {
                target_capacity,
                avoid_capacity,
                ..
            } => (target_capacity, 0, avoid_capacity, 0),
            Self::First {
                target_capacity,
                fallback_capacity,
                avoid_capacity,
                ..
            }
            | Self::MissingItemReqs {
                target_capacity,
                fallback_capacity,
                avoid_capacity,
                ..
            } => (target_capacity, fallback_capacity, avoid_capacity, 0),
            Self::BlockingZones {
                target_capacity,
                avoid_capacity,
                zone_filter_bytes,
                ..
            } => (target_capacity, 0, avoid_capacity, zone_filter_bytes),
            Self::MissingItemReqsForRoute { .. } => (0, 0, 0, 0),
        };

        if let Self::MissingItemReqsForRoute { route_bytes, .. } = self {
            return Some(route_bytes);
        }

        let mut bytes = 0usize;
        add_input_bound(&mut bytes, targets, std::mem::size_of::<WorldTile>())?;
        add_input_bound(&mut bytes, fallback, std::mem::size_of::<WorldTile>())?;
        add_input_bound(&mut bytes, avoid, std::mem::size_of::<AvoidRect>())?;
        bytes = bytes.checked_add(zone_filter)?;
        Some(bytes)
    }
}

fn add_input_bound(total: &mut usize, capacity: usize, element_bytes: usize) -> Option<()> {
    *total = total.checked_add(capacity.checked_mul(element_bytes)?)?;
    Some(())
}

/// Checked, allocation-free input and scratch/result reservation returned
/// before the request's target arrays are materialized.
///
/// This is reusable size metadata, not an issued work or ledger permit. Each
/// cursor construction still checks its pack, input shape, and future layout.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchPreflight {
    shape: SearchRequestShape,
    pack: crate::contract::PackId,
    input_bytes: usize,
    reservation: SearchReservation,
}

impl SearchPreflight {
    /// Exact boxed input bytes for the supplied shape capacities.
    pub const fn input_bytes(&self) -> usize {
        self.input_bytes
    }

    /// Static scratch, result, and future-layout bounds.
    pub const fn reservation(&self) -> SearchReservation {
        self.reservation
    }

    fn covers(&self, pack: crate::contract::PackId, actual: SearchRequestShape) -> bool {
        self.pack == pack && self.shape.matches_pack(pack) && self.shape.covers(actual)
    }
}

impl SearchRequest {
    /// Heap bytes owned by the request, excluding its inline enum storage.
    /// This is checked because admission must fail closed on arithmetic
    /// overflow rather than silently undercharge an input buffer.
    pub fn retained_input_bytes(&self) -> Option<usize> {
        self.preflight_shape()?.retained_input_bytes()
    }

    /// Exact input shape, including the route's owned compact storage.
    pub fn preflight_shape(&self) -> Option<SearchRequestShape> {
        SearchRequestShape::from_request(self)
    }

    pub const fn max_settles(&self) -> usize {
        match self {
            Self::Single { max_settles, .. }
            | Self::Many { max_settles, .. }
            | Self::First { max_settles, .. }
            | Self::MissingItemReqs { max_settles, .. }
            | Self::BlockingZones { max_settles, .. }
            | Self::UnresolvedQuestGates { max_settles, .. } => *max_settles,
            Self::MissingItemReqsForRoute { .. } => 0,
        }
    }
}

/// Phase-aware static reservation for one cooperative request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchReservation {
    /// Maximum simultaneous owned heap across search, reconstruction and
    /// result folding. Includes `result_bytes`; do not add that bound again.
    pub peak_bytes: usize,
    /// Maximum retained terminal payload, already covered by `peak_bytes`.
    pub result_bytes: usize,
    pub future_layout_bytes: usize,
    pub max_missing_requests: usize,
    pub max_missing_carry_ids: usize,
    pub max_missing_worn_ids: usize,
    /// Maximum unique item rows that a plan may retain, derived from the
    /// loaded graph's carry and worn rows and capped by the diagnosis bound.
    pub max_plan_item_rows: usize,
    pub max_quest_gates: usize,
    /// Informational subset of `peak_bytes`; do not add it a second time.
    pub zone_filter_bytes: usize,
}

/// Result of one cooperative search, transferred exactly once at completion.
pub enum SearchOutcome {
    Single {
        route: Result<CompactRoute, RouteError>,
    },
    Many {
        results: Box<[Result<TargetCost, TargetError>]>,
        best: Option<(usize, CompactRoute)>,
    },
    First {
        search: CompactFirstRouteSearch,
    },
    MissingItemReqs(Option<MissingRequirements>),
    /// Return the exact input route with its carry/wear diagnosis.
    MissingItemReqsForRoute {
        route: CompactRoute,
        missing: MissingRequirements,
    },
    BlockingZones(Option<Box<[ZoneKey]>>),
    UnresolvedQuestGates(Result<Option<Box<[QuestGate]>>, QuestFamilyMismatch>),
}

/// A borrowed view of one missing requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingReqView<'a> {
    /// The edge needs this carried stack.
    Carry { id: i32, count: i32 },
    /// The edge needs any one of these worn alternatives.
    WearAny { ids: &'a [i32] },
}

#[derive(Clone, Copy)]
enum MissingReqEntry {
    Carry { id: i32, count: i32 },
    WearAny { packed_edge: u32, order: u32 },
}

/// Missing route requirements backed by their immutable source edges.
///
/// Carry rows are copied scalar facts. A worn alternative is resolved from
/// its packed source-edge reference through `lease`; its ID slice is not
/// copied into the result.
pub struct MissingRequirements {
    entries: Vec<MissingReqEntry>,
    lease: NavLease,
}

impl MissingRequirements {
    fn new(entries: Vec<MissingReqEntry>, lease: NavLease) -> Self {
        Self { entries, lease }
    }

    /// Number of requirements, in the legacy stable sort order.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no missing requirements.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow one requirement and, for `WearAny`, its original ordered IDs.
    pub fn requirement_at(&self, index: usize) -> Option<MissingReqView<'_>> {
        match *self.entries.get(index)? {
            MissingReqEntry::Carry { id, count } => Some(MissingReqView::Carry { id, count }),
            MissingReqEntry::WearAny { packed_edge, .. } => {
                let edge = diagnostic_edge(self.lease.graph(), packed_edge)?;
                Some(MissingReqView::WearAny {
                    ids: &edge.worn_req,
                })
            }
        }
    }
    /// Checked bytes in the owned requirement-entry allocation.
    ///
    /// The immutable source graph held by the lease is charged separately.
    pub fn try_retained_bytes(&self) -> Option<usize> {
        self.entries
            .capacity()
            .checked_mul(std::mem::size_of::<MissingReqEntry>())
    }
}

impl std::fmt::Debug for MissingRequirements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut list = f.debug_list();
        for index in 0..self.entries.len() {
            match self.requirement_at(index) {
                Some(requirement) => {
                    list.entry(&requirement);
                }
                None => {
                    list.entry(&"<invalid source edge>");
                }
            }
        }
        list.finish()
    }
}

/// Compact counterpart of [`FallbackRoute`].
#[expect(
    clippy::large_enum_variant,
    reason = "a completed route transfers inline without another result allocation"
)]
#[derive(Debug)]
pub enum CompactFallbackRoute {
    Routed(CompactRoute),
    Failed(RouteError),
    Undecided,
}

/// Compact preferred/fallback answer of a proof-carrying first-goal search.
pub struct CompactFirstRouteSearch {
    preferred: Result<CompactRoute, RouteError>,
    fallback: Option<CompactFallbackRoute>,
    settled: usize,
    capacities: SearchCapacities,
    proof: ReverseProof,
    completion_partitions: usize,
}

impl CompactFirstRouteSearch {
    /// Checked maximum bytes for this route-family result at `max_settles`.
    /// A successful preferred answer suppresses the fallback route, so at
    /// most one compact route backing is retained.
    pub fn storage_upper_bound(max_settles: usize) -> Option<usize> {
        std::mem::size_of::<Self>().checked_add(compact_route_backing_bytes(max_settles)?)
    }

    /// The preferred route or its domain failure.
    pub fn route(&self) -> Result<&CompactRoute, RouteError> {
        self.preferred.as_ref().map_err(|error| *error)
    }

    /// The fallback answer, if the search had a fallback goal set.
    pub fn fallback(&self) -> Option<&CompactFallbackRoute> {
        self.fallback.as_ref()
    }

    /// Transfer both route answers without cloning either compact result.
    pub fn into_routes(
        self,
    ) -> (
        Result<CompactRoute, RouteError>,
        Option<CompactFallbackRoute>,
    ) {
        (self.preferred, self.fallback)
    }

    /// Forward nodes settled; reverse proof nodes are not included.
    pub fn settled(&self) -> usize {
        self.settled
    }

    /// Peak scratch entry capacities at the point the search stopped.
    pub fn scratch_capacities(&self) -> SearchCapacities {
        self.capacities
    }

    /// The preferred goals' backward proof.
    pub fn proof(&self) -> ReverseProof {
        self.proof
    }

    /// Completion searches grouped by the selected endpoint's zone set.
    pub fn completion_partitions(&self) -> usize {
        self.completion_partitions
    }
}

/// Completed request result plus the shared world-state snapshot.
pub struct SearchCompletion {
    outcome: SearchOutcome,
    state: Arc<WorldState>,
    forward_settled: usize,
    pack: PackId,
}

impl SearchCompletion {
    /// Total forward-search labels settled across every request pass.
    pub const fn forward_settled(&self) -> usize {
        self.forward_settled
    }
    /// Borrow a view of a successful route while this completion owns it.
    ///
    /// `lease` must identify the same immutable pack used by the search. The
    /// view borrows compact storage and resolves its edge indexes through that
    /// lease without allocating or publishing a plan.
    pub fn route_view<'a>(
        &'a self,
        lease: &'a NavLease,
    ) -> Result<Option<RouteView<'a>>, ContractError> {
        if self.pack != lease.identity().pack() {
            return Err(ContractError::PackMismatch);
        }
        let route = match &self.outcome {
            SearchOutcome::Single { route: Ok(route) }
            | SearchOutcome::MissingItemReqsForRoute { route, .. } => Some(route),
            SearchOutcome::Many {
                best: Some((_, route)),
                ..
            } => Some(route),
            SearchOutcome::First { search } => {
                search.route().ok().or_else(|| match search.fallback() {
                    Some(CompactFallbackRoute::Routed(route)) => Some(route),
                    _ => None,
                })
            }
            _ => None,
        };
        Ok(route.map(|route| RouteView::borrowed_compact(self.pack, route, lease.graph())))
    }
    /// Transfer the outcome and shared state handle back to the owning job.
    pub fn into_parts(self) -> (SearchOutcome, Arc<WorldState>) {
        (self.outcome, self.state)
    }
}

/// Cooperative cursor step. `Complete` transfers the sole result owner.
#[expect(
    clippy::large_enum_variant,
    reason = "each cursor checkpoint returns inline without a completion allocation"
)]
pub enum CursorStep {
    Complete(SearchCompletion),
    Yielded,
    Cancelled(Cancellation),
    Failed(SearchCursorError),
}

/// Search cursor construction or execution failure.
#[derive(Debug, thiserror::Error)]
pub enum SearchCursorError {
    #[error("search request retained-byte arithmetic overflowed")]
    RetainedBytesOverflow,
    #[error("search request exceeds the compact cursor's representable settle cap")]
    SettleCapTooLarge,
    #[error("search cursor allocation failed")]
    Allocation,
    #[error("search cursor has already completed")]
    AlreadyCompleted,
    #[error("search result exceeds the compact cursor's representable layout")]
    ResultNotRepresentable,
    #[error("search request does not fit the preflighted reservation")]
    PreflightMismatch,
}

type SearchFuture =
    Pin<Box<dyn Future<Output = Result<SearchCompletion, SearchCursorError>> + Send + 'static>>;

/// Owned, resumable search state. The cursor owns only a cloned handle to the
/// caller's shared world state; callers charge that backing once themselves.
pub struct SearchCursor {
    future: Option<SearchFuture>,
    progress: Arc<AtomicUsize>,
    input_bytes: usize,
    reservation: SearchReservation,
}

impl SearchCursor {
    /// Construct the owned request future after calculating its full static
    /// reservation without allocating a search cursor or zone filter.
    pub fn new(
        request: SearchRequest,
        lease: NavLease,
        state: Arc<WorldState>,
    ) -> Result<Self, SearchCursorError> {
        if request.max_settles() > NODE_BUDGET {
            return Err(SearchCursorError::SettleCapTooLarge);
        }
        let shape = SearchRequestShape::from_request(&request)
            .ok_or(SearchCursorError::RetainedBytesOverflow)?;
        let preflight = Self::preflight(shape, &lease, &state)
            .ok_or(SearchCursorError::RetainedBytesOverflow)?;
        Self::new_preflighted(request, lease, state, preflight)
    }

    /// Construct a cursor only when the materialized inputs fit the
    /// admission-time shape and the lease that produced it.
    pub fn new_preflighted(
        request: SearchRequest,
        lease: NavLease,
        state: Arc<WorldState>,
        preflight: SearchPreflight,
    ) -> Result<Self, SearchCursorError> {
        if request.max_settles() > NODE_BUDGET {
            return Err(SearchCursorError::SettleCapTooLarge);
        }
        let shape = SearchRequestShape::from_request(&request)
            .ok_or(SearchCursorError::RetainedBytesOverflow)?;
        if !preflight.covers(lease.identity().pack(), shape) {
            return Err(SearchCursorError::PreflightMismatch);
        }
        let input_bytes = shape
            .retained_input_bytes()
            .ok_or(SearchCursorError::RetainedBytesOverflow)?;
        if input_bytes > preflight.input_bytes {
            return Err(SearchCursorError::PreflightMismatch);
        }
        let reservation = preflight.reservation;
        let progress = Arc::new(AtomicUsize::new(0));
        let future: SearchFuture = Box::pin(run_search_request(
            request,
            lease,
            state,
            reservation,
            Some(Arc::clone(&progress)),
        ));
        if std::mem::size_of_val(future.as_ref().get_ref()) != reservation.future_layout_bytes {
            return Err(SearchCursorError::ResultNotRepresentable);
        }
        Ok(Self {
            future: Some(future),
            progress,
            input_bytes,
            reservation,
        })
    }

    /// Check exact shape capacities and all static charges without
    /// materializing request inputs or allocating a cursor.
    pub fn preflight(
        shape: SearchRequestShape,
        lease: &NavLease,
        state: &Arc<WorldState>,
    ) -> Option<SearchPreflight> {
        if !shape.is_consistent() || shape.max_settles() > NODE_BUDGET {
            return None;
        }
        if !shape.matches_pack(lease.identity().pack()) {
            return None;
        }
        let input_bytes = shape.retained_input_bytes()?;
        let envelope = lease.search_envelope();
        if !envelope.is_representable() {
            return None;
        }
        let mut reservation = search_worst_case_reservation(shape, lease.graph(), envelope)?;
        reservation.future_layout_bytes =
            search_future_layout_bytes(shape.max_settles(), lease, state)?;
        Some(SearchPreflight {
            shape,
            pack: lease.identity().pack(),
            input_bytes,
            reservation,
        })
    }

    /// Measure the concrete request-future layout with borrowed shared handles
    /// and empty request slices. The result is independent of request values.
    pub fn future_layout_bytes_for(
        request: &SearchRequest,
        lease: &NavLease,
        state: &Arc<WorldState>,
    ) -> Option<usize> {
        search_future_layout_bytes(request.max_settles(), lease, state)
    }

    /// Allocation-free checked upper bound for scratch, future, and result
    /// custody.
    pub fn worst_case_reservation(
        request: &SearchRequest,
        lease: &NavLease,
        state: &Arc<WorldState>,
    ) -> Option<SearchReservation> {
        let shape = SearchRequestShape::from_request(request)?;
        Self::preflight(shape, lease, state).map(|preflight| preflight.reservation())
    }

    /// Heap bytes owned by the request's boxed inputs.
    pub const fn input_bytes(&self) -> usize {
        self.input_bytes
    }

    /// Maximum simultaneously retained algorithm and result heap.
    pub const fn retained_bytes(&self) -> usize {
        self.reservation.peak_bytes
    }

    /// Concrete allocation size of the pinned future (its Arc/NavLease
    /// handles and inline request storage are included).
    pub const fn future_layout_bytes(&self) -> usize {
        self.reservation.future_layout_bytes
    }

    /// Static result and scratch bounds, including output-specific limits.
    pub const fn reservation(&self) -> SearchReservation {
        self.reservation
    }

    /// Cumulative forward-search settlements completed so far.
    ///
    /// This count excludes scratch initialization, proof preparation, and
    /// cooperative work units that do not settle a search label.
    pub fn forward_settled(&self) -> usize {
        self.progress.load(AtomicOrdering::Relaxed)
    }

    /// Advance only at work-unit boundaries. `ctx` is never stored in the
    /// future, so a returned cursor holds no slice-budget borrow.
    pub fn advance<P: WorkPhase>(&mut self, ctx: &mut StepContext<'_, P>) -> CursorStep {
        let mut context = Context::from_waker(Waker::noop());
        let mut checkpoint = ctx.checkpoint();
        loop {
            match checkpoint {
                Checkpoint::Continue => {}
                Checkpoint::Yield => return CursorStep::Yielded,
                Checkpoint::Cancel(reason) => {
                    self.future.take();
                    return CursorStep::Cancelled(reason);
                }
            }
            let polled = match self.future.as_mut() {
                Some(future) => future.as_mut().poll(&mut context),
                None => return CursorStep::Failed(SearchCursorError::AlreadyCompleted),
            };
            match polled {
                Poll::Ready(Ok(completion)) => {
                    self.future.take();
                    return CursorStep::Complete(completion);
                }
                Poll::Ready(Err(error)) => {
                    self.future.take();
                    return CursorStep::Failed(error);
                }
                Poll::Pending => checkpoint = ctx.consume_unit(),
            }
        }
    }
}

fn search_future_layout_bytes(
    max_settles: usize,
    lease: &NavLease,
    state: &Arc<WorldState>,
) -> Option<usize> {
    if max_settles > NODE_BUDGET {
        return None;
    }
    let dummy_request = SearchRequest::Single {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        options: FindOptions::default(),
        avoid: Box::default(),
        max_settles: 0,
    };
    let future = run_search_request(
        dummy_request,
        lease.clone(),
        Arc::clone(state),
        SearchReservation::default(),
        None,
    );
    Some(std::mem::size_of_val(&future))
}

fn add_reserved(total: &mut usize, bytes: usize) -> Option<()> {
    *total = total.checked_add(bytes)?;
    Some(())
}

fn vector_storage_bound<T>(capacity: usize) -> Option<usize> {
    if capacity == 0 {
        return Some(0);
    }
    capacity
        .checked_next_power_of_two()?
        .checked_mul(std::mem::size_of::<T>())
}

fn hash_storage_bound<K, V>(capacity: usize) -> Option<usize> {
    crate::world_state::map_storage_bound(
        capacity,
        std::mem::size_of::<(K, V)>(),
        std::mem::align_of::<(K, V)>(),
    )
}

fn hash_storage_peak<K, V>(capacity: usize) -> Option<usize> {
    hash_storage_bound::<K, V>(capacity)?.checked_mul(2)
}

fn set_storage_peak<K>(capacity: usize) -> Option<usize> {
    crate::world_state::map_storage_bound(
        capacity,
        std::mem::size_of::<K>(),
        std::mem::align_of::<K>(),
    )?
    .checked_mul(2)
}

fn compact_route_backing_bytes(settles: usize) -> Option<usize> {
    let transitions = settles;
    let walk_tiles = transitions.checked_add(1)?;
    let legs = transitions.checked_mul(2)?.checked_add(1)?;
    let checkpoints = legs.div_ceil(64);
    walk_tiles
        .checked_mul(std::mem::size_of::<WorldTile>())?
        .checked_add(transitions.checked_add(3)?.checked_div(4)?)?
        .checked_add(transitions.checked_mul(std::mem::size_of::<u32>())?)?
        .checked_add(checkpoints.checked_mul(std::mem::size_of::<CompactCheckpoint>())?)
}

fn route_result_bytes(settles: usize) -> Option<usize> {
    std::mem::size_of::<CompactRoute>().checked_add(compact_route_backing_bytes(settles)?)
}

fn route_missing_requirements_reservation(
    route_bytes: usize,
    edge_capacity: usize,
    envelope: crate::contract::SearchGraphEnvelope,
) -> Option<SearchReservation> {
    if edge_capacity > NODE_BUDGET {
        return None;
    }
    let max_missing_requests = envelope.carry_rows.checked_add(edge_capacity)?;
    let max_plan_item_rows = envelope
        .carry_rows
        .checked_add(envelope.worn_rows)?
        .min(max_missing_requests);
    let max_missing_worn_ids = edge_capacity.checked_mul(envelope.max_worn_ids_per_edge)?;
    let progress_counter_bytes = std::mem::size_of::<Arc<AtomicUsize>>()
        .checked_add(std::mem::size_of::<AtomicUsize>().checked_mul(3)?)?;
    let missing_sort_scratch =
        vector_storage_bound::<usize>(max_missing_requests)?.checked_mul(2)?;
    let mut scratch_bytes = 0usize;
    for bytes in [
        hash_storage_peak::<i32, i32>(envelope.carry_rows)?,
        missing_sort_scratch,
        progress_counter_bytes,
    ] {
        add_reserved(&mut scratch_bytes, bytes)?;
    }

    let missing_items = vector_storage_bound::<MissingReqEntry>(max_missing_requests)?;
    let mut result_bytes =
        std::mem::size_of::<SearchOutcome>().checked_add(std::mem::size_of::<SearchCompletion>())?;
    add_reserved(&mut result_bytes, missing_items)?;
    add_reserved(&mut result_bytes, route_bytes)?;
    Some(SearchReservation {
        peak_bytes: scratch_bytes.checked_add(result_bytes)?,
        result_bytes,
        future_layout_bytes: 0,
        max_missing_requests,
        max_missing_carry_ids: envelope.carry_rows,
        max_missing_worn_ids,
        max_plan_item_rows,
        max_quest_gates: 0,
        zone_filter_bytes: 0,
    })
}

/// Allocation-free static bound for a search cursor's scratch and result bytes.
fn search_worst_case_reservation(
    shape: SearchRequestShape,
    graph: &TransportGraph,
    envelope: crate::contract::SearchGraphEnvelope,
) -> Option<SearchReservation> {
    let settles = shape.max_settles();
    if settles > NODE_BUDGET {
        return None;
    }
    if let SearchRequestShape::MissingItemReqsForRoute {
        route_bytes,
        edge_capacity,
        ..
    } = shape
    {
        return route_missing_requirements_reservation(route_bytes, edge_capacity, envelope);
    }

    let options = shape.options();
    let carry_rows = envelope.carry_rows;
    let return_rows = envelope.return_rows;
    let max_worn_ids_per_edge = envelope.max_worn_ids_per_edge;
    let max_quest_gates = if matches!(shape, SearchRequestShape::UnresolvedQuestGates { .. }) {
        envelope.max_quest_gates
    } else {
        0
    };
    let (max_missing_requests, max_missing_carry_ids, max_missing_worn_ids, max_plan_item_rows) =
        if matches!(shape, SearchRequestShape::MissingItemReqs { .. }) {
            let max_missing_requests = carry_rows.checked_add(settles)?;
            (
                max_missing_requests,
                carry_rows,
                settles.checked_mul(max_worn_ids_per_edge)?,
                carry_rows
                    .checked_add(envelope.worn_rows)?
                    .min(max_missing_requests),
            )
        } else {
            (0, 0, 0, 0)
        };

    let teleport_edges = if options.allow_teleports {
        graph.teleports.len()
    } else {
        0
    };
    let target_count = shape.target_count()?;
    let zone_count = graph.zones.as_ref().map_or(0, |table| table.zones().len());

    let mut zone_filter_bytes = 0usize;
    if graph.zones.is_some()
        && (!matches!(shape, SearchRequestShape::BlockingZones { .. }) || !options.zones.is_all())
    {
        let one_filter = ZoneFilterOwned::worst_case_bytes(zone_count)?;
        zone_filter_bytes = one_filter.checked_mul(2)?;
    }
    let zone_partition_bytes = if graph.zones.is_some()
        && matches!(
            shape,
            SearchRequestShape::Many { .. }
                | SearchRequestShape::First { .. }
                | SearchRequestShape::MissingItemReqs { .. }
                | SearchRequestShape::BlockingZones { .. }
                | SearchRequestShape::UnresolvedQuestGates { .. }
        ) {
        // Each partition and its goal/index arrays reserve their exact input
        // capacities once. A key can contain only IDs from one spatial bucket.
        let row_bytes = match shape {
            SearchRequestShape::Many { .. } => std::mem::size_of::<IndexedZoneGoal>(),
            SearchRequestShape::BlockingZones { .. } => {
                std::mem::size_of::<(Vec<u16>, usize, WorldTile)>()
            }
            _ => std::mem::size_of::<FirstZoneGoal>(),
        };
        let goal_bytes = std::mem::size_of::<WorldTile>().checked_add(
            if matches!(shape, SearchRequestShape::Many { .. }) {
                std::mem::size_of::<usize>()
            } else {
                0
            },
        )?;
        let zone_ids = graph.zones.as_ref()?.max_zones_per_tile();
        target_count.checked_mul(
            row_bytes
                .checked_add(zone_ids.checked_mul(std::mem::size_of::<u16>())?)?
                .checked_add(goal_bytes)?,
        )?
    } else {
        0
    };
    // FirstTargets reserves each set once, without growth. Small sets borrow
    // the input slice. Preferred and fallback capacities are separate.
    let target_set_bytes = |capacity| {
        if capacity <= 8 {
            Some(0)
        } else {
            hash_storage_bound::<WorldTile, ()>(capacity)
        }
    };
    let first_target_sets = match shape {
        SearchRequestShape::First {
            target_capacity,
            fallback_capacity,
            ..
        }
        | SearchRequestShape::MissingItemReqs {
            target_capacity,
            fallback_capacity,
            ..
        } => target_set_bytes(target_capacity)?
            .checked_add(target_set_bytes(fallback_capacity)?)?
            .checked_add(std::mem::size_of::<FirstGoals<'_>>())?,
        SearchRequestShape::BlockingZones { .. }
        | SearchRequestShape::UnresolvedQuestGates { .. } => {
            target_set_bytes(target_count)?.checked_add(std::mem::size_of::<FirstGoals<'_>>())?
        }
        _ => 0,
    };

    let missing_sort_scratch = if matches!(shape, SearchRequestShape::MissingItemReqs { .. }) {
        vector_storage_bound::<usize>(max_missing_requests)?.checked_mul(2)?
    } else {
        0
    };

    let blocking_key_scratch = if matches!(shape, SearchRequestShape::BlockingZones { .. }) {
        vector_storage_bound::<ZoneKey>(zone_count)?
    } else {
        0
    };
    let progress_counter_bytes = std::mem::size_of::<Arc<AtomicUsize>>()
        .checked_add(std::mem::size_of::<AtomicUsize>().checked_mul(3)?)?;
    // RouteTrace counts its path first, then reserves exactly that many u32s.
    let trace_bytes = settles.checked_mul(std::mem::size_of::<u32>())?;
    let retained_candidates = match shape {
        SearchRequestShape::Single { .. }
        | SearchRequestShape::Many {
            reconstruct_best: false,
            ..
        }
        | SearchRequestShape::BlockingZones {
            diagnosis: BlockerDiagnosis::SingleTarget,
            ..
        } => 0usize,
        // A successful preferred candidate discards the fallback immediately.
        // Each partition therefore retains only one previous action trace.
        _ => 1,
    };
    // These buffers may live across zone partitions and across the final
    // materialization. Unlike the heap and resource model, they are not
    // released when search_kernel returns.
    let mut common_bytes = 0usize;
    for bytes in [
        // The metered reached map reserves once and stores only balances.
        hash_storage_bound::<WorldTile, u32>(target_count)?,
        zone_filter_bytes,
        progress_counter_bytes,
        zone_partition_bytes,
        first_target_sets,
        blocking_key_scratch,
        set_storage_peak::<u16>(zone_count)?,
    ] {
        add_reserved(&mut common_bytes, bytes)?;
    }
    // Only Many builds a unique-goal index and per-target cost/result arrays.
    if matches!(shape, SearchRequestShape::Many { .. }) {
        for bytes in [
            hash_storage_peak::<WorldTile, usize>(target_count)?,
            vector_storage_bound::<usize>(target_count)?.checked_mul(2)?,
            vector_storage_bound::<Result<TargetCost, TargetError>>(target_count)?
                .checked_mul(3)?,
        ] {
            add_reserved(&mut common_bytes, bytes)?;
        }
    }
    let mut resource_bytes = 0usize;
    for bytes in [
        hash_storage_peak::<i32, i64>(carry_rows)?,
        hash_storage_peak::<i32, i32>(carry_rows)?,
        vector_storage_bound::<i32>(carry_rows)?,
        set_storage_peak::<i32>(return_rows)?,
        vector_storage_bound::<i32>(resources::RESOURCE_LIMIT)?,
        vector_storage_bound::<[i32; resources::RESOURCE_LIMIT]>(resources::BALANCE_LIMIT)?
            .checked_mul(2)?,
        hash_storage_peak::<[i32; resources::RESOURCE_LIMIT], u32>(resources::BALANCE_LIMIT)?,
    ] {
        add_reserved(&mut resource_bytes, bytes)?;
    }
    let reverse_entries = REVERSE_PROOF_BUDGET.checked_add(1)?;
    // ReverseClosure reserves its complete seen/queue capacities once. At
    // most one preferred/fallback proof is live, and finish_proofs drops it
    // before predecessor reconstruction.
    let reverse_bytes = crate::world_state::map_storage_bound(
        reverse_entries,
        std::mem::size_of::<WorldTile>(),
        std::mem::align_of::<WorldTile>(),
    )?
    .checked_add(vector_storage_bound::<WorldTile>(reverse_entries)?)?
    .checked_add(set_storage_peak::<WorldTile>(teleport_edges)?)?;
    let search_phase = common_bytes
        .checked_add(resources::packed_search_storage_bound(settles)?)?
        .checked_add(resource_bytes)?
        .checked_add(reverse_bytes)?
        .checked_add(vector_storage_bound::<usize>(teleport_edges)?)?
        .checked_add(trace_bytes.checked_mul(retained_candidates)?)?;
    // The heap, resource model and reverse proof are gone here. Existing
    // partition candidates coexist only with the predecessor table and one
    // newly reconstructed action trace.
    let backtrace_phase = common_bytes
        .checked_add(resources::packed_predecessor_storage_bound(settles)?)?
        .checked_add(trace_bytes.checked_mul(retained_candidates.checked_add(1)?)?)?;

    let mut result_bytes =
        std::mem::size_of::<SearchOutcome>().checked_add(std::mem::size_of::<SearchCompletion>())?;
    match shape {
        SearchRequestShape::Single { .. } => {
            add_reserved(&mut result_bytes, route_result_bytes(settles)?)?;
        }
        SearchRequestShape::Many {
            reconstruct_best, ..
        } => {
            add_reserved(
                &mut result_bytes,
                target_count.checked_mul(std::mem::size_of::<Result<TargetCost, TargetError>>())?,
            )?;
            if reconstruct_best {
                add_reserved(&mut result_bytes, route_result_bytes(settles)?)?;
            }
        }
        SearchRequestShape::First { .. } => {
            add_reserved(
                &mut result_bytes,
                CompactFirstRouteSearch::storage_upper_bound(settles)?,
            )?;
        }
        SearchRequestShape::MissingItemReqs { .. } => {
            let missing_items = vector_storage_bound::<MissingReqEntry>(max_missing_requests)?;
            add_reserved(&mut result_bytes, missing_items)?;
        }
        SearchRequestShape::BlockingZones { .. } => {
            add_reserved(
                &mut result_bytes,
                zone_count.checked_mul(std::mem::size_of::<ZoneKey>())?,
            )?;
        }
        SearchRequestShape::UnresolvedQuestGates { .. } => {
            add_reserved(
                &mut result_bytes,
                vector_storage_bound::<QuestGate>(max_quest_gates)?,
            )?;
        }
        SearchRequestShape::MissingItemReqsForRoute { .. } => {
            unreachable!("route-only diagnostics return before search reservation")
        }
    }
    // Single-target search and SingleTarget blocker diagnosis materialize
    // their compact route directly from the one live predecessor table.
    // First-goal searches drop partition tables before route materialization.
    let mut materialization_phase = common_bytes
        .checked_add(result_bytes)?
        .checked_add(trace_bytes.checked_mul(retained_candidates)?)?;
    match shape {
        SearchRequestShape::Single { .. } => {
            add_reserved(
                &mut materialization_phase,
                resources::packed_predecessor_storage_bound(settles)?,
            )?;
        }
        SearchRequestShape::MissingItemReqs { .. } => {
            add_reserved(&mut materialization_phase, missing_sort_scratch)?;
            add_reserved(
                &mut materialization_phase,
                hash_storage_peak::<i32, i32>(carry_rows)?,
            )?;
        }
        SearchRequestShape::BlockingZones {
            diagnosis: BlockerDiagnosis::SingleTarget,
            ..
        } => {
            add_reserved(
                &mut materialization_phase,
                resources::packed_predecessor_storage_bound(settles)?,
            )?;
            add_reserved(&mut materialization_phase, route_result_bytes(settles)?)?;
        }
        SearchRequestShape::BlockingZones { .. }
        | SearchRequestShape::UnresolvedQuestGates { .. } => {
            add_reserved(&mut materialization_phase, route_result_bytes(settles)?)?;
        }
        SearchRequestShape::Many { .. } | SearchRequestShape::First { .. } => {}
        SearchRequestShape::MissingItemReqsForRoute { .. } => {
            unreachable!("route-only diagnostics return before search reservation")
        }
    }
    Some(SearchReservation {
        peak_bytes: search_phase.max(backtrace_phase).max(materialization_phase),
        result_bytes,
        future_layout_bytes: 0,
        max_missing_requests,
        max_missing_carry_ids,
        max_missing_worn_ids,
        max_plan_item_rows,
        max_quest_gates,
        zone_filter_bytes,
    })
}
/// A partition candidate keeps only predecessor actions, not expanded walk
/// slices. This permits the next full-budget search to coexist with the
/// candidate; only the final winner materializes the public compact route.
/// The exact i32 origin and original graph landings preserve every coordinate.
struct RouteTrace {
    origin: WorldTile,
    destination: WorldTile,
    ticks: f64,
    steps: Vec<u32>,
    essence: Option<EssenceSession>,
}

impl RouteTrace {
    async fn from_predecessors(
        origin: WorldTile,
        destination: WorldTile,
        ticks: f64,
        came_from: &Predecessors,
        essence: Option<EssenceSession>,
    ) -> Result<Self, SearchCursorError> {
        let mut trace = came_from
            .backtrace(destination)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let mut count = 0usize;
        while trace.next().is_some() {
            work_unit().await;
            count = count
                .checked_add(1)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
        }
        if trace.current_tile() != origin {
            return Err(SearchCursorError::ResultNotRepresentable);
        }
        let mut steps = Vec::new();
        steps
            .try_reserve_exact(count)
            .map_err(|_| SearchCursorError::Allocation)?;
        let mut trace = came_from
            .backtrace(destination)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        while let Some(step) = trace.next() {
            work_unit().await;
            let (tag, value) = match step {
                BacktraceStep::Walk { tile } => {
                    let parent = trace.current_tile();
                    let direction = STEPS
                        .iter()
                        .position(|&(dx, dz)| {
                            parent.level == tile.level
                                && parent.x.checked_add(dx) == Some(tile.x)
                                && parent.z.checked_add(dz) == Some(tile.z)
                        })
                        .ok_or(SearchCursorError::ResultNotRepresentable)?;
                    (0u32, direction)
                }
                BacktraceStep::Transport { index, .. } => (1, index),
                BacktraceStep::Teleport { index, .. } => (2, index),
                BacktraceStep::EssenceReturn { portal, .. } => (3, portal),
            };
            let value =
                u32::try_from(value).map_err(|_| SearchCursorError::ResultNotRepresentable)?;
            if value >= 1 << 30 {
                return Err(SearchCursorError::ResultNotRepresentable);
            }
            steps.push((tag << 30) | value);
        }
        if steps.len() != count || trace.current_tile() != origin {
            return Err(SearchCursorError::ResultNotRepresentable);
        }
        reverse_cooperative(&mut steps).await;
        Ok(Self {
            origin,
            destination,
            ticks,
            steps,
            essence,
        })
    }

    async fn materialize(self, graph: &TransportGraph) -> Result<CompactRoute, SearchCursorError> {
        let transitions = self.steps.len();
        let mut edge_count = 0usize;
        for &step in &self.steps {
            work_unit().await;
            if step >> 30 != 0 {
                edge_count += 1;
            }
        }
        let walk_capacity = transitions
            .checked_add(1)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let tag_capacity = transitions
            .checked_add(3)
            .ok_or(SearchCursorError::ResultNotRepresentable)?
            / 4;
        let transition_count =
            u32::try_from(transitions).map_err(|_| SearchCursorError::ResultNotRepresentable)?;
        let leg_count = edge_count
            .checked_mul(2)
            .and_then(|count| count.checked_add(1))
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let mut walk_tiles = Vec::new();
        walk_tiles
            .try_reserve_exact(walk_capacity)
            .map_err(|_| SearchCursorError::Allocation)?;
        let mut tags = Vec::new();
        tags.try_reserve_exact(tag_capacity)
            .map_err(|_| SearchCursorError::Allocation)?;
        for _ in 0..tag_capacity {
            work_unit().await;
            tags.push(0);
        }
        let mut edge_ids = Vec::new();
        edge_ids
            .try_reserve_exact(edge_count)
            .map_err(|_| SearchCursorError::Allocation)?;
        let mut tile = self.origin;
        walk_tiles.push(tile);
        for (index, &step) in self.steps.iter().enumerate() {
            work_unit().await;
            let tag = (step >> 30) as u8;
            let value = (step & ((1 << 30) - 1)) as usize;
            tile = match tag {
                0 => {
                    let &(dx, dz) = STEPS
                        .get(value)
                        .ok_or(SearchCursorError::ResultNotRepresentable)?;
                    WorldTile {
                        x: tile
                            .x
                            .checked_add(dx)
                            .ok_or(SearchCursorError::ResultNotRepresentable)?,
                        z: tile
                            .z
                            .checked_add(dz)
                            .ok_or(SearchCursorError::ResultNotRepresentable)?,
                        level: tile.level,
                    }
                }
                1 | 2 => {
                    let edge = if tag == 1 {
                        graph.edges.get(value)
                    } else {
                        graph.teleports.get(value)
                    }
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                    edge.landing_from(tile)
                        .ok_or(SearchCursorError::ResultNotRepresentable)?
                }
                _ => {
                    ESSENCE_MINE_PORTALS
                        .get(value)
                        .ok_or(SearchCursorError::ResultNotRepresentable)?;
                    self.essence
                        .as_ref()
                        .ok_or(SearchCursorError::ResultNotRepresentable)?
                        .return_tile
                }
            };
            walk_tiles.push(tile);
            tags[index >> 2] |= tag << ((index & 3) * 2);
            if tag != 0 {
                edge_ids.push(step & ((1 << 30) - 1));
            }
        }
        if tile != self.destination {
            return Err(SearchCursorError::ResultNotRepresentable);
        }
        let mut route = CompactRoute::from_compact_parts(
            self.origin,
            self.destination,
            self.ticks,
            walk_tiles.into_boxed_slice(),
            tags.into_boxed_slice(),
            transition_count,
            leg_count,
            edge_ids.into_boxed_slice(),
            self.essence,
            Box::default(),
        );
        route.build_checkpoints(graph).await?;
        Ok(route)
    }
}

async fn compact_route_from_predecessors(
    origin: WorldTile,
    destination: WorldTile,
    ticks: f64,
    came_from: &Predecessors,
    graph: &TransportGraph,
    essence: Option<EssenceSession>,
) -> Result<CompactRoute, SearchCursorError> {
    let mut trace = came_from
        .backtrace(destination)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let mut transitions = 0usize;
    let mut edge_count = 0usize;
    while let Some(step) = trace.next() {
        work_unit().await;
        transitions = transitions
            .checked_add(1)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        if !matches!(step, BacktraceStep::Walk { .. }) {
            edge_count = edge_count
                .checked_add(1)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
        }
    }
    if trace.current_tile() != origin {
        return Err(SearchCursorError::ResultNotRepresentable);
    }

    let walk_capacity = transitions
        .checked_add(1)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let tag_capacity = transitions
        .checked_add(3)
        .ok_or(SearchCursorError::ResultNotRepresentable)?
        .checked_div(4)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let transition_count =
        u32::try_from(transitions).map_err(|_| SearchCursorError::ResultNotRepresentable)?;
    let leg_count = edge_count
        .checked_mul(2)
        .and_then(|count| count.checked_add(1))
        .and_then(|count| u32::try_from(count).ok())
        .ok_or(SearchCursorError::ResultNotRepresentable)?;

    let mut walk_tiles = Vec::new();
    walk_tiles
        .try_reserve_exact(walk_capacity)
        .map_err(|_| SearchCursorError::Allocation)?;
    let mut tags = Vec::new();
    tags.try_reserve_exact(tag_capacity)
        .map_err(|_| SearchCursorError::Allocation)?;
    for _ in 0..tag_capacity {
        work_unit().await;
        tags.push(0);
    }
    let mut edge_ids = Vec::new();
    edge_ids
        .try_reserve_exact(edge_count)
        .map_err(|_| SearchCursorError::Allocation)?;

    let mut trace = came_from
        .backtrace(destination)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let mut reverse_index = 0usize;
    while let Some(step) = trace.next() {
        work_unit().await;
        let (tile, tag, edge) = match step {
            BacktraceStep::Walk { tile } => (tile, 0u8, None),
            BacktraceStep::Transport { tile, index } => (
                tile,
                1,
                Some(u32::try_from(index).map_err(|_| SearchCursorError::ResultNotRepresentable)?),
            ),
            BacktraceStep::Teleport { tile, index } => (
                tile,
                2,
                Some(u32::try_from(index).map_err(|_| SearchCursorError::ResultNotRepresentable)?),
            ),
            BacktraceStep::EssenceReturn { tile, portal } => (
                tile,
                3,
                Some(u32::try_from(portal).map_err(|_| SearchCursorError::ResultNotRepresentable)?),
            ),
        };
        walk_tiles.push(tile);
        if let Some(edge) = edge {
            edge_ids.push(edge);
        }
        let forward_index = transitions
            .checked_sub(reverse_index + 1)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let tag_byte = tags
            .get_mut(forward_index >> 2)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        *tag_byte |= tag << ((forward_index & 3) * 2);
        reverse_index = reverse_index
            .checked_add(1)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
    }
    if reverse_index != transitions || trace.current_tile() != origin {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    walk_tiles.push(origin);
    reverse_cooperative(&mut walk_tiles).await;
    reverse_cooperative(&mut edge_ids).await;

    let mut route = CompactRoute::from_compact_parts(
        origin,
        destination,
        ticks,
        walk_tiles.into_boxed_slice(),
        tags.into_boxed_slice(),
        transition_count,
        leg_count,
        edge_ids.into_boxed_slice(),
        essence,
        Box::default(),
    );
    route.build_checkpoints(graph).await?;
    Ok(route)
}
#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative single search borrows the unchanged collision, graph and search inputs"
)]
async fn run_single_compact_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    zones: Option<&ZoneFilter<'_>>,
    deadline: Option<Instant>,
    progress: Option<&AtomicUsize>,
) -> Result<(Result<CompactRoute, RouteError>, SearchCapacities, usize), SearchCursorError> {
    if from == to {
        let route = compact_route_from_predecessors(
            from,
            to,
            0.0,
            &Predecessors::empty(),
            graph,
            options.essence,
        )
        .await?;
        return Ok((Ok(route), SearchCapacities::default(), 0));
    }
    let mut goals = Goals::Single { to, cost: None };
    let search = search_kernel(
        collision,
        graph,
        from,
        CostModel::running(),
        budget,
        options.allow_teleports,
        options.allow_wilderness,
        state,
        options.essence.as_ref(),
        Relax::Strict,
        avoid,
        zones,
        &mut goals,
        deadline,
        true,
        progress,
    )
    .await;
    if search.stop == SearchStop::Allocation {
        return Err(SearchCursorError::Allocation);
    }
    let route = match goals {
        Goals::Single {
            cost: Some(cost), ..
        } => Ok(compact_route_from_predecessors(
            from,
            to,
            cost.ticks,
            &search.came_from,
            graph,
            options.essence,
        )
        .await?),
        _ if search.stop == SearchStop::Budget => Err(RouteError::BudgetExhausted),
        _ => Err(RouteError::NoPath),
    };
    Ok((route, search.capacities, search.settled))
}

async fn new_owned_zone_filter(
    graph: &TransportGraph,
    state: &WorldState,
    endpoints: &[WorldTile],
    exemptions: ZoneExempt,
) -> Result<Option<ZoneFilterOwned>, SearchCursorError> {
    if exemptions.is_all() {
        return Ok(None);
    }
    let Some(table) = graph.zones.as_ref() else {
        return Ok(None);
    };
    work_unit().await;
    let filter = ZoneFilterOwned::new(
        table.zones().len(),
        state.combat_level,
        endpoints,
        exemptions,
    )
    .map_err(|_| SearchCursorError::Allocation)?;
    Ok(Some(filter))
}

async fn complete_owned_zone_filter(
    filter: &mut ZoneFilterOwned,
    table: &ZoneTable,
) -> Result<(), SearchCursorError> {
    if filter.zone_count() != table.zones().len() {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    while !filter.is_ready() {
        work_unit().await;
        filter.advance(table, 1);
    }
    if filter.view(table).is_none() {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    Ok(())
}

fn owned_zone_filter_view<'a>(
    graph: &'a TransportGraph,
    filter: Option<&'a ZoneFilterOwned>,
) -> Result<Option<ZoneFilter<'a>>, SearchCursorError> {
    match (filter, graph.zones.as_ref()) {
        (Some(filter), Some(table)) => filter
            .view(table)
            .map(Some)
            .ok_or(SearchCursorError::ResultNotRepresentable),
        (Some(_), None) => Err(SearchCursorError::ResultNotRepresentable),
        (None, _) => Ok(None),
    }
}

async fn goal_requires_zone_completion(
    graph: &TransportGraph,
    filter: &ZoneFilter<'_>,
    goal: WorldTile,
) -> bool {
    for zone in filter.blocking_at(&graph.wilderness, goal) {
        work_unit().await;
        if !filter.origin_active(&graph.wilderness, zone) {
            return true;
        }
    }
    false
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative single search borrows the unchanged collision, graph and search inputs"
)]
async fn single_cursor_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    progress: Option<&AtomicUsize>,
) -> Result<(Result<CompactRoute, RouteError>, usize), SearchCursorError> {
    if from == to {
        let (result, _, settled) = run_single_compact_search(
            collision, graph, from, to, options, state, avoid, budget, None, None, progress,
        )
        .await?;
        return Ok((result, settled));
    }

    let mut safe_owned = new_owned_zone_filter(graph, state, &[from], ZoneExempt::NONE).await?;
    if let (Some(filter), Some(table)) = (safe_owned.as_mut(), graph.zones.as_ref()) {
        complete_owned_zone_filter(filter, table).await?;
    }
    let safe_view = owned_zone_filter_view(graph, safe_owned.as_ref())?;
    let safe_impossible = if let Some(filter) = safe_view.as_ref() {
        zone_goal_proof_cooperative(
            collision,
            graph,
            from,
            std::slice::from_ref(&to),
            options,
            state,
            Relax::Strict,
            filter,
            true,
        )
        .await?
        .proof
            == ReverseProof::Unreachable
    } else {
        false
    };
    let mut forward_settled = 0usize;
    let mut result = if safe_impossible {
        Err(RouteError::NoPath)
    } else {
        let (result, _, settled) = run_single_compact_search(
            collision,
            graph,
            from,
            to,
            options,
            state,
            avoid,
            budget,
            safe_view.as_ref(),
            None,
            progress,
        )
        .await?;
        forward_settled = settled;
        result
    };

    if matches!(
        result,
        Err(RouteError::NoPath | RouteError::BudgetExhausted)
    ) {
        if let Some(filter) = safe_view.as_ref() {
            let goal_completion = goal_requires_zone_completion(graph, filter, to).await;
            for granted in [ZoneExempt::NONE, options.zones]
                .into_iter()
                .take(1 + usize::from(options.zones != ZoneExempt::NONE))
            {
                if granted == ZoneExempt::NONE && !goal_completion {
                    continue;
                }
                let mut completion =
                    new_owned_zone_filter(graph, state, &[from, to], granted).await?;
                if let (Some(filter), Some(table)) = (completion.as_mut(), graph.zones.as_ref()) {
                    complete_owned_zone_filter(filter, table).await?;
                }
                let completion_view = owned_zone_filter_view(graph, completion.as_ref())?;
                let (completion_result, _, settled) = run_single_compact_search(
                    collision,
                    graph,
                    from,
                    to,
                    options,
                    state,
                    avoid,
                    budget,
                    completion_view.as_ref(),
                    None,
                    progress,
                )
                .await?;
                forward_settled = forward_settled
                    .checked_add(settled)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                result = completion_result;
                if !matches!(
                    result,
                    Err(RouteError::NoPath | RouteError::BudgetExhausted)
                ) {
                    break;
                }
            }
        }
    }
    Ok((result, forward_settled))
}

struct ManyCursorPass {
    results: Vec<Result<TargetCost, TargetError>>,
    came_from: Predecessors,
    forward_settled: usize,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative many-target pass borrows the unchanged collision, graph and search inputs"
)]
async fn many_cursor_pass(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    zones: Option<&ZoneFilter<'_>>,
    progress: Option<&AtomicUsize>,
) -> Result<ManyCursorPass, SearchCursorError> {
    let mut unique = HashMap::new();
    unique
        .try_reserve(targets.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    let mut costs = Vec::new();
    costs
        .try_reserve_exact(targets.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    let mut remaining = 0usize;
    for &target in targets {
        work_unit().await;
        if unique.contains_key(&target) {
            continue;
        }
        let result = if target == from {
            Ok(TargetCost {
                ticks: 0.0,
                settled_at: 0,
            })
        } else if let Some(zones) = zones {
            let proof = zone_goal_proof_cooperative(
                collision,
                graph,
                from,
                std::slice::from_ref(&target),
                options,
                state,
                Relax::Strict,
                zones,
                true,
            )
            .await?;
            if proof.proof == ReverseProof::Unreachable {
                Err(TargetError::NoPath)
            } else {
                remaining = remaining
                    .checked_add(1)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                Err(TargetError::NotSettled)
            }
        } else {
            remaining = remaining
                .checked_add(1)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            Err(TargetError::NotSettled)
        };
        let index = costs.len();
        costs.push(result);
        unique.insert(target, index);
    }

    let mut goals = Goals::Many {
        unique: &unique,
        costs: &mut costs,
        remaining,
    };
    let search = if remaining == 0 {
        KernelOutcome::empty()
    } else {
        search_kernel(
            collision,
            graph,
            from,
            CostModel::running(),
            budget,
            options.allow_teleports,
            options.allow_wilderness,
            state,
            options.essence.as_ref(),
            Relax::Strict,
            avoid,
            zones,
            &mut goals,
            None,
            true,
            progress,
        )
        .await
    };
    let error = match search.stop {
        SearchStop::Exhausted => TargetError::NoPath,
        SearchStop::Budget => TargetError::BudgetExhausted,
        SearchStop::Allocation => return Err(SearchCursorError::Allocation),
        SearchStop::Deadline | SearchStop::Completed => TargetError::NotSettled,
    };
    for result in &mut costs {
        work_unit().await;
        if *result == Err(TargetError::NotSettled) {
            *result = Err(error);
        }
    }
    let mut results = Vec::new();
    results
        .try_reserve_exact(targets.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    for &target in targets {
        work_unit().await;
        let index = *unique
            .get(&target)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        results.push(costs[index]);
    }
    Ok(ManyCursorPass {
        results,
        came_from: search.came_from,
        forward_settled: search.settled,
    })
}

struct IndexedZoneGoal {
    key: Vec<u16>,
    index: usize,
}

async fn consider_many_best(
    best: &mut Option<(usize, RouteTrace)>,
    indices: Option<&[usize]>,
    targets: &[WorldTile],
    results: &[Result<TargetCost, TargetError>],
    origin: WorldTile,
    essence: Option<EssenceSession>,
    came_from: &Predecessors,
) -> Result<(), SearchCursorError> {
    let mut selected: Option<(usize, WorldTile, TargetCost)> = None;
    for (local_index, (&target, result)) in targets.iter().zip(results).enumerate() {
        work_unit().await;
        let Ok(cost) = result else {
            continue;
        };
        let index = match indices {
            Some(indices) => *indices
                .get(local_index)
                .ok_or(SearchCursorError::ResultNotRepresentable)?,
            None => local_index,
        };
        let incumbent = selected
            .as_ref()
            .map(|(old_index, _, old_cost)| (*old_index, old_cost.ticks))
            .or_else(|| {
                best.as_ref()
                    .map(|(old_index, route)| (*old_index, route.ticks))
            });
        if incumbent.is_none_or(|(old_index, old_ticks)| {
            cost.ticks < old_ticks || (cost.ticks == old_ticks && index < old_index)
        }) {
            selected = Some((index, target, *cost));
        }
    }
    if let Some((index, target, cost)) = selected {
        let route =
            RouteTrace::from_predecessors(origin, target, cost.ticks, came_from, essence).await?;
        *best = Some((index, route));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative many-target search borrows the unchanged collision, graph and search inputs"
)]
async fn many_cursor_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    reconstruct_best: bool,
    progress: Option<&AtomicUsize>,
) -> Result<
    (
        Vec<Result<TargetCost, TargetError>>,
        Option<(usize, CompactRoute)>,
        usize,
    ),
    SearchCursorError,
> {
    let mut safe_owned = new_owned_zone_filter(graph, state, &[from], ZoneExempt::NONE).await?;
    if let (Some(filter), Some(table)) = (safe_owned.as_mut(), graph.zones.as_ref()) {
        complete_owned_zone_filter(filter, table).await?;
    }
    let safe_view = owned_zone_filter_view(graph, safe_owned.as_ref())?;
    let safe = many_cursor_pass(
        collision,
        graph,
        from,
        targets,
        options,
        state,
        avoid,
        budget,
        safe_view.as_ref(),
        progress,
    )
    .await?;
    let mut forward_settled = safe.forward_settled;
    let mut best = None;
    if reconstruct_best {
        consider_many_best(
            &mut best,
            None,
            targets,
            &safe.results,
            from,
            options.essence,
            &safe.came_from,
        )
        .await?;
    }
    let mut results = safe.results;
    // The safe pass's predecessor table is no longer used. Release it before
    // another zone partition allocates its own table.
    drop(safe.came_from);

    let mut partitions = Vec::new();
    if let Some(filter) = safe_view.as_ref() {
        partitions
            .try_reserve_exact(targets.len())
            .map_err(|_| SearchCursorError::Allocation)?;
        for (index, &target) in targets.iter().enumerate() {
            work_unit().await;
            let key = goal_zone_key_cooperative(graph, filter, target).await?;
            if !key.is_empty() || options.zones != ZoneExempt::NONE {
                partitions.push(IndexedZoneGoal { key, index });
            }
        }
    }
    sort_unstable_by_cooperative(&mut partitions, |left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.index.cmp(&right.index))
    })
    .await;

    for granted in [ZoneExempt::NONE, options.zones]
        .into_iter()
        .take(1 + usize::from(options.zones != ZoneExempt::NONE))
    {
        let mut start = 0usize;
        while start < partitions.len() {
            work_unit().await;
            let mut end = start + 1;
            while end < partitions.len() && partitions[end].key == partitions[start].key {
                work_unit().await;
                end += 1;
            }
            if partitions[start].key.is_empty() && granted == ZoneExempt::NONE {
                start = end;
                continue;
            }
            let mut indices = Vec::new();
            let mut goals = Vec::new();
            indices
                .try_reserve_exact(end - start)
                .map_err(|_| SearchCursorError::Allocation)?;
            goals
                .try_reserve_exact(end - start)
                .map_err(|_| SearchCursorError::Allocation)?;
            for input in &partitions[start..end] {
                work_unit().await;
                if matches!(
                    results[input.index],
                    Err(TargetError::NoPath | TargetError::BudgetExhausted)
                ) {
                    indices.push(input.index);
                    goals.push(targets[input.index]);
                }
            }
            if !indices.is_empty() {
                let endpoint = *goals
                    .first()
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                let mut completion =
                    new_owned_zone_filter(graph, state, &[from, endpoint], granted).await?;
                if let (Some(filter), Some(table)) = (completion.as_mut(), graph.zones.as_ref()) {
                    complete_owned_zone_filter(filter, table).await?;
                }
                let completion_view = owned_zone_filter_view(graph, completion.as_ref())?;
                let pass = many_cursor_pass(
                    collision,
                    graph,
                    from,
                    &goals,
                    options,
                    state,
                    avoid,
                    budget,
                    completion_view.as_ref(),
                    progress,
                )
                .await?;
                forward_settled = forward_settled
                    .checked_add(pass.forward_settled)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                if reconstruct_best {
                    consider_many_best(
                        &mut best,
                        Some(&indices),
                        &goals,
                        &pass.results,
                        from,
                        options.essence,
                        &pass.came_from,
                    )
                    .await?;
                }
                for (position, &index) in indices.iter().enumerate() {
                    work_unit().await;
                    results[index] = pass.results[position];
                }
            }
            start = end;
        }
    }
    let best = match best {
        Some((index, trace)) => Some((index, trace.materialize(graph).await?)),
        None => None,
    };
    Ok((results, best, forward_settled))
}

#[derive(Clone, Copy)]
enum FirstRouteMaterialization {
    Compact,
    DiagnosticEdges,
}

enum FirstRoutePayload {
    Compact(RouteTrace),
    DiagnosticEdges(Vec<u32>),
}

struct FirstRouteCandidate {
    destination: WorldTile,
    ticks: f64,
    payload: FirstRoutePayload,
}

enum FirstFallbackCandidate {
    Routed(FirstRouteCandidate),
    Failed(RouteError),
    Undecided,
}

struct CompactFirstPass {
    preferred: Result<FirstRouteCandidate, RouteError>,
    fallback: Option<FirstFallbackCandidate>,
    settled: usize,
    capacities: SearchCapacities,
    proof: ReverseProof,
    completion_partitions: usize,
}

impl FirstRouteCandidate {
    async fn into_compact_route(
        self,
        graph: &TransportGraph,
    ) -> Result<CompactRoute, SearchCursorError> {
        match self.payload {
            FirstRoutePayload::Compact(trace) => trace.materialize(graph).await,
            FirstRoutePayload::DiagnosticEdges(_) => {
                unreachable!("diagnostic candidates are never published as compact routes")
            }
        }
    }

    fn into_diagnostic_edges(self) -> Vec<u32> {
        match self.payload {
            FirstRoutePayload::DiagnosticEdges(edges) => edges,
            FirstRoutePayload::Compact(_) => {
                unreachable!("compact candidates are not diagnostic edge traces")
            }
        }
    }
}

async fn first_route_candidate_from_predecessors(
    materialization: FirstRouteMaterialization,
    from: WorldTile,
    destination: WorldTile,
    ticks: f64,
    came_from: &Predecessors,
    essence: Option<EssenceSession>,
) -> Result<FirstRouteCandidate, SearchCursorError> {
    let payload = match materialization {
        FirstRouteMaterialization::Compact => FirstRoutePayload::Compact(
            RouteTrace::from_predecessors(from, destination, ticks, came_from, essence).await?,
        ),
        FirstRouteMaterialization::DiagnosticEdges => FirstRoutePayload::DiagnosticEdges(
            edge_backtrace_from_predecessors(from, destination, came_from).await?,
        ),
    };
    Ok(FirstRouteCandidate {
        destination,
        ticks,
        payload,
    })
}

fn first_route_order(route: &FirstRouteCandidate) -> (i32, i32, i32) {
    (
        route.destination.x,
        route.destination.z,
        route.destination.level,
    )
}

async fn choose_first_route(
    current: &mut Result<FirstRouteCandidate, RouteError>,
    candidate: Result<FirstRouteCandidate, RouteError>,
    order: Option<&[WorldTile]>,
) {
    match candidate {
        Ok(route) => {
            let replace = match current.as_ref() {
                Err(_) => true,
                Ok(old) if route.ticks == old.ticks => match order {
                    Some(targets) => {
                        target_precedes(targets, route.destination, old.destination).await
                    }
                    None => first_route_order(&route) > first_route_order(old),
                },
                Ok(old) => route.ticks < old.ticks,
            };
            if replace {
                *current = Ok(route);
            }
        }
        Err(RouteError::BudgetExhausted) if current.is_err() => {
            *current = Err(RouteError::BudgetExhausted);
        }
        Err(_) => {}
    }
}

async fn target_precedes(targets: &[WorldTile], candidate: WorldTile, old: WorldTile) -> bool {
    for &tile in targets {
        work_unit().await;
        if tile == old {
            return false;
        }
        if tile == candidate {
            return true;
        }
    }
    false
}

fn merge_first_fallback(
    current: &mut Option<FirstFallbackCandidate>,
    candidate: FirstFallbackCandidate,
) {
    match candidate {
        FirstFallbackCandidate::Routed(route) => {
            if !matches!(
                current.as_ref(),
                Some(FirstFallbackCandidate::Routed(old))
                    if old.ticks < route.ticks
                        || (old.ticks == route.ticks && first_route_order(old) >= first_route_order(&route))
            ) {
                *current = Some(FirstFallbackCandidate::Routed(route));
            }
        }
        FirstFallbackCandidate::Undecided
            if !matches!(current.as_ref(), Some(FirstFallbackCandidate::Routed(_))) =>
        {
            *current = Some(FirstFallbackCandidate::Undecided)
        }
        FirstFallbackCandidate::Failed(error)
            if current.is_none()
                || matches!(
                    current.as_ref(),
                    Some(FirstFallbackCandidate::Failed(RouteError::NoPath))
                ) =>
        {
            *current = Some(FirstFallbackCandidate::Failed(error))
        }
        _ => {}
    }
}

fn into_first_pass(mut pass: CompactFirstPass, fallback_only: bool) -> CompactFirstPass {
    if fallback_only {
        let answer = match std::mem::replace(&mut pass.preferred, Err(RouteError::NoPath)) {
            Ok(route) => FirstFallbackCandidate::Routed(route),
            Err(error) => FirstFallbackCandidate::Failed(error),
        };
        pass.fallback = Some(answer);
    }
    pass
}

async fn into_compact_first_search(
    pass: CompactFirstPass,
    graph: &TransportGraph,
) -> Result<CompactFirstRouteSearch, SearchCursorError> {
    let preferred = match pass.preferred {
        Ok(route) => Ok(route.into_compact_route(graph).await?),
        Err(error) => Err(error),
    };
    let fallback = match pass.fallback {
        Some(FirstFallbackCandidate::Routed(route)) => Some(CompactFallbackRoute::Routed(
            route.into_compact_route(graph).await?,
        )),
        Some(FirstFallbackCandidate::Failed(error)) => Some(CompactFallbackRoute::Failed(error)),
        Some(FirstFallbackCandidate::Undecided) => Some(CompactFallbackRoute::Undecided),
        None => None,
    };
    Ok(CompactFirstRouteSearch {
        preferred,
        fallback,
        settled: pass.settled,
        capacities: pass.capacities,
        proof: pass.proof,
        completion_partitions: pass.completion_partitions,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the compact first-goal search borrows the unchanged collision, graph and search inputs"
)]
async fn first_search_core_compact(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    relax: Relax,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
    zones: Option<&ZoneFilter<'_>>,
    progress: Option<&AtomicUsize>,
    materialization: FirstRouteMaterialization,
) -> Result<CompactFirstPass, SearchCursorError> {
    if targets.is_empty() {
        return Ok(CompactFirstPass {
            preferred: Err(RouteError::NoPath),
            fallback: (!fallback.is_empty()).then_some(FirstFallbackCandidate::Undecided),
            settled: 0,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
            completion_partitions: 0,
        });
    }
    for &target in targets {
        work_unit().await;
        if target == from {
            let route = first_route_candidate_from_predecessors(
                materialization,
                from,
                from,
                0.0,
                &Predecessors::empty(),
                options.essence,
            )
            .await?;
            return Ok(CompactFirstPass {
                preferred: Ok(route),
                fallback: None,
                settled: 0,
                capacities: SearchCapacities::default(),
                proof: ReverseProof::NotRun,
                completion_partitions: 0,
            });
        }
    }

    let mut goal_proof_capacity = SearchCapacities::default();
    if fallback.is_empty() {
        if let Some(zones) = zones {
            let proof = zone_goal_proof_cooperative(
                collision, graph, from, targets, options, state, relax, zones, true,
            )
            .await?;
            goal_proof_capacity = SearchCapacities {
                reverse: proof.seen,
                reverse_queue: proof.queue,
                zone_mask_words: zones.mask_words(),
                ..SearchCapacities::default()
            };
            if proof.proof == ReverseProof::Unreachable {
                return Ok(CompactFirstPass {
                    preferred: Err(RouteError::NoPath),
                    fallback: None,
                    settled: 0,
                    capacities: goal_proof_capacity,
                    proof: proof.proof,
                    completion_partitions: 0,
                });
            }
        }
    }

    let mut inventory_can_grow = false;
    for edge in graph
        .edges
        .iter()
        .chain(graph.teleports.iter().filter(|_| options.allow_teleports))
    {
        work_unit().await;
        if !edge.item_returns.is_empty() && edge_allowed(state, edge, relax) {
            inventory_can_grow = true;
            break;
        }
    }
    let gates = ProofGates {
        collision,
        graph,
        state,
        essence: options.essence.as_ref(),
        from,
        use_teleports: options.allow_teleports,
        allow_wilderness: options.allow_wilderness,
        relax,
        inventory_can_grow,
        cooperative_storage: true,
    };
    let first = FirstGoals::new_cooperative(
        gates,
        targets,
        fallback,
        budget,
        reachable_budget,
        options.prefer_target_order,
    )
    .await?;
    let mut goals = Goals::First(Box::new(first));
    let search = search_kernel(
        collision,
        graph,
        from,
        CostModel::running(),
        reachable_budget,
        options.allow_teleports,
        options.allow_wilderness,
        state,
        options.essence.as_ref(),
        relax,
        avoid,
        zones,
        &mut goals,
        None,
        true,
        progress,
    )
    .await;
    if search.stop == SearchStop::Allocation {
        return Err(SearchCursorError::Allocation);
    }
    let Goals::First(first) = goals else {
        unreachable!("a first-goal search keeps its goals");
    };
    let preferred = if let Some((destination, cost)) = first.preferred.found {
        Ok(first_route_candidate_from_predecessors(
            materialization,
            from,
            destination,
            cost.ticks,
            &search.came_from,
            options.essence,
        )
        .await?)
    } else {
        Err(first.preferred.closed.unwrap_or(RouteError::NoPath))
    };
    let fallback_result = if preferred.is_ok() || fallback.is_empty() {
        None
    } else {
        match (first.fallback.found, first.fallback.closed) {
            (Some((destination, cost)), _) => Some(FirstFallbackCandidate::Routed(
                first_route_candidate_from_predecessors(
                    materialization,
                    from,
                    destination,
                    cost.ticks,
                    &search.came_from,
                    options.essence,
                )
                .await?,
            )),
            (None, Some(error)) => Some(FirstFallbackCandidate::Failed(error)),
            (None, None) => Some(FirstFallbackCandidate::Undecided),
        }
    };
    let mut capacities = search.capacities;
    capacities.include(goal_proof_capacity);
    Ok(CompactFirstPass {
        preferred,
        fallback: fallback_result,
        settled: search.settled,
        capacities,
        proof: search.proof,
        completion_partitions: 0,
    })
}

struct FirstZoneGoal {
    key: Box<[u16]>,
    /// Index into preferred goals followed by fallback goals.
    index: usize,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative first-goal search borrows the unchanged collision, graph and search inputs"
)]
async fn first_cursor_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    relax: Relax,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
    progress: Option<&AtomicUsize>,
) -> Result<CompactFirstRouteSearch, SearchCursorError> {
    let pass = first_cursor_search_with_output(
        collision,
        graph,
        from,
        targets,
        fallback,
        options,
        state,
        relax,
        avoid,
        budget,
        reachable_budget,
        progress,
        FirstRouteMaterialization::Compact,
    )
    .await?;
    into_compact_first_search(pass, graph).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative first-goal search borrows the unchanged collision, graph and search inputs"
)]
async fn first_cursor_search_with_output(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    relax: Relax,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
    progress: Option<&AtomicUsize>,
    materialization: FirstRouteMaterialization,
) -> Result<CompactFirstPass, SearchCursorError> {
    let fallback_only = targets.is_empty() && !fallback.is_empty();
    let (targets, fallback) = if fallback_only {
        (fallback, &[] as &[WorldTile])
    } else {
        (targets, fallback)
    };

    let mut safe_owned = new_owned_zone_filter(graph, state, &[from], ZoneExempt::NONE).await?;
    if let (Some(filter), Some(table)) = (safe_owned.as_mut(), graph.zones.as_ref()) {
        complete_owned_zone_filter(filter, table).await?;
    }
    let safe_view = owned_zone_filter_view(graph, safe_owned.as_ref())?;
    let mut target_is_origin = false;
    for &target in targets {
        work_unit().await;
        if target == from {
            target_is_origin = true;
            break;
        }
    }
    let Some(filter) = safe_view.as_ref() else {
        let pass = first_search_core_compact(
            collision,
            graph,
            from,
            targets,
            fallback,
            options,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
            None,
            progress,
            materialization,
        )
        .await?;
        return Ok(into_first_pass(pass, fallback_only));
    };
    if targets.is_empty() || target_is_origin {
        let pass = first_search_core_compact(
            collision,
            graph,
            from,
            targets,
            fallback,
            options,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
            Some(filter),
            progress,
            materialization,
        )
        .await?;
        return Ok(into_first_pass(pass, fallback_only));
    }

    let partition_capacity = targets
        .len()
        .checked_add(fallback.len())
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let mut partitions = Vec::new();
    partitions
        .try_reserve_exact(partition_capacity)
        .map_err(|_| SearchCursorError::Allocation)?;
    for (index, &target) in targets.iter().enumerate() {
        work_unit().await;
        let key = goal_zone_key_cooperative(graph, filter, target).await?;
        if !key.is_empty() || options.zones != ZoneExempt::NONE {
            partitions.push(FirstZoneGoal {
                key: key.into_boxed_slice(),
                index,
            });
        }
    }
    for (index, &target) in fallback.iter().enumerate() {
        work_unit().await;
        let key = goal_zone_key_cooperative(graph, filter, target).await?;
        if !key.is_empty() || options.zones != ZoneExempt::NONE {
            partitions.push(FirstZoneGoal {
                key: key.into_boxed_slice(),
                index: targets.len() + index,
            });
        }
    }
    sort_unstable_by_cooperative(&mut partitions, |left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.index.cmp(&right.index))
    })
    .await;
    if partitions.is_empty() {
        let pass = first_search_core_compact(
            collision,
            graph,
            from,
            targets,
            fallback,
            options,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
            Some(filter),
            progress,
            materialization,
        )
        .await?;
        return Ok(into_first_pass(pass, fallback_only));
    }

    let mut merged = first_search_core_compact(
        collision,
        graph,
        from,
        targets,
        fallback,
        options,
        state,
        relax,
        avoid,
        budget,
        reachable_budget,
        Some(filter),
        progress,
        materialization,
    )
    .await?;
    if !options.prefer_target_order
        && (matches!(
            merged.fallback.as_ref(),
            Some(FirstFallbackCandidate::Routed(_))
        ) || !matches!(
            merged.preferred.as_ref(),
            Err(RouteError::NoPath | RouteError::BudgetExhausted)
        ))
    {
        return Ok(into_first_pass(merged, fallback_only));
    }

    for granted in [ZoneExempt::NONE, options.zones]
        .into_iter()
        .take(1 + usize::from(options.zones != ZoneExempt::NONE))
    {
        let mut start = 0usize;
        while start < partitions.len() {
            work_unit().await;
            let mut end = start + 1;
            while end < partitions.len() && partitions[end].key == partitions[start].key {
                work_unit().await;
                end += 1;
            }
            if partitions[start].key.is_empty() && granted == ZoneExempt::NONE {
                start = end;
                continue;
            }
            let mut preferred_count = 0usize;
            let mut fallback_count = 0usize;
            for goal in &partitions[start..end] {
                work_unit().await;
                if goal.index < targets.len() {
                    preferred_count += 1;
                } else {
                    fallback_count += 1;
                }
            }
            let mut preferred = Vec::new();
            preferred
                .try_reserve_exact(preferred_count)
                .map_err(|_| SearchCursorError::Allocation)?;
            let mut fallbacks = Vec::new();
            fallbacks
                .try_reserve_exact(fallback_count)
                .map_err(|_| SearchCursorError::Allocation)?;
            for goal in &partitions[start..end] {
                work_unit().await;
                if goal.index < targets.len() {
                    preferred.push(targets[goal.index]);
                } else {
                    fallbacks.push(fallback[goal.index - targets.len()]);
                }
            }
            let endpoint = preferred
                .first()
                .or_else(|| fallbacks.first())
                .copied()
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            let mut completion =
                new_owned_zone_filter(graph, state, &[from, endpoint], granted).await?;
            if let (Some(completion), Some(table)) = (completion.as_mut(), graph.zones.as_ref()) {
                complete_owned_zone_filter(completion, table).await?;
            }
            let completion_view = owned_zone_filter_view(graph, completion.as_ref())?;
            let mut pass = if preferred.is_empty() {
                let mut fallback_pass = first_search_core_compact(
                    collision,
                    graph,
                    from,
                    &fallbacks,
                    &[],
                    options,
                    state,
                    relax,
                    avoid,
                    budget,
                    reachable_budget,
                    completion_view.as_ref(),
                    progress,
                    materialization,
                )
                .await?;
                let answer = match std::mem::replace(
                    &mut fallback_pass.preferred,
                    Err(RouteError::NoPath),
                ) {
                    Ok(route) => FirstFallbackCandidate::Routed(route),
                    Err(error) => FirstFallbackCandidate::Failed(error),
                };
                fallback_pass.fallback = Some(answer);
                fallback_pass
            } else {
                first_search_core_compact(
                    collision,
                    graph,
                    from,
                    &preferred,
                    &fallbacks,
                    options,
                    state,
                    relax,
                    avoid,
                    budget,
                    reachable_budget,
                    completion_view.as_ref(),
                    progress,
                    materialization,
                )
                .await?
            };
            pass.completion_partitions = 0;
            pass.capacities.zone_mask_words = pass
                .capacities
                .zone_mask_words
                .checked_add(filter.mask_words())
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            merged.settled = merged
                .settled
                .checked_add(pass.settled)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            merged.completion_partitions = merged
                .completion_partitions
                .checked_add(1)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            merged.capacities.include(pass.capacities);
            choose_first_route(
                &mut merged.preferred,
                pass.preferred,
                options.prefer_target_order.then_some(targets),
            )
            .await;
            if merged.preferred.is_ok() {
                // The final result suppresses fallback whenever preferred wins.
                // Release that obsolete trace before the next partition search.
                merged.fallback = None;
            } else if let Some(answer) = pass.fallback {
                merge_first_fallback(&mut merged.fallback, answer);
            }
            start = end;
        }
        if merged.preferred.is_ok() {
            merged.fallback = None;
            break;
        }
        if matches!(
            merged.fallback.as_ref(),
            Some(FirstFallbackCandidate::Routed(_))
        ) {
            break;
        }
    }
    Ok(into_first_pass(merged, fallback_only))
}

/// Why [`find`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// No walk/transport path reaches the destination.
    NoPath,
    /// The node-expansion budget was exhausted before reaching it.
    BudgetExhausted,
}

/// Per-target failure of a shared search. Unlike single-target
/// [`RouteError`], a caller deadline can leave a target unsettled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    NoPath,
    BudgetExhausted,
    NotSettled,
}

impl From<RouteError> for TargetError {
    fn from(error: RouteError) -> Self {
        match error {
            RouteError::NoPath => Self::NoPath,
            RouteError::BudgetExhausted => Self::BudgetExhausted,
        }
    }
}

/// Node-expansion budget bounding a [`find`] search — the m8aq route-cutoff
/// concept. The whole 2004 world is ~16M tiles, so any real route stays far
/// under this; it only stops pathological floods.
pub const NODE_BUDGET: usize = 4_000_000;
/// Small initial table shared by every search shape; the kernel grows it
/// only when the visited entries need more space.
const SYNC_FIRST_TABLE_INITIAL_SETTLES: usize = 6;

/// The interact radius (chebyshev) a transport edge is usable from: any
/// standable tile within this distance of the edge's `at` is a valid
/// take-off. The game only accepts `op_loc` from adjacent (1). A radius
/// of 3 let Dijkstra "use" a door through a fence (Lumbridge cow pen:
/// inside tile to the north-west road gate) and the walker then walked
/// into the wall. The approach is derived at expansion time, never baked.
const INTERACT_RADIUS: i32 = 1;

/// The eight step deltas (client coordinates: +x east, +z north).
const STEPS: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// The client's `PL_WALK_*` movement masks, as `u32` to match the flag word.
const MASK_N: u32 = CollisionFlag::PL_WALK_N as u32;
const MASK_E: u32 = CollisionFlag::PL_WALK_E as u32;
const MASK_S: u32 = CollisionFlag::PL_WALK_S as u32;
const MASK_W: u32 = CollisionFlag::PL_WALK_W as u32;
const MASK_NE: u32 = CollisionFlag::PL_WALK_NE as u32;
const MASK_SE: u32 = CollisionFlag::PL_WALK_SE as u32;
const MASK_NW: u32 = CollisionFlag::PL_WALK_NW as u32;
const MASK_SW: u32 = CollisionFlag::PL_WALK_SW as u32;

/// Dijkstra over `collision` walk steps (each costing the search's run
/// rate) and `graph` transport edges (each costing `edge.ticks`), all in
/// one heap minimizing total ticks. Walk steps are 8-directional and stay
/// on `from`'s level; transports may change level.
///
/// The origin may sit on a blocked tile (a loc-blocked tele landing); only
/// the tiles stepped *onto* are tested. Destinations are reached exactly:
/// a blocked destination is `NoPath` unless a transport lands on it.
/// Edges are gated by the fail-closed empty [`WorldState`] — an edge
/// whose requirements nothing proves is never relaxed. Callers that know
/// the player's facts use [`find_with`].
pub fn find(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
) -> Result<Route, RouteError> {
    find_with(
        collision,
        graph,
        from,
        to,
        FindOptions::default(),
        &WorldState::empty(),
    )
}

/// [`find`] with explicit opt-ins ([`FindOptions::allow_teleports`],
/// [`FindOptions::allow_wilderness`], and [`FindOptions::allow_bank_fetch`])
/// and the gating [`WorldState`]: an edge is relaxed only when
/// every skill / quest / varp / worn gate is satisfied and every quest-stage
/// gate is `True`. Held item gates remain reusable; consumed requirements are
/// debited along the path (including returned charge variants), so later gates
/// see the remaining stacks. Resource labels are bounded and shared by balance;
/// ordinary walk expansions allocate no per-node inventory. Missing facts fail
/// closed — the flag alone
/// never fetches ([`find_missing_item_reqs`] is the session's diagnosis
/// arm, and the session itself lives in [`crate::bank_fetch`]); an
/// `Unknown` stage gate is named by [`find_unresolved_quest_gates`].
pub fn find_with(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with_avoid(collision, graph, from, to, opts, state, &[])
}

/// [`find_with`] with inspect-local avoidance rectangles. Walk steps never
/// enter a rect from outside; a start already inside any rect stays
/// searchable until the route leaves the union. Transport, any-tile
/// teleport, and essence-return landings follow the same outside-in rule;
/// takeoff `at` tiles are not independently filtered.
pub fn find_with_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Result<Route, RouteError> {
    find_with_avoid_bounded(collision, graph, from, to, opts, state, avoid, NODE_BUDGET)
}

/// [`find_with_avoid`] with an injectable node-expansion cap. Production
/// inspect uses [`find_with_avoid`] (the default [`NODE_BUDGET`]); tests
/// may pass a smaller bound to distinguish `BudgetExhausted` from `NoPath`.
#[allow(clippy::too_many_arguments)] // collision/graph/tiles/opts/state/avoid/budget surface
pub fn find_with_avoid_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        CostModel::running(),
        budget,
        opts.allow_teleports,
        opts.allow_wilderness,
        state,
        opts.essence.as_ref(),
        avoid,
        opts.zones,
    )
}

/// Native bank-search cap: 500,000 non-goal expansions precede an accepted
/// goal, which has 1-based settle ordinal 500,001.
pub const BANK_TARGET_BUDGET: usize = 500_001;
/// Settles a first-goal search's goal set keeps the search going while the
/// set's backward proof has not shown any of its goals reachable. A proof of
/// reachability lifts the set to [`find_with`]'s full budget; a proof of
/// unreachability stops it at once.
/// Only a goal set unreachable from a large region whose own backward region
/// is also large spends all of it: on the 289 bake, stands fed through
/// ladders by the unstamped upper planes. Measured reachable in-scene stands
/// need at most ~347,000 settles (a 599-tick Wilderness detour; ~279,000
/// with every teleport usable), and 2^19 settles hold about 90 MB of search
/// tables.
pub const FIRST_TARGET_BUDGET: usize = 1 << 19;
/// Tiles the backward proof may admit before it leaves the answer to the
/// forward search alone. It takes one step per settled node, so this only
/// caps its own set (a 12-byte tile each, plus hash-table overhead) when
/// the forward budget is larger.
const REVERSE_PROOF_BUDGET: usize = 1 << 18;

/// A target's exact shortest-path cost and 1-based shared settle ordinal.
/// An origin shortcut has ordinal zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetCost {
    pub ticks: f64,
    pub settled_at: usize,
}

/// Allocated entry capacities for this request's search scratch at stop.
/// HashMap/heap capacities never shrink during the flood; these are their
/// peak allocated entry capacities, not retained per worker or pack bytes.
/// `reverse` and `reverse_queue` are the larger of the goal sets' backward
/// proof sets and queues (zero when no proof ran); each proof's scratch is
/// dropped as soon as it decides or its set stops.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchCapacities {
    pub distances: usize,
    pub predecessors: usize,
    pub settled: usize,
    pub heap: usize,
    pub reverse: usize,
    pub reverse_queue: usize,
    /// Allocated u64 mask words (zero for all-exempt and legacy worlds).
    pub zone_mask_words: usize,
}

impl SearchCapacities {
    fn include(&mut self, other: Self) {
        self.distances = self.distances.max(other.distances);
        self.predecessors = self.predecessors.max(other.predecessors);
        self.settled = self.settled.max(other.settled);
        self.heap = self.heap.max(other.heap);
        self.reverse = self.reverse.max(other.reverse);
        self.reverse_queue = self.reverse_queue.max(other.reverse_queue);
        self.zone_mask_words = self.zone_mask_words.max(other.zone_mask_words);
    }
}

/// How the backward proof beside a goal search ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReverseProof {
    /// No proof ran: an empty goal set, an origin shortcut, or a search
    /// kind that does not carry one.
    #[default]
    NotRun,
    /// Undecided when the forward search settled a goal or hit its budget.
    Open,
    /// The backward region met the origin, or the landing of a teleport the
    /// origin can take: a goal is reachable, and the forward search keeps
    /// its full budget to find the cheapest.
    Reachable,
    /// A teleport usable elsewhere lands in the backward region (every tile
    /// may then precede it), or the region outgrew the proof's cap; the
    /// forward search decides within its budget.
    Abandoned,
    /// The backward region closed without the origin: no goal is reachable.
    Unreachable,
}

/// What a first-goal search established about its fallback goal set.
#[derive(Debug, Clone, PartialEq)]
pub enum FallbackRoute {
    /// The cheapest fallback goal, settled on the way.
    Routed(Route),
    /// No fallback goal can settle: the set's own proof shows it
    /// unreachable, or everything reachable settled
    /// ([`RouteError::NoPath`]); or a settle past its own budget exists
    /// without its proof showing a goal reachable
    /// ([`RouteError::BudgetExhausted`]).
    Failed(RouteError),
    /// Only more settles could decide it: its proof shows a goal reachable,
    /// or the preferred proof stopped the search within the fallback's
    /// budget. A search over the fallback set alone decides it.
    Undecided,
}

/// One first-settled-target search. Dijkstra settles nodes in increasing
/// route cost, so the route is the cheapest reachable input target without
/// paying to prove whether every other target is reachable. A search with a
/// fallback goal set ([`find_first_with_fallback`]) also reports what it
/// established about that set when no preferred goal routes.
pub struct FirstRouteSearch {
    route: Result<Route, RouteError>,
    fallback: Option<FallbackRoute>,
    settled: usize,
    capacities: SearchCapacities,
    proof: ReverseProof,
    completion_partitions: usize,
}

impl FirstRouteSearch {
    pub fn route(&self) -> Result<&Route, RouteError> {
        self.route.as_ref().map_err(|error| *error)
    }

    /// The fallback set's answer, when no preferred goal routed and the
    /// search had a fallback set; `None` otherwise.
    pub fn fallback(&self) -> Option<&FallbackRoute> {
        self.fallback.as_ref()
    }

    pub fn into_route(self) -> Result<Route, RouteError> {
        self.route
    }

    /// The preferred route and, as for [`Self::fallback`], the fallback's.
    pub fn into_routes(self) -> (Result<Route, RouteError>, Option<FallbackRoute>) {
        (self.route, self.fallback)
    }

    /// Forward nodes settled (the backward proof's tiles are not counted).
    pub fn settled(&self) -> usize {
        self.settled
    }

    /// Peak scratch entry capacities at the point the search stopped.
    pub fn scratch_capacities(&self) -> SearchCapacities {
        self.capacities
    }

    /// The preferred goals' backward proof.
    pub fn proof(&self) -> ReverseProof {
        self.proof
    }
    /// Additional searches grouped by the chosen endpoint's zone set.
    pub fn completion_partitions(&self) -> usize {
        self.completion_partitions
    }
}

/// Search until the cheapest reachable target settles, under the same
/// native gates as [`find_with`]. Dijkstra alone reports an unreachable goal
/// set only after flooding everything reachable from `from`, so a backward
/// closure from the targets ([`ReverseProof`]) runs one step per settled
/// node: a goal set sealed in a small region (a dead-end pocket, a gated
/// room) is proven unreachable after about twice that region's size. The
/// search spends at most [`FIRST_TARGET_BUDGET`] settles unless the closure
/// proves a goal reachable, and then keeps [`find_with`]'s full budget.
/// Small goal sets are scanned inline without allocation; larger fallback
/// sets build one membership map instead of scanning every target for every
/// settled node.
pub fn find_first_with(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
) -> FirstRouteSearch {
    find_first_with_fallback(collision, graph, from, targets, &[], opts, state)
}

/// [`find_first_with`] with avoidance rectangles ([`find_with_avoid`]).
pub fn find_first_with_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> FirstRouteSearch {
    find_first_with_fallback_avoid(collision, graph, from, targets, &[], opts, state, avoid)
}

/// First-goal search with explicit unproven and reachable settle caps.
#[expect(
    clippy::too_many_arguments,
    reason = "The first-goal route inputs plus the two explicit settle caps."
)]
pub(crate) fn find_first_with_avoid_budgets(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
) -> FirstRouteSearch {
    first_search(
        collision,
        graph,
        from,
        targets,
        &[],
        opts,
        state,
        Relax::Strict,
        avoid,
        budget,
        reachable_budget,
    )
}

/// [`find_first_with`] over preferred `targets` with a `fallback` goal set,
/// sharing one search: the cheapest reachable preferred goal wins even over
/// a cheaper fallback goal, and the search records the first fallback goal
/// it settles on the way ([`FallbackRoute`]). The search stops as it would
/// over the preferred goals alone; the fallback set keeps its own budget
/// and backward proof, and whatever it reports is what a search over the
/// fallback alone reports, as far as the shared settles decide it. Its
/// proof starts only when one of its goals settles past its unproven budget
/// or the preferred set stops without a goal, caught up to one step per
/// settle spent (so a fallback goal within that budget, or a preferred goal,
/// costs it nothing, and only one proof's scratch is live at a time). A
/// goal settling past the budget counts only if the proof showed a goal
/// reachable within it; proven unreachable, or past its budget without
/// proving a goal reachable, the set has failed; otherwise it is
/// [`FallbackRoute::Undecided`] for the caller to search alone. Of the two
/// searches, at most one then stops at the unproven budget.
pub fn find_first_with_fallback(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
) -> FirstRouteSearch {
    find_first_with_fallback_avoid(collision, graph, from, targets, fallback, opts, state, &[])
}

/// [`find_first_with_fallback`] with avoidance rectangles
/// ([`find_with_avoid`]). The backward proofs ignore the rectangles: they
/// over-approximate reachability, so a goal they prove unreachable stays
/// unreachable with the rectangles too.
#[allow(clippy::too_many_arguments)] // search surface plus avoid rects
pub fn find_first_with_fallback_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> FirstRouteSearch {
    first_search(
        collision,
        graph,
        from,
        targets,
        fallback,
        opts,
        state,
        Relax::Strict,
        avoid,
        FIRST_TARGET_BUDGET,
        NODE_BUDGET,
    )
}

fn zone_filter<'a>(
    graph: &'a TransportGraph,
    state: &WorldState,
    endpoints: &[WorldTile],
    exemptions: &ZoneExempt,
) -> Option<ZoneFilter<'a>> {
    if exemptions.is_all() {
        return None;
    }
    graph
        .zones
        .as_ref()
        .map(|table| ZoneFilter::new(table, state.combat_level, endpoints, exemptions))
}

/// Partition only active destination zones that need completion permission.
/// Inactive wilderness portions and continuously escapable origin zones
/// retain the shared safe pass.
fn goal_zone_key(
    filter: &ZoneFilter<'_>,
    wilderness: &crate::transport::WildernessRules,
    goal: WorldTile,
) -> Vec<u16> {
    let mut key: Vec<_> = filter
        .blocking_at(wilderness, goal)
        .filter(|&index| !filter.origin_active(wilderness, index))
        .collect();
    key.sort_unstable();
    key
}

async fn goal_zone_key_cooperative(
    graph: &TransportGraph,
    filter: &ZoneFilter<'_>,
    goal: WorldTile,
) -> Result<Vec<u16>, SearchCursorError> {
    let mut key_len = 0usize;
    for index in filter.blocking_at(&graph.wilderness, goal) {
        work_unit().await;
        if !filter.origin_active(&graph.wilderness, index) {
            key_len = key_len
                .checked_add(1)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
        }
    }
    let mut key = Vec::new();
    key.try_reserve_exact(key_len)
        .map_err(|_| SearchCursorError::Allocation)?;
    for index in filter.blocking_at(&graph.wilderness, goal) {
        work_unit().await;
        if !filter.origin_active(&graph.wilderness, index) {
            key.push(index);
        }
    }
    sort_unstable_by_cooperative(&mut key, |left, right| left.cmp(right)).await;
    Ok(key)
}

/// Bound the goal-side proof, not the safe route. Deep zone goals usually
/// close after the goal and its eight neighbours, avoiding a world exhaust.
/// An open proof always leaves the full forward-search budget intact.
const ZONE_GOAL_PROOF_STEPS: usize = 64;

#[allow(clippy::too_many_arguments)]
fn zone_goal_proof(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    relax: Relax,
    zones: &ZoneFilter<'_>,
) -> ReverseReport {
    drive_to_completion(zone_goal_proof_cooperative(
        collision, graph, from, targets, opts, state, relax, zones, false,
    ))
    .unwrap_or_default()
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative zone proof borrows the unchanged collision, graph and search inputs"
)]
async fn zone_goal_proof_cooperative(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    relax: Relax,
    zones: &ZoneFilter<'_>,
    cooperative_storage: bool,
) -> Result<ReverseReport, SearchCursorError> {
    if targets.is_empty() {
        return Ok(ReverseReport::default());
    }
    for &goal in targets {
        work_unit().await;
        if !zones.destination_only_at(&graph.wilderness, goal) {
            return Ok(ReverseReport::default());
        }
    }
    let mut inventory_can_grow = false;
    for edge in graph
        .edges
        .iter()
        .chain(graph.teleports.iter().filter(|_| opts.allow_teleports))
    {
        work_unit().await;
        if !edge.item_returns.is_empty() && edge_allowed(state, edge, relax) {
            inventory_can_grow = true;
            break;
        }
    }
    let gates = ProofGates {
        collision,
        graph,
        state,
        essence: opts.essence.as_ref(),
        from,
        use_teleports: opts.allow_teleports,
        allow_wilderness: opts.allow_wilderness,
        relax,
        inventory_can_grow,
        cooperative_storage,
    };
    let mut closure = ReverseClosure::new(gates).await?;
    closure.zones = Some(zones);
    closure.targets = targets;
    for &goal in targets {
        work_unit().await;
        let proof = {
            let admission = std::pin::pin!(closure.admit(goal));
            admission.await?
        };
        if let Some(proof) = proof {
            return Ok(closure.report(proof));
        }
    }
    for _ in 0..ZONE_GOAL_PROOF_STEPS {
        work_unit().await;
        if let Some(proof) = closure.step().await? {
            return Ok(closure.report(proof));
        }
    }
    Ok(closure.report(ReverseProof::Open))
}

#[allow(clippy::too_many_arguments)]
fn first_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    relax: Relax,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
) -> FirstRouteSearch {
    if targets.is_empty() && !fallback.is_empty() {
        let mut result = first_search(
            collision,
            graph,
            from,
            fallback,
            &[],
            opts,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
        );
        result.fallback = Some(
            match std::mem::replace(&mut result.route, Err(RouteError::NoPath)) {
                Ok(route) => FallbackRoute::Routed(route),
                Err(error) => FallbackRoute::Failed(error),
            },
        );
        return result;
    }
    // Grants are fallback permissions, never permission to skip safe routing.
    let Some(filter) = zone_filter(graph, state, &[from], &ZoneExempt::NONE) else {
        return first_search_core(
            collision,
            graph,
            from,
            targets,
            fallback,
            opts,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
            None,
        );
    };
    if targets.is_empty() || targets.contains(&from) {
        return first_search_core(
            collision,
            graph,
            from,
            targets,
            fallback,
            opts,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
            Some(&filter),
        );
    }
    let mut partitions =
        std::collections::BTreeMap::<Vec<u16>, (Vec<WorldTile>, Vec<WorldTile>)>::new();
    for (goal, preferred) in targets
        .iter()
        .map(|&goal| (goal, true))
        .chain(fallback.iter().map(|&goal| (goal, false)))
    {
        let key = goal_zone_key(&filter, &graph.wilderness, goal);
        if !key.is_empty() || opts.zones != ZoneExempt::NONE {
            let partition = partitions.entry(key).or_default();
            if preferred {
                partition.0.push(goal);
            } else {
                partition.1.push(goal);
            }
        }
    }
    if partitions.is_empty() {
        return first_search_core(
            collision,
            graph,
            from,
            targets,
            fallback,
            opts,
            state,
            relax,
            avoid,
            budget,
            reachable_budget,
            Some(&filter),
        );
    }
    let run = |preferred: &[WorldTile], fallbacks: &[WorldTile], zones: Option<&ZoneFilter<'_>>| {
        if preferred.is_empty() {
            let pass = first_search_core(
                collision,
                graph,
                from,
                fallbacks,
                &[],
                opts,
                state,
                relax,
                avoid,
                budget,
                reachable_budget,
                zones,
            );
            FirstRouteSearch {
                route: Err(RouteError::NoPath),
                fallback: (!fallback.is_empty()).then_some(match pass.route {
                    Ok(route) => FallbackRoute::Routed(route),
                    Err(error) => FallbackRoute::Failed(error),
                }),
                settled: pass.settled,
                capacities: pass.capacities,
                proof: pass.proof,
                completion_partitions: 0,
            }
        } else {
            first_search_core(
                collision,
                graph,
                from,
                preferred,
                fallbacks,
                opts,
                state,
                relax,
                avoid,
                budget,
                reachable_budget,
                zones,
            )
        }
    };
    let mut merged = run(targets, fallback, Some(&filter));
    // Ordinary first-goal searches prefer safe arrivals. Ordered bank goals
    // also compare each destination's own-zone completion before transit grants.
    if !opts.prefer_target_order
        && (matches!(merged.fallback.as_ref(), Some(FallbackRoute::Routed(_)))
            || !matches!(
                merged.route,
                Err(RouteError::NoPath | RouteError::BudgetExhausted)
            ))
    {
        return merged;
    }
    for exemptions in [ZoneExempt::NONE, opts.zones]
        .into_iter()
        .take(1 + usize::from(opts.zones != ZoneExempt::NONE))
    {
        for (key, (preferred, fallbacks)) in &partitions {
            if key.is_empty() && exemptions == ZoneExempt::NONE {
                continue;
            }
            let endpoint = preferred
                .first()
                .or_else(|| fallbacks.first())
                .expect("a completion partition is nonempty");
            let completion = zone_filter(graph, state, &[from, *endpoint], &exemptions);
            merged.completion_partitions += 1;
            let mut result = run(preferred, fallbacks, completion.as_ref());
            // The shared mask remains live while this completion runs.
            result.capacities.zone_mask_words += filter.mask_words();
            merged.settled += result.settled;
            merged.capacities.include(result.capacities);
            if opts.prefer_target_order
                && matches!((&merged.route, &result.route), (Ok(old), Ok(new))
                    if old.ticks == new.ticks)
            {
                if let (Ok(old), Ok(new)) = (&merged.route, &result.route) {
                    if drive_to_completion(target_precedes(targets, new.dest, old.dest)) {
                        merged.route = result.route;
                    }
                }
            } else {
                choose_route(&mut merged.route, result.route);
            }
            if let Some(answer) = result.fallback {
                merge_fallback(&mut merged.fallback, answer);
            }
        }
        // Goal entry never requires granting unrelated danger-zone transit.
        if merged.route.is_ok() {
            merged.fallback = None;
            break;
        }
        if matches!(merged.fallback.as_ref(), Some(FallbackRoute::Routed(_))) {
            break;
        }
    }
    merged
}

fn route_order(route: &Route) -> (i32, i32, i32) {
    (route.dest.x, route.dest.z, route.dest.level)
}

fn choose_route(current: &mut Result<Route, RouteError>, candidate: Result<Route, RouteError>) {
    match candidate {
        Ok(route) => {
            if current.as_ref().map_or(true, |old| {
                route.ticks < old.ticks
                    || (route.ticks == old.ticks && route_order(&route) > route_order(old))
            }) {
                *current = Ok(route);
            }
        }
        Err(RouteError::BudgetExhausted) if current.is_err() => {
            *current = Err(RouteError::BudgetExhausted);
        }
        Err(_) => {}
    }
}

fn merge_fallback(current: &mut Option<FallbackRoute>, candidate: FallbackRoute) {
    match candidate {
        FallbackRoute::Routed(route) => {
            if !matches!(current, Some(FallbackRoute::Routed(old))
                if old.ticks < route.ticks || (old.ticks == route.ticks && route_order(old) >= route_order(&route)))
            {
                *current = Some(FallbackRoute::Routed(route));
            }
        }
        FallbackRoute::Undecided if !matches!(current, Some(FallbackRoute::Routed(_))) => {
            *current = Some(FallbackRoute::Undecided)
        }
        FallbackRoute::Failed(error)
            if current.is_none()
                || matches!(current, Some(FallbackRoute::Failed(RouteError::NoPath))) =>
        {
            *current = Some(FallbackRoute::Failed(error))
        }
        _ => {}
    }
}

/// The proof-carrying goal search behind [`find_first_with_fallback`]
/// (strict gates) and the single-target diagnoses ([`Relax`]). The
/// backward closures honor the same gates as the
/// forward search, so a strict proof never hides a relaxed route. Each goal
/// set's `budget` holds until its own proof shows it reachable, then
/// `reachable_budget` applies ([`FirstGoals`]).
#[allow(clippy::too_many_arguments)] // search surface plus relaxation/avoid/budgets
fn first_search_core(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    relax: Relax,
    avoid: &[AvoidRect],
    budget: usize,
    reachable_budget: usize,
    zones: Option<&ZoneFilter<'_>>,
) -> FirstRouteSearch {
    if targets.is_empty() {
        return FirstRouteSearch {
            route: Err(RouteError::NoPath),
            fallback: (!fallback.is_empty()).then_some(FallbackRoute::Undecided),
            settled: 0,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
            completion_partitions: 0,
        };
    }
    if targets.contains(&from) {
        return FirstRouteSearch {
            route: Ok(Route {
                legs: vec![Leg::Walk { tiles: vec![from] }],
                dest: from,
                ticks: 0.0,
            }),
            fallback: None,
            settled: 0,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
            completion_partitions: 0,
        };
    }

    let mut goal_proof_capacity = SearchCapacities::default();
    if fallback.is_empty() {
        if let Some(zones) = zones {
            let proof = zone_goal_proof(collision, graph, from, targets, opts, state, relax, zones);
            goal_proof_capacity = SearchCapacities {
                reverse: proof.seen,
                reverse_queue: proof.queue,
                zone_mask_words: zones.mask_words(),
                ..SearchCapacities::default()
            };
            if proof.proof == ReverseProof::Unreachable {
                return FirstRouteSearch {
                    route: Err(RouteError::NoPath),
                    fallback: None,
                    settled: 0,
                    capacities: goal_proof_capacity,
                    proof: proof.proof,
                    completion_partitions: 0,
                };
            }
        }
    }

    let gates = ProofGates {
        collision,
        graph,
        state,
        essence: opts.essence.as_ref(),
        from,
        use_teleports: opts.allow_teleports,
        allow_wilderness: opts.allow_wilderness,
        relax,
        inventory_can_grow: graph
            .edges
            .iter()
            .chain(graph.teleports.iter().filter(|_| opts.allow_teleports))
            .any(|edge| !edge.item_returns.is_empty() && edge_allowed(state, edge, relax)),
        cooperative_storage: false,
    };
    let first = match drive_to_completion(FirstGoals::new_cooperative(
        gates,
        targets,
        fallback,
        budget,
        reachable_budget,
        opts.prefer_target_order,
    )) {
        Ok(first) => first,
        Err(_) => {
            return FirstRouteSearch {
                route: Err(RouteError::BudgetExhausted),
                fallback: (!fallback.is_empty()).then_some(FallbackRoute::Undecided),
                settled: 0,
                capacities: SearchCapacities::default(),
                proof: ReverseProof::Open,
                completion_partitions: 0,
            };
        }
    };
    let mut goals = Goals::First(Box::new(first));
    // The goal sets hold the budgets.
    let search = drive_to_completion(search_kernel(
        collision,
        graph,
        from,
        CostModel::running(),
        reachable_budget,
        opts.allow_teleports,
        opts.allow_wilderness,
        state,
        opts.essence.as_ref(),
        relax,
        avoid,
        zones,
        &mut goals,
        None,
        false,
        None,
    ));
    let Goals::First(mut first) = goals else {
        unreachable!("a first-goal search keeps its goals");
    };
    if search.stop == SearchStop::Allocation {
        first.preferred.closed = Some(RouteError::BudgetExhausted);
        first.fallback.closed = Some(RouteError::BudgetExhausted);
    }
    let route_to = |(dest, cost): (WorldTile, TargetCost)| {
        let (legs, ticks) = reconstruct(
            dest,
            &search.came_from,
            graph,
            CostModel::running(),
            opts.essence.as_ref(),
        );
        debug_assert_eq!(ticks, cost.ticks);
        Route { legs, dest, ticks }
    };
    let route = match first.preferred.found {
        Some(goal) => Ok(route_to(goal)),
        None => Err(first.preferred.closed.unwrap_or(RouteError::NoPath)),
    };
    let fallback_route = if route.is_ok() || fallback.is_empty() {
        None
    } else {
        Some(match (first.fallback.found, first.fallback.closed) {
            (Some(goal), _) => FallbackRoute::Routed(route_to(goal)),
            (None, Some(error)) => FallbackRoute::Failed(error),
            (None, None) => FallbackRoute::Undecided,
        })
    };
    let mut capacities = search.capacities;
    capacities.include(goal_proof_capacity);
    FirstRouteSearch {
        route,
        fallback: fallback_route,
        settled: search.settled,
        capacities,
        proof: search.proof,
        completion_partitions: 0,
    }
}

/// Per-input results in caller order, with one predecessor tree for lazy
/// reconstruction. The graph is borrowed, not copied, and the heap/dist/done
/// scratch is dropped when the search returns.
pub struct RoutesToTargets<'a> {
    targets: &'a [WorldTile],
    results: Vec<Result<TargetCost, TargetError>>,
    came_from: Predecessors,
    graph: &'a TransportGraph,
    essence: Option<EssenceSession>,
    settled: usize,
    complete: bool,
    capacities: SearchCapacities,
    completions: Vec<(Vec<usize>, Predecessors)>,
}

impl RoutesToTargets<'_> {
    pub fn results(&self) -> &[Result<TargetCost, TargetError>] {
        &self.results
    }

    pub fn settled(&self) -> usize {
        self.settled
    }

    /// False only if the caller's deadline interrupted the all-target flood.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Per-request peak scratch entry capacities (the scratch is already
    /// dropped except for the predecessor tree needed by [`Self::route`]).
    pub fn scratch_capacities(&self) -> SearchCapacities {
        self.capacities
    }

    pub fn completion_partitions(&self) -> usize {
        self.completions.len()
    }

    /// Reconstruct only the chosen target, without cloning paths for the rest.
    pub fn route(&self, target_index: usize) -> Result<Route, TargetError> {
        let cost = self.results[target_index]?;
        let dest = self.targets[target_index];
        let tree = self
            .completions
            .iter()
            .rev()
            .find(|(indices, _)| indices.contains(&target_index))
            .map_or(&self.came_from, |(_, tree)| tree);
        let (legs, ticks) = reconstruct(
            dest,
            tree,
            self.graph,
            CostModel::running(),
            self.essence.as_ref(),
        );
        debug_assert_eq!(ticks, cost.ticks);
        Ok(Route { legs, dest, ticks })
    }
}

/// Search all targets with the same native options and 4M cap as [`find_with`].
pub fn find_many_with<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
) -> RoutesToTargets<'a> {
    find_many_with_avoid_bounded(
        collision,
        graph,
        from,
        targets,
        opts,
        state,
        &[],
        NODE_BUDGET,
    )
}

/// Shared bounded search. Duplicate targets retain separate input rows.
#[allow(clippy::too_many_arguments)]
pub fn find_many_with_avoid_bounded<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
) -> RoutesToTargets<'a> {
    find_many_with_avoid_bounded_until(
        collision, graph, from, targets, opts, state, avoid, budget, None,
    )
}

/// As above, with a caller-supplied completion deadline. The wall-clock
/// bound is caller policy; the search checks it every 256 heap pops and at
/// exit, reports unsettled targets, and leaves fallback choice to the caller.
#[allow(clippy::too_many_arguments)]
pub fn find_many_with_avoid_bounded_until<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    deadline: Option<Instant>,
) -> RoutesToTargets<'a> {
    let Some(filter) = zone_filter(graph, state, &[from], &ZoneExempt::NONE) else {
        return many_search(
            collision, graph, from, targets, opts, state, avoid, budget, deadline, None,
        );
    };
    let mut partitions = std::collections::BTreeMap::<Vec<u16>, Vec<usize>>::new();
    for (index, &target) in targets.iter().enumerate() {
        let key = goal_zone_key(&filter, &graph.wilderness, target);
        if !key.is_empty() || opts.zones != ZoneExempt::NONE {
            partitions.entry(key).or_default().push(index);
        }
    }
    if partitions.is_empty() {
        return many_search(
            collision,
            graph,
            from,
            targets,
            opts,
            state,
            avoid,
            budget,
            deadline,
            Some(&filter),
        );
    }
    let shared = many_search(
        collision,
        graph,
        from,
        targets,
        opts,
        state,
        avoid,
        budget,
        deadline,
        Some(&filter),
    );
    let mut results = shared.results;
    let mut settled = shared.settled;
    let mut complete = shared.complete;
    let mut capacities = shared.capacities;
    let shared_tree = if results.iter().any(Result::is_ok) {
        shared.came_from
    } else {
        Predecessors::empty()
    };
    let mut retained_capacity = shared_tree.capacity();
    let mut completions = Vec::with_capacity(partitions.len());
    for exemptions in [ZoneExempt::NONE, opts.zones]
        .into_iter()
        .take(1 + usize::from(opts.zones != ZoneExempt::NONE))
    {
        for (key, partition) in &partitions {
            if key.is_empty() && exemptions == ZoneExempt::NONE {
                continue;
            }
            let indices: Vec<_> = partition
                .iter()
                .copied()
                .filter(|&index| {
                    matches!(
                        results[index],
                        Err(TargetError::NoPath | TargetError::BudgetExhausted)
                    )
                })
                .collect();
            if indices.is_empty() {
                continue;
            }
            let goals: Vec<_> = indices.iter().map(|&index| targets[index]).collect();
            let completion = zone_filter(graph, state, &[from, goals[0]], &exemptions);
            let pass = many_search(
                collision,
                graph,
                from,
                &goals,
                opts,
                state,
                avoid,
                budget,
                deadline,
                completion.as_ref(),
            );
            for (&index, &result) in indices.iter().zip(&pass.results) {
                results[index] = result;
            }
            settled += pass.settled;
            complete &= pass.complete;
            let mut peak = pass.capacities;
            peak.predecessors += retained_capacity;
            peak.zone_mask_words += filter.mask_words();
            capacities.include(peak);
            let tree = if pass.results.iter().any(Result::is_ok) {
                retained_capacity += pass.came_from.capacity();
                pass.came_from
            } else {
                Predecessors::empty()
            };
            completions.push((indices, tree));
        }
    }
    RoutesToTargets {
        targets,
        results,
        came_from: shared_tree,
        graph,
        essence: opts.essence,
        settled,
        complete,
        capacities,
        completions,
    }
}

#[allow(clippy::too_many_arguments)]
fn many_search<'a>(
    collision: &WorldCollision,
    graph: &'a TransportGraph,
    from: WorldTile,
    targets: &'a [WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    deadline: Option<Instant>,
    zones: Option<&ZoneFilter<'_>>,
) -> RoutesToTargets<'a> {
    let mut unique = HashMap::with_capacity(targets.len());
    let mut input_indices = Vec::with_capacity(targets.len());
    let mut costs = Vec::new();
    let mut proof_capacity = SearchCapacities::default();
    for &target in targets {
        let index = *unique.entry(target).or_insert_with(|| {
            costs.push(if target == from {
                Ok(TargetCost {
                    ticks: 0.0,
                    settled_at: 0,
                })
            } else if let Some(zones) = zones {
                let proof = zone_goal_proof(
                    collision,
                    graph,
                    from,
                    &[target],
                    opts,
                    state,
                    Relax::Strict,
                    zones,
                );
                proof_capacity.reverse = proof_capacity.reverse.max(proof.seen);
                proof_capacity.reverse_queue = proof_capacity.reverse_queue.max(proof.queue);
                if proof.proof == ReverseProof::Unreachable {
                    Err(TargetError::NoPath)
                } else {
                    Err(TargetError::NotSettled)
                }
            } else {
                Err(TargetError::NotSettled)
            });
            costs.len() - 1
        });
        input_indices.push(index);
    }
    let remaining = costs
        .iter()
        .filter(|&&result| result == Err(TargetError::NotSettled))
        .count();
    let mut goals = Goals::Many {
        unique: &unique,
        costs: &mut costs,
        remaining,
    };
    let mut search = if remaining == 0 {
        KernelOutcome::empty()
    } else {
        drive_to_completion(search_kernel(
            collision,
            graph,
            from,
            CostModel::running(),
            budget,
            opts.allow_teleports,
            opts.allow_wilderness,
            state,
            opts.essence.as_ref(),
            Relax::Strict,
            avoid,
            zones,
            &mut goals,
            deadline,
            false,
            None,
        ))
    };
    proof_capacity.zone_mask_words = zones.map_or(0, ZoneFilter::mask_words);
    search.capacities.include(proof_capacity);
    let error = match search.stop {
        SearchStop::Exhausted => TargetError::NoPath,
        SearchStop::Budget | SearchStop::Allocation => TargetError::BudgetExhausted,
        SearchStop::Deadline | SearchStop::Completed => TargetError::NotSettled,
    };
    for result in &mut costs {
        if *result == Err(TargetError::NotSettled) {
            *result = Err(error);
        }
    }
    RoutesToTargets {
        targets,
        results: input_indices
            .into_iter()
            .map(|index| costs[index])
            .collect(),
        came_from: search.came_from,
        graph,
        essence: opts.essence,
        settled: search.settled,
        capacities: search.capacities,
        complete: search.stop != SearchStop::Deadline,
        completions: Vec::new(),
    }
}

/// Diagnose a refused single-target route with zones active along the best
/// route after lifting zone restrictions. The strict search must fail with
/// `NoPath`, and an all-zone-exempt search must find a route before its
/// transitions can serve as the witness.
#[allow(clippy::too_many_arguments)]
pub fn find_blocking_zones(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<ZoneKey>> {
    let filter = zone_filter(graph, state, &[from, to], &opts.zones)?;
    if !matches!(
        find_with_avoid(collision, graph, from, to, opts, state, avoid),
        Err(RouteError::NoPath)
    ) {
        return None;
    }
    // The strict call has already exhausted its safe and completion stages;
    // run only the final no-zone search instead of repeating those stages.
    let route = run_single_search(
        collision,
        graph,
        from,
        to,
        CostModel::running(),
        NODE_BUDGET,
        opts.allow_teleports,
        opts.allow_wilderness,
        state,
        opts.essence.as_ref(),
        avoid,
        None,
    )
    .ok()?;
    let mut blockers = HashSet::new();
    let mut previous = from;
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                for &arrival in tiles.iter().skip(1) {
                    blockers.extend(filter.blocking_transition_at(
                        &graph.wilderness,
                        previous,
                        arrival,
                        &|| arrival == to,
                    ));
                    previous = arrival;
                }
            }
            Leg::Transport { edge } => {
                let arrival = edge.to;
                blockers.extend(filter.blocking_transition_at(
                    &graph.wilderness,
                    previous,
                    arrival,
                    &|| arrival == to,
                ));
                previous = arrival;
            }
        }
    }
    blocking_zone_keys(graph.zones.as_ref()?, blockers)
}

/// Multi-goal refusal attribution uses the union of reachable frontiers for
/// the selected target set, grouped by active destination zones. A goal's
/// own zones allow arrival, never transit to targets outside those zones.
#[allow(clippy::too_many_arguments)]
pub fn find_first_blocking_zones(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<ZoneKey>> {
    let mut filter = zone_filter(graph, state, &[from], &opts.zones)?;
    if !matches!(
        find_first_with_avoid(collision, graph, from, targets, opts, state, avoid).into_route(),
        Err(RouteError::NoPath)
    ) {
        return None;
    }
    let relaxed = FindOptions {
        zones: ZoneExempt::all(),
        ..opts
    };
    find_first_with_avoid(collision, graph, from, targets, relaxed, state, avoid)
        .into_route()
        .ok()?;
    let mut partitions: std::collections::BTreeMap<Vec<u16>, Vec<WorldTile>> =
        std::collections::BTreeMap::new();
    for &goal in targets {
        partitions
            .entry(goal_zone_key(&filter, &graph.wilderness, goal))
            .or_default()
            .push(goal);
    }
    let mut blockers = HashSet::new();
    for (key, goals) in partitions {
        filter.select_destination((!key.is_empty()).then_some(goals[0]));
        if !blocking_frontier(
            collision,
            graph,
            from,
            &goals,
            opts,
            state,
            avoid,
            &filter,
            &mut blockers,
        ) {
            return None;
        }
    }
    blocking_zone_keys(graph.zones.as_ref()?, blockers)
}

#[allow(clippy::too_many_arguments)]
fn blocking_frontier(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    filter: &ZoneFilter<'_>,
    blockers: &mut HashSet<u16>,
) -> bool {
    let mut goals = Goals::Frontier {
        targets: FirstTargets::new(targets),
        blockers,
    };
    let search = drive_to_completion(search_kernel(
        collision,
        graph,
        from,
        CostModel::running(),
        NODE_BUDGET,
        opts.allow_teleports,
        opts.allow_wilderness,
        state,
        opts.essence.as_ref(),
        Relax::Strict,
        avoid,
        Some(filter),
        &mut goals,
        None,
        false,
        None,
    ));
    search.stop == SearchStop::Exhausted
}

fn blocking_zone_keys(table: &ZoneTable, blockers: HashSet<u16>) -> Option<Vec<ZoneKey>> {
    let mut keys: Vec<_> = blockers.into_iter().map(|index| table.key(index)).collect();
    keys.sort_unstable_by_key(|key| match key {
        ZoneKey::Zone(index) => (0, *index),
        ZoneKey::Group(index) => (1, *index),
    });
    keys.dedup();
    (!keys.is_empty()).then_some(keys)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative kernel borrows the unchanged frontier search inputs"
)]
async fn blocking_frontier_cooperative(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    filter: &ZoneFilter<'_>,
    blockers: &mut HashSet<u16>,
    budget: usize,
    progress: Option<&AtomicUsize>,
) -> Result<(bool, usize), SearchCursorError> {
    let mut goals = Goals::Frontier {
        targets: FirstTargets::new_cooperative(targets).await?,
        blockers,
    };
    let search = search_kernel(
        collision,
        graph,
        from,
        CostModel::running(),
        budget,
        options.allow_teleports,
        options.allow_wilderness,
        state,
        options.essence.as_ref(),
        Relax::Strict,
        avoid,
        Some(filter),
        &mut goals,
        None,
        true,
        progress,
    )
    .await;
    if search.stop == SearchStop::Allocation {
        return Err(SearchCursorError::Allocation);
    }
    Ok((search.stop == SearchStop::Exhausted, search.settled))
}

async fn blocking_zone_keys_cooperative(
    table: &ZoneTable,
    blockers: HashSet<u16>,
) -> Result<Option<Box<[ZoneKey]>>, SearchCursorError> {
    let mut keys = Vec::new();
    keys.try_reserve_exact(blockers.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    for index in blockers {
        work_unit().await;
        keys.push(table.key(index));
    }
    sort_unstable_by_cooperative(&mut keys, |left, right| match (left, right) {
        (ZoneKey::Zone(left), ZoneKey::Zone(right))
        | (ZoneKey::Group(left), ZoneKey::Group(right)) => left.cmp(right),
        (ZoneKey::Zone(_), ZoneKey::Group(_)) => Ordering::Less,
        (ZoneKey::Group(_), ZoneKey::Zone(_)) => Ordering::Greater,
    })
    .await;
    dedup_sorted_by_cooperative(&mut keys).await;
    Ok((!keys.is_empty()).then(|| keys.into_boxed_slice()))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the blocker diagnosis borrows the unchanged single-target inputs"
)]
async fn blocking_single_target_cooperative(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    target: WorldTile,
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    filter: &ZoneFilter<'_>,
    budget: usize,
    progress: Option<&AtomicUsize>,
) -> Result<(Option<Box<[ZoneKey]>>, usize), SearchCursorError> {
    let (strict, mut forward_settled) = single_cursor_search(
        collision, graph, from, target, options, state, avoid, budget, progress,
    )
    .await?;
    if !matches!(&strict, Err(RouteError::NoPath)) {
        return Ok((None, forward_settled));
    }
    drop(strict);

    let (relaxed, _, relaxed_settled) = run_single_compact_search(
        collision, graph, from, target, options, state, avoid, budget, None, None, progress,
    )
    .await?;
    forward_settled = forward_settled
        .checked_add(relaxed_settled)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let Ok(route) = relaxed else {
        return Ok((None, forward_settled));
    };

    let table = graph
        .zones
        .as_ref()
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let mut blockers = HashSet::new();
    blockers
        .try_reserve(table.zones().len())
        .map_err(|_| SearchCursorError::Allocation)?;
    let mut previous = from;
    for leg in route.cursor(graph) {
        work_unit().await;
        match leg {
            CompactRouteLeg::Walk(tiles) => {
                for &arrival in tiles.iter().skip(1) {
                    work_unit().await;
                    for zone in
                        filter.blocking_transition_at(&graph.wilderness, previous, arrival, &|| {
                            arrival == target
                        })
                    {
                        work_unit().await;
                        blockers.insert(zone);
                    }
                    previous = arrival;
                }
            }
            CompactRouteLeg::Transport(step) => {
                let arrival = step.actual_to();
                for zone in
                    filter.blocking_transition_at(&graph.wilderness, previous, arrival, &|| {
                        arrival == target
                    })
                {
                    work_unit().await;
                    blockers.insert(zone);
                }
                previous = arrival;
            }
        }
    }
    drop(route);
    Ok((
        blocking_zone_keys_cooperative(table, blockers).await?,
        forward_settled,
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative blocker diagnosis borrows the unchanged collision, graph and search inputs"
)]
async fn blocking_cursor_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    diagnosis: BlockerDiagnosis,
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    mut filter: ZoneFilterOwned,
    budget: usize,
    progress: Option<&AtomicUsize>,
) -> Result<(Option<Box<[ZoneKey]>>, usize), SearchCursorError> {
    if diagnosis == BlockerDiagnosis::SingleTarget && targets.len() != 1 {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    if options.zones.is_all() {
        return Ok((None, 0));
    }
    let Some(table) = graph.zones.as_ref() else {
        return Ok((None, 0));
    };
    if filter.zone_count() != table.zones().len() {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    complete_owned_zone_filter(&mut filter, table).await?;
    if diagnosis == BlockerDiagnosis::SingleTarget {
        let view = filter
            .view(table)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        return blocking_single_target_cooperative(
            collision, graph, from, targets[0], options, state, avoid, &view, budget, progress,
        )
        .await;
    }
    let strict = first_cursor_search(
        collision,
        graph,
        from,
        targets,
        &[],
        options,
        state,
        Relax::Strict,
        avoid,
        FIRST_TARGET_BUDGET.min(budget),
        budget,
        progress,
    )
    .await?;
    let mut forward_settled = strict.settled();
    let (strict_preferred, _) = strict.into_routes();
    if !matches!(strict_preferred, Err(RouteError::NoPath)) {
        return Ok((None, forward_settled));
    }
    let relaxed = FindOptions {
        zones: ZoneExempt::all(),
        ..options
    };
    let relaxed_search = first_cursor_search(
        collision,
        graph,
        from,
        targets,
        &[],
        relaxed,
        state,
        Relax::Strict,
        avoid,
        FIRST_TARGET_BUDGET.min(budget),
        budget,
        progress,
    )
    .await?;
    forward_settled = forward_settled
        .checked_add(relaxed_search.settled())
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let (relaxed_preferred, _) = relaxed_search.into_routes();
    if relaxed_preferred.is_err() {
        return Ok((None, forward_settled));
    }
    // This result is only a reachability witness. Do not retain its walk
    // buffers while the blocker frontier starts another full-budget search.
    drop(relaxed_preferred);
    let mut partitions = Vec::new();
    partitions
        .try_reserve_exact(targets.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    for (index, &goal) in targets.iter().enumerate() {
        work_unit().await;
        let filter_view = filter
            .view(table)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let key = goal_zone_key_cooperative(graph, &filter_view, goal).await?;
        partitions.push((key, index, goal));
    }
    sort_unstable_by_cooperative(&mut partitions, |left, right| {
        left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1))
    })
    .await;
    let mut blockers = HashSet::new();
    blockers
        .try_reserve(table.zones().len())
        .map_err(|_| SearchCursorError::Allocation)?;
    let mut start = 0usize;
    while start < partitions.len() {
        work_unit().await;
        let mut end = start + 1;
        while end < partitions.len() && partitions[end].0 == partitions[start].0 {
            work_unit().await;
            end += 1;
        }
        let mut goals = Vec::new();
        goals
            .try_reserve_exact(end - start)
            .map_err(|_| SearchCursorError::Allocation)?;
        for (_, _, goal) in &partitions[start..end] {
            work_unit().await;
            goals.push(*goal);
        }
        let key = &partitions[start].0;
        filter.select_destination((!key.is_empty()).then_some(goals[0]));
        let view = filter
            .view(table)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let (exhausted, settled) = blocking_frontier_cooperative(
            collision,
            graph,
            from,
            &goals,
            options,
            state,
            avoid,
            &view,
            &mut blockers,
            NODE_BUDGET.min(budget),
            progress,
        )
        .await?;
        forward_settled = forward_settled
            .checked_add(settled)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        if !exhausted {
            return Ok((None, forward_settled));
        }
        start = end;
    }
    Ok((
        blocking_zone_keys_cooperative(table, blockers).await?,
        forward_settled,
    ))
}

/// A missing `item_req`/`worn_req` fact the BankBudget session must
/// supply before a strict [`find_with`] can route: an `item_req` stack
/// count the state cannot prove, or a `worn_req` list (any-of) with no
/// worn alternative. A missing `worn_all_req` is not fetchable and remains
/// strict in every diagnostic search. [`missing_item_reqs`] names only the
/// carryable facts; [`find`]/[`find_with`] never relax an edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MissingReq {
    /// The edge needs `count` of obj `id` carried (`item_req`).
    Carry { id: i32, count: i32 },
    /// The edge needs any one of `ids` worn (`worn_req` is any-of): the
    /// session fetches whichever alternative the player can obtain.
    WearAny { ids: Vec<i32> },
}

/// Diagnose a strict [`find_with`] `NoPath`: run the same search with
/// only the `item_req`/`worn_req` gates ignored, and collect every such
/// fact on the relaxed route that `state` could not prove. `worn_all_req`
/// stays strict: an inventory or bank alternative cannot satisfy a
/// conjunctive equipment prerequisite. When that search fails too, a
/// crossing may need a fetched item *and* a quest-stage gate the evidence
/// leaves `Unknown`: the search runs once more also crossing `Unknown`
/// gates, and names only the carry/wear facts on that route
/// ([`find_unresolved_quest_gates`] names its gates), so neither diagnosis
/// hides behind the other's gate. Returns `None` when no relaxed search
/// routes, or the one that does needs no item — a skill/quest/varp gate, a
/// `False` stage gate or a plain hole in the graph blocks, or only a journal
/// read can help, and no fetch-and-wear session can. This is the BankBudget
/// session's diagnosis arm ([`crate::bank_fetch::plan_bank_fetch`]);
/// [`find`] and [`find_with`] themselves never ignore an item gate — missing
/// facts still fail closed. The relaxed search carries the backward
/// [`ReverseProof`], so an unreachable diagnosis target (a solid tile no
/// transport lands on, a sealed pocket) costs about its own backward region
/// instead of a relaxed flood of everything reachable from `from`.
pub fn find_missing_item_reqs(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
) -> Option<Vec<MissingReq>> {
    find_missing_item_reqs_with_avoid(collision, graph, from, to, opts, state, &[])
}

/// [`find_missing_item_reqs`] with the same avoidance semantics as
/// [`find_with_avoid`].
pub fn find_missing_item_reqs_with_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<MissingReq>> {
    find_missing_item_reqs_with_avoid_bounded(
        collision,
        graph,
        from,
        to,
        opts,
        state,
        avoid,
        NODE_BUDGET,
    )
}

/// [`find_missing_item_reqs_with_avoid`] with an injectable node-expansion
/// cap. Production inspect uses the default-budget form.
#[allow(clippy::too_many_arguments)] // collision/graph/tiles/opts/state/avoid/budget surface
pub fn find_missing_item_reqs_with_avoid_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
) -> Option<Vec<MissingReq>> {
    let search = |relax| {
        first_search(
            collision,
            graph,
            from,
            std::slice::from_ref(&to),
            &[],
            opts,
            state,
            relax,
            avoid,
            budget,
            budget,
        )
        .into_route()
        .ok()
    };
    if let Some(route) = search(Relax::CarryWorn) {
        return Some(missing_item_reqs(&route, state));
    }
    // Only an `Unknown` stage gate can make the combined search differ.
    if !graph
        .edges
        .iter()
        .chain(&graph.teleports)
        .any(|edge| state.quest_gates(edge) == Truth::Unknown)
    {
        return None;
    }
    let missing = missing_item_reqs(&search(Relax::CarryWornUnknownQuest)?, state);
    (!missing.is_empty()).then_some(missing)
}

/// [`find_missing_item_reqs_with_avoid`] over a first-goal set, as
/// [`find_first_with_fallback_avoid`] searches it: one relaxed search over
/// the preferred `targets` with the `fallback` set, plus one over the
/// fallback alone only when the shared search leaves it
/// [`FallbackRoute::Undecided`]. The diagnosis is the chosen route's: the
/// cheapest preferred goal's when one routes relaxed, else the fallback
/// set's. The searches keep the strict first-goal budgets
/// ([`FIRST_TARGET_BUDGET`] unless the backward proof shows a goal
/// reachable), so a goal set costs what its strict search may, never one
/// diagnosis per goal. The `Unknown` stage-gate retry and the `None` cases
/// are [`find_missing_item_reqs`]'s.
#[allow(clippy::too_many_arguments)] // first-goal search surface plus avoid rects
pub fn find_first_missing_item_reqs_with_avoid(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<MissingReq>> {
    let search = |relax| {
        let first = |targets: &[WorldTile], fallback: &[WorldTile]| {
            first_search(
                collision,
                graph,
                from,
                targets,
                fallback,
                opts,
                state,
                relax,
                avoid,
                FIRST_TARGET_BUDGET,
                NODE_BUDGET,
            )
            .into_routes()
        };
        match first(targets, fallback) {
            (Ok(route), _) | (Err(_), Some(FallbackRoute::Routed(route))) => Some(route),
            (Err(_), Some(FallbackRoute::Undecided)) => first(fallback, &[]).0.ok(),
            (Err(_), Some(FallbackRoute::Failed(_)) | None) => None,
        }
    };
    if let Some(route) = search(Relax::CarryWorn) {
        return Some(missing_item_reqs(&route, state));
    }
    if !graph
        .edges
        .iter()
        .chain(&graph.teleports)
        .any(|edge| state.quest_gates(edge) == Truth::Unknown)
    {
        return None;
    }
    let missing = missing_item_reqs(&search(Relax::CarryWornUnknownQuest)?, state);
    (!missing.is_empty()).then_some(missing)
}

/// Initial carry supply needed by the whole route, and missing worn gates.
/// Consumed counts accumulate; credits from charge downgrades reduce later
/// demand. A held gate needs its stack intact at the point where it is used.
/// If anything is missing, carry targets include already held gates so the
/// bank planner can reserve a carried copy when choosing a worn alternative.
/// Counts are total initial supply; BankBudget withdraws only the shortage.
pub fn missing_item_reqs(route: &Route, state: &WorldState) -> Vec<MissingReq> {
    let mut required = HashMap::<i32, i32>::new();
    let mut missing = Vec::new();
    for leg in route.legs.iter().rev() {
        let Leg::Transport { edge } = leg else {
            continue;
        };
        // Work backwards so a balance-dependent debit has one exact
        // inverse and earlier charge returns reduce only later demand.
        for &(id, count) in &edge.item_returns {
            let demand = required.entry(id).or_default();
            *demand = demand.saturating_sub(count).max(0);
        }
        for &(id, packed_count) in &edge.consumed_req {
            let demand = required.entry(id).or_default();
            *demand = edge.required_supply_before(id, packed_count, *demand);
        }
        for &(id, count) in &edge.item_req {
            let demand = required.entry(id).or_default();
            *demand = (*demand).max(count);
        }
        if !edge.worn_req.is_empty() && !edge.worn_req.iter().any(|id| state.worn.contains(id)) {
            missing.push(MissingReq::WearAny {
                ids: edge.worn_req.clone(),
            });
        }
    }
    if missing.is_empty()
        && required
            .iter()
            .all(|(id, &count)| state.inv.get(id).copied().unwrap_or(0) >= count)
    {
        return missing;
    }
    missing.extend(
        required
            .into_iter()
            .filter(|&(_, count)| count > 0)
            .map(|(id, count)| MissingReq::Carry { id, count }),
    );
    missing.sort_by_key(|r| match r {
        MissingReq::Carry { id, .. } => (*id, 0),
        MissingReq::WearAny { ids } => (ids.first().copied().unwrap_or(0), 1),
    });
    missing.dedup();
    missing
}

fn missing_demand_slot(
    required: &mut HashMap<i32, i32>,
    id: i32,
) -> Result<&mut i32, SearchCursorError> {
    if !required.contains_key(&id) {
        required
            .try_reserve(1)
            .map_err(|_| SearchCursorError::Allocation)?;
        required.insert(id, 0);
    }
    required
        .get_mut(&id)
        .ok_or(SearchCursorError::ResultNotRepresentable)
}

fn push_missing_req(
    missing: &mut Vec<MissingReqEntry>,
    item: MissingReqEntry,
    maximum: usize,
) -> Result<(), SearchCursorError> {
    if missing.len() >= maximum {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    missing
        .try_reserve(1)
        .map_err(|_| SearchCursorError::Allocation)?;
    missing.push(item);
    Ok(())
}

async fn sort_missing_stably(
    missing: &mut [MissingReqEntry],
    graph: &TransportGraph,
) -> Result<(), SearchCursorError> {
    let mut order = Vec::new();
    order
        .try_reserve_exact(missing.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    for index in 0..missing.len() {
        work_unit().await;
        order.push(index);
    }
    sort_unstable_by_cooperative(&mut order, |left, right| {
        let key = |request: MissingReqEntry| match request {
            MissingReqEntry::Carry { id, .. } => (id, 0),
            MissingReqEntry::WearAny { packed_edge, .. } => (
                diagnostic_edge(graph, packed_edge)
                    .and_then(|edge| edge.worn_req.first().copied())
                    .unwrap_or(0),
                1,
            ),
        };
        key(missing[*left])
            .cmp(&key(missing[*right]))
            .then_with(|| {
                missing_entry_order(missing[*left]).cmp(&missing_entry_order(missing[*right]))
            })
    })
    .await;

    let mut destination_for_source = Vec::new();
    destination_for_source
        .try_reserve_exact(order.len())
        .map_err(|_| SearchCursorError::Allocation)?;
    for _ in 0..order.len() {
        work_unit().await;
        destination_for_source.push(0);
    }
    for (destination, &source) in order.iter().enumerate() {
        work_unit().await;
        destination_for_source[source] = destination;
    }
    for index in 0..missing.len() {
        work_unit().await;
        while destination_for_source[index] != index {
            work_unit().await;
            let destination = destination_for_source[index];
            missing.swap(index, destination);
            destination_for_source.swap(index, destination);
        }
    }
    Ok(())
}

const DIAGNOSTIC_TELEPORT_BIT: u32 = 1 << 31;
const DIAGNOSTIC_EDGE_INDEX_MASK: u32 = !DIAGNOSTIC_TELEPORT_BIT;
fn missing_entry_order(entry: MissingReqEntry) -> u32 {
    match entry {
        MissingReqEntry::Carry { .. } => 0,
        MissingReqEntry::WearAny { order, .. } => order,
    }
}

fn diagnostic_edge(graph: &TransportGraph, packed_edge: u32) -> Option<&TransportEdge> {
    let index = usize::try_from(packed_edge & DIAGNOSTIC_EDGE_INDEX_MASK).ok()?;
    if packed_edge & DIAGNOSTIC_TELEPORT_BIT != 0 {
        graph.teleports.get(index)
    } else {
        graph.edges.get(index)
    }
}

async fn missing_entries_equal(
    left: MissingReqEntry,
    right: MissingReqEntry,
    graph: &TransportGraph,
) -> Result<bool, SearchCursorError> {
    match (left, right) {
        (
            MissingReqEntry::Carry {
                id: left_id,
                count: left_count,
            },
            MissingReqEntry::Carry {
                id: right_id,
                count: right_count,
            },
        ) => Ok(left_id == right_id && left_count == right_count),
        (
            MissingReqEntry::WearAny {
                packed_edge: left_edge,
                ..
            },
            MissingReqEntry::WearAny {
                packed_edge: right_edge,
                ..
            },
        ) => {
            let left = &diagnostic_edge(graph, left_edge)
                .ok_or(SearchCursorError::ResultNotRepresentable)?
                .worn_req;
            let right = &diagnostic_edge(graph, right_edge)
                .ok_or(SearchCursorError::ResultNotRepresentable)?
                .worn_req;
            if left.len() != right.len() {
                return Ok(false);
            }
            for (left_id, right_id) in left.iter().zip(right) {
                work_unit().await;
                if left_id != right_id {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn dedup_missing_stably(
    missing: &mut Vec<MissingReqEntry>,
    graph: &TransportGraph,
) -> Result<(), SearchCursorError> {
    let mut kept = 0usize;
    let mut scanned = 0usize;
    while scanned < missing.len() {
        work_unit().await;
        let duplicate =
            kept > 0 && missing_entries_equal(missing[kept - 1], missing[scanned], graph).await?;
        if !duplicate {
            if kept != scanned {
                missing[kept] = missing[scanned];
            }
            kept += 1;
        }
        scanned += 1;
    }
    missing.truncate(kept);
    Ok(())
}

async fn edge_backtrace_from_predecessors(
    origin: WorldTile,
    destination: WorldTile,
    came_from: &Predecessors,
) -> Result<Vec<u32>, SearchCursorError> {
    let mut trace = came_from
        .backtrace(destination)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    let mut edge_count = 0usize;
    while let Some(step) = trace.next() {
        work_unit().await;
        if matches!(
            step,
            BacktraceStep::Transport { .. } | BacktraceStep::Teleport { .. }
        ) {
            edge_count = edge_count
                .checked_add(1)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
        }
    }
    if trace.current_tile() != origin {
        return Err(SearchCursorError::ResultNotRepresentable);
    }

    let mut edges = Vec::new();
    edges
        .try_reserve_exact(edge_count)
        .map_err(|_| SearchCursorError::Allocation)?;
    let mut trace = came_from
        .backtrace(destination)
        .ok_or(SearchCursorError::ResultNotRepresentable)?;
    while let Some(step) = trace.next() {
        work_unit().await;
        let packed = match step {
            BacktraceStep::Transport { index, .. } => {
                let index =
                    u32::try_from(index).map_err(|_| SearchCursorError::ResultNotRepresentable)?;
                if index & DIAGNOSTIC_TELEPORT_BIT != 0 {
                    return Err(SearchCursorError::ResultNotRepresentable);
                }
                index
            }
            BacktraceStep::Teleport { index, .. } => {
                let index =
                    u32::try_from(index).map_err(|_| SearchCursorError::ResultNotRepresentable)?;
                if index & DIAGNOSTIC_TELEPORT_BIT != 0 {
                    return Err(SearchCursorError::ResultNotRepresentable);
                }
                index | DIAGNOSTIC_TELEPORT_BIT
            }
            BacktraceStep::Walk { .. } | BacktraceStep::EssenceReturn { .. } => continue,
        };
        edges.push(packed);
    }
    if edges.len() != edge_count || trace.current_tile() != origin {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    Ok(edges)
}

struct MissingReqAccumulator {
    required: HashMap<i32, i32>,
    missing: Vec<MissingReqEntry>,
    worn_ids: usize,
    next_order: u32,
    max_requests: usize,
    max_carry_ids: usize,
    max_worn_ids: usize,
}

impl MissingReqAccumulator {
    fn new(
        max_requests: usize,
        max_carry_ids: usize,
        max_worn_ids: usize,
        edge_count: usize,
    ) -> Result<Self, SearchCursorError> {
        // Each traversed edge contributes at most one WearAny entry.
        // Carried requirements are folded into one row per item ID.
        let max_requests = max_requests.min(
            max_carry_ids
                .checked_add(edge_count)
                .ok_or(SearchCursorError::ResultNotRepresentable)?,
        );
        let mut required = HashMap::<i32, i32>::new();
        required
            .try_reserve(max_carry_ids)
            .map_err(|_| SearchCursorError::Allocation)?;
        let mut missing = Vec::new();
        missing
            .try_reserve_exact(max_requests)
            .map_err(|_| SearchCursorError::Allocation)?;
        Ok(Self {
            required,
            missing,
            worn_ids: 0,
            next_order: 0,
            max_requests,
            max_carry_ids,
            max_worn_ids,
        })
    }

    async fn include_edge(
        &mut self,
        edge: &TransportEdge,
        packed_edge: u32,
        state: &WorldState,
    ) -> Result<(), SearchCursorError> {
        let order = self.next_order;
        self.next_order = self
            .next_order
            .checked_add(1)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        for &(id, count) in &edge.item_returns {
            work_unit().await;
            let demand = missing_demand_slot(&mut self.required, id)?;
            *demand = demand.saturating_sub(count).max(0);
            if self.required.len() > self.max_carry_ids {
                return Err(SearchCursorError::ResultNotRepresentable);
            }
        }
        for &(id, packed_count) in &edge.consumed_req {
            work_unit().await;
            let demand = missing_demand_slot(&mut self.required, id)?;
            *demand = edge.required_supply_before(id, packed_count, *demand);
            if self.required.len() > self.max_carry_ids {
                return Err(SearchCursorError::ResultNotRepresentable);
            }
        }
        for &(id, count) in &edge.item_req {
            work_unit().await;
            let demand = missing_demand_slot(&mut self.required, id)?;
            *demand = (*demand).max(count);
            if self.required.len() > self.max_carry_ids {
                return Err(SearchCursorError::ResultNotRepresentable);
            }
        }
        if !edge.worn_req.is_empty() {
            let mut worn = false;
            for id in &edge.worn_req {
                work_unit().await;
                if state.worn.contains(id) {
                    worn = true;
                    break;
                }
            }
            if !worn {
                self.worn_ids = self
                    .worn_ids
                    .checked_add(edge.worn_req.len())
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                if self.worn_ids > self.max_worn_ids {
                    return Err(SearchCursorError::ResultNotRepresentable);
                }
                push_missing_req(
                    &mut self.missing,
                    MissingReqEntry::WearAny { packed_edge, order },
                    self.max_requests,
                )?;
            }
        }
        Ok(())
    }

    async fn finish(
        mut self,
        state: &WorldState,
        graph: &TransportGraph,
    ) -> Result<Vec<MissingReqEntry>, SearchCursorError> {
        let mut all_required_held = self.missing.is_empty();
        if all_required_held {
            for (&id, &count) in &self.required {
                work_unit().await;
                if state.inv.get(&id).copied().unwrap_or(0) < count {
                    all_required_held = false;
                    break;
                }
            }
        }
        if all_required_held {
            return Ok(self.missing);
        }
        for (id, count) in self.required {
            work_unit().await;
            if count > 0 {
                push_missing_req(
                    &mut self.missing,
                    MissingReqEntry::Carry { id, count },
                    self.max_requests,
                )?;
            }
        }
        sort_missing_stably(&mut self.missing, graph).await?;
        dedup_missing_stably(&mut self.missing, graph).await?;
        Ok(self.missing)
    }
}

async fn missing_item_reqs_edges(
    route_edges: &[u32],
    graph: &TransportGraph,
    state: &WorldState,
    max_requests: usize,
    max_carry_ids: usize,
    max_worn_ids: usize,
) -> Result<Vec<MissingReqEntry>, SearchCursorError> {
    let mut accumulator =
        MissingReqAccumulator::new(max_requests, max_carry_ids, max_worn_ids, route_edges.len())?;
    for &packed_edge in route_edges {
        work_unit().await;
        let edge =
            diagnostic_edge(graph, packed_edge).ok_or(SearchCursorError::ResultNotRepresentable)?;
        accumulator.include_edge(edge, packed_edge, state).await?;
    }
    accumulator.finish(state, graph).await
}

async fn missing_item_reqs_compact_route(
    route: &CompactRoute,
    graph: &TransportGraph,
    state: &WorldState,
    max_requests: usize,
    max_carry_ids: usize,
    max_worn_ids: usize,
) -> Result<Vec<MissingReqEntry>, SearchCursorError> {
    let mut accumulator = MissingReqAccumulator::new(
        max_requests,
        max_carry_ids,
        max_worn_ids,
        route.edge_ids.len(),
    )?;
    let mut edge_position = route.edge_ids.len();
    for transition in (0..route.transition_count as usize).rev() {
        work_unit().await;
        let tag = route
            .tags
            .get(transition >> 2)
            .map(|byte| (byte >> ((transition & 3) * 2)) & 0x03)
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let (edge, packed_edge) = match tag {
            0 => continue,
            1..=3 => {
                edge_position = edge_position
                    .checked_sub(1)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                let packed_index = *route
                    .edge_ids
                    .get(edge_position)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                if tag == 3 {
                    continue;
                }
                let index = usize::try_from(packed_index)
                    .map_err(|_| SearchCursorError::ResultNotRepresentable)?;
                let edge = if tag == 1 {
                    graph.edges.get(index)
                } else {
                    graph.teleports.get(index)
                }
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
                let packed_edge = if tag == 1 {
                    packed_index
                } else {
                    packed_index | DIAGNOSTIC_TELEPORT_BIT
                };
                (edge, packed_edge)
            }
            _ => return Err(SearchCursorError::ResultNotRepresentable),
        };
        accumulator.include_edge(edge, packed_edge, state).await?;
    }
    if edge_position != 0 {
        return Err(SearchCursorError::ResultNotRepresentable);
    }
    accumulator.finish(state, graph).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "the missing-requirement route borrows the unchanged collision, graph and search inputs"
)]
async fn missing_route_for_relaxation(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    relax: Relax,
    progress: Option<&AtomicUsize>,
) -> Result<(Option<Vec<u32>>, usize), SearchCursorError> {
    let preferred_budget = FIRST_TARGET_BUDGET.min(budget);
    let search = first_cursor_search_with_output(
        collision,
        graph,
        from,
        targets,
        fallback,
        options,
        state,
        relax,
        avoid,
        preferred_budget,
        budget,
        progress,
        FirstRouteMaterialization::DiagnosticEdges,
    )
    .await?;
    let mut settled = search.settled;
    let (preferred, fallback_route) = (search.preferred, search.fallback);
    match preferred {
        Ok(route) => Ok((Some(route.into_diagnostic_edges()), settled)),
        Err(_) => match fallback_route {
            Some(FirstFallbackCandidate::Routed(route)) => {
                Ok((Some(route.into_diagnostic_edges()), settled))
            }
            Some(FirstFallbackCandidate::Undecided) => {
                let fallback_search = first_cursor_search_with_output(
                    collision,
                    graph,
                    from,
                    fallback,
                    &[],
                    options,
                    state,
                    relax,
                    avoid,
                    preferred_budget,
                    budget,
                    progress,
                    FirstRouteMaterialization::DiagnosticEdges,
                )
                .await?;
                settled = settled
                    .checked_add(fallback_search.settled)
                    .ok_or(SearchCursorError::ResultNotRepresentable)?;
                Ok((
                    fallback_search
                        .preferred
                        .ok()
                        .map(FirstRouteCandidate::into_diagnostic_edges),
                    settled,
                ))
            }
            Some(FirstFallbackCandidate::Failed(_)) | None => Ok((None, settled)),
        },
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the missing-requirement search borrows the unchanged collision, graph and search inputs"
)]
async fn missing_cursor_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    fallback: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    max_missing_requests: usize,
    max_missing_carry_ids: usize,
    max_missing_worn_ids: usize,
    progress: Option<&AtomicUsize>,
) -> Result<(Option<Vec<MissingReqEntry>>, usize), SearchCursorError> {
    let (route_edges, mut forward_settled) = missing_route_for_relaxation(
        collision,
        graph,
        from,
        targets,
        fallback,
        options,
        state,
        avoid,
        budget,
        Relax::CarryWorn,
        progress,
    )
    .await?;
    if let Some(route_edges) = route_edges {
        let missing = missing_item_reqs_edges(
            &route_edges,
            graph,
            state,
            max_missing_requests,
            max_missing_carry_ids,
            max_missing_worn_ids,
        )
        .await?;
        return Ok((Some(missing), forward_settled));
    }
    for edge in graph.edges.iter().chain(&graph.teleports) {
        work_unit().await;
        if state.quest_gates(edge) == Truth::Unknown {
            let (route_edges, settled) = missing_route_for_relaxation(
                collision,
                graph,
                from,
                targets,
                fallback,
                options,
                state,
                avoid,
                budget,
                Relax::CarryWornUnknownQuest,
                progress,
            )
            .await?;
            forward_settled = forward_settled
                .checked_add(settled)
                .ok_or(SearchCursorError::ResultNotRepresentable)?;
            let Some(route_edges) = route_edges else {
                return Ok((None, forward_settled));
            };
            let missing = missing_item_reqs_edges(
                &route_edges,
                graph,
                state,
                max_missing_requests,
                max_missing_carry_ids,
                max_missing_worn_ids,
            )
            .await?;
            return Ok(((!missing.is_empty()).then_some(missing), forward_settled));
        }
    }
    Ok((None, forward_settled))
}

async fn run_search_request(
    request: SearchRequest,
    lease: NavLease,
    state: Arc<WorldState>,
    reservation: SearchReservation,
    progress_counter: Option<Arc<AtomicUsize>>,
) -> Result<SearchCompletion, SearchCursorError> {
    let pack = lease.identity().pack();
    let collision = lease.collision_world();
    let graph = lease.graph();
    let progress = progress_counter.as_deref();
    let (mut outcome, forward_settled) = match request {
        SearchRequest::Single {
            from,
            to,
            options,
            avoid,
            max_settles,
        } => {
            let (route, settled) = single_cursor_search(
                collision,
                graph,
                from,
                to,
                options,
                &state,
                &avoid,
                max_settles,
                progress,
            )
            .await?;
            (SearchOutcome::Single { route }, settled)
        }
        SearchRequest::Many {
            from,
            targets,
            options,
            avoid,
            max_settles,
            reconstruct_best,
        } => {
            let (results, best, settled) = many_cursor_search(
                collision,
                graph,
                from,
                &targets,
                options,
                &state,
                &avoid,
                max_settles,
                reconstruct_best,
                progress,
            )
            .await?;
            (
                SearchOutcome::Many {
                    results: results.into_boxed_slice(),
                    best,
                },
                settled,
            )
        }
        SearchRequest::First {
            from,
            targets,
            fallback,
            options,
            avoid,
            max_settles,
        } => {
            let search = first_cursor_search(
                collision,
                graph,
                from,
                &targets,
                &fallback,
                options,
                &state,
                Relax::Strict,
                &avoid,
                FIRST_TARGET_BUDGET.min(max_settles),
                max_settles,
                progress,
            )
            .await?;
            let settled = search.settled();
            (SearchOutcome::First { search }, settled)
        }
        SearchRequest::MissingItemReqs {
            from,
            targets,
            fallback,
            options,
            avoid,
            max_settles,
        } => {
            let (missing, settled) = missing_cursor_search(
                collision,
                graph,
                from,
                &targets,
                &fallback,
                options,
                &state,
                &avoid,
                max_settles,
                reservation.max_missing_requests,
                reservation.max_missing_carry_ids,
                reservation.max_missing_worn_ids,
                progress,
            )
            .await?;
            (
                SearchOutcome::MissingItemReqs(
                    missing.map(|entries| MissingRequirements::new(entries, lease.clone())),
                ),
                settled,
            )
        }
        SearchRequest::MissingItemReqsForRoute { route } => {
            if route
                .lease
                .as_ref()
                .map(|route_lease| route_lease.identity().pack())
                != Some(pack)
            {
                return Err(SearchCursorError::PreflightMismatch);
            }
            let missing = missing_item_reqs_compact_route(
                &route,
                graph,
                &state,
                reservation.max_missing_requests,
                reservation.max_missing_carry_ids,
                reservation.max_missing_worn_ids,
            )
            .await?;
            (
                SearchOutcome::MissingItemReqsForRoute {
                    route,
                    missing: MissingRequirements::new(missing, lease.clone()),
                },
                0,
            )
        }
        SearchRequest::BlockingZones {
            diagnosis,
            from,
            targets,
            options,
            avoid,
            max_settles,
            zone_filter,
        } => {
            let (zones, settled) = blocking_cursor_search(
                collision,
                graph,
                from,
                &targets,
                diagnosis,
                options,
                &state,
                &avoid,
                zone_filter,
                max_settles,
                progress,
            )
            .await?;
            (SearchOutcome::BlockingZones(zones), settled)
        }
        SearchRequest::UnresolvedQuestGates {
            from,
            targets,
            options,
            avoid,
            max_settles,
        } => {
            let (gates, settled) = unresolved_quest_cursor_search(
                collision,
                graph,
                from,
                &targets,
                options,
                &state,
                &avoid,
                max_settles,
                reservation.max_quest_gates,
                progress,
            )
            .await?;
            (SearchOutcome::UnresolvedQuestGates(gates), settled)
        }
    };
    match &mut outcome {
        SearchOutcome::Single { route: Ok(route) } => route.retain_lease(lease.clone()),
        SearchOutcome::Many {
            best: Some((_, route)),
            ..
        } => route.retain_lease(lease.clone()),
        SearchOutcome::First { search } => {
            if let Ok(route) = &mut search.preferred {
                route.retain_lease(lease.clone());
            }
            if let Some(CompactFallbackRoute::Routed(route)) = &mut search.fallback {
                route.retain_lease(lease.clone());
            }
        }
        _ => {}
    }
    let forward_settled = progress_counter
        .as_deref()
        .map_or(forward_settled, |progress| {
            progress.load(AtomicOrdering::Relaxed)
        });
    Ok(SearchCompletion {
        outcome,
        state,
        forward_settled,
        pack,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cooperative kernel borrows the unchanged quest search inputs"
)]
async fn unresolved_quest_cursor_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    options: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: usize,
    max_quest_gates: usize,
    progress: Option<&AtomicUsize>,
) -> Result<(Result<Option<Box<[QuestGate]>>, QuestFamilyMismatch>, usize), SearchCursorError> {
    if let (Some(evidence), Some(pack)) = (&state.quest_evidence, graph.quest_family) {
        if *evidence.family() != pack {
            return Ok((
                Err(QuestFamilyMismatch {
                    pack: Some(pack),
                    expected: *evidence.family(),
                }),
                0,
            ));
        }
    }
    let mut forward_settled = 0usize;
    let mut selected = None;
    for relax in [Relax::UnknownQuest, Relax::CarryWornUnknownQuest] {
        let search = first_cursor_search(
            collision,
            graph,
            from,
            targets,
            &[],
            options,
            state,
            relax,
            avoid,
            FIRST_TARGET_BUDGET.min(budget),
            budget,
            progress,
        )
        .await?;
        forward_settled = forward_settled
            .checked_add(search.settled())
            .ok_or(SearchCursorError::ResultNotRepresentable)?;
        let (preferred, _) = search.into_routes();
        if let Ok(route) = preferred {
            selected = Some(route);
            break;
        }
    }
    let Some(route) = selected else {
        return Ok((Ok(None), forward_settled));
    };
    let mut unresolved = Vec::new();
    unresolved
        .try_reserve_exact(max_quest_gates)
        .map_err(|_| SearchCursorError::Allocation)?;
    for leg in route.cursor(graph) {
        work_unit().await;
        let CompactRouteLeg::Transport(step) = leg else {
            continue;
        };
        let Some(edge) = step.source() else {
            continue;
        };
        let Some(gates) = &edge.quest_gates else {
            continue;
        };
        let evidence = match &state.quest_evidence {
            Some(evidence) if evidence.family() != gates.family() => {
                return Ok((
                    Err(QuestFamilyMismatch {
                        pack: Some(*gates.family()),
                        expected: *evidence.family(),
                    }),
                    forward_settled,
                ));
            }
            evidence => evidence.as_ref(),
        };
        for gate in gates.gates() {
            work_unit().await;
            let truth = evidence.map_or(Truth::Unknown, |evidence| evidence.test(gate));
            if truth != Truth::Unknown {
                continue;
            }
            let mut duplicate = false;
            for previous in &unresolved {
                work_unit().await;
                if previous == gate {
                    duplicate = true;
                    break;
                }
            }
            if !duplicate {
                if unresolved.len() == max_quest_gates {
                    return Err(SearchCursorError::ResultNotRepresentable);
                }
                unresolved.push(gate.clone());
            }
        }
    }
    Ok((
        Ok((!unresolved.is_empty()).then(|| unresolved.into_boxed_slice())),
        forward_settled,
    ))
}

/// Diagnose a strict search that could not route through a quest-stage
/// gate: search again crossing every gate the state's evidence leaves
/// `Unknown` (a `False` gate still closes, every other requirement stays
/// strict), and name those gates on the cheapest such route. When that
/// search fails too, a crossing may need an `Unknown` gate *and* a
/// carry/wear fact the state lacks: the search runs once more also ignoring
/// `item_req`/`worn_req`, and names only the undecided gates on that route
/// ([`find_missing_item_reqs`] names its items), so neither diagnosis hides
/// behind the other's gate. `Ok(Some)` is what the caller's action owner
/// must resolve — read the named journals, refresh its provider, search
/// again; the gates come back as values, never as a diagnostic string.
/// `Ok(None)`: no evidence can open a route (no relaxed search routes, or
/// its route needs no undecided gate). `Err`: the state's evidence was
/// prepared from another quest family than the pack's gates, so no evidence
/// it supplies can decide them; the caller refuses rather than re-reading
/// journals. A search that routes never crosses an `Unknown` gate. Runs only
/// after a failure: the strict searches never collect anything.
pub fn find_unresolved_quest_gates(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> Result<Option<Arc<[QuestGate]>>, QuestFamilyMismatch> {
    if let (Some(evidence), Some(pack)) = (&state.quest_evidence, graph.quest_family) {
        if *evidence.family() != pack {
            return Err(QuestFamilyMismatch {
                pack: Some(pack),
                expected: *evidence.family(),
            });
        }
    }
    let search = |relax| {
        first_search(
            collision,
            graph,
            from,
            targets,
            &[],
            opts,
            state,
            relax,
            avoid,
            NODE_BUDGET,
            NODE_BUDGET,
        )
        .into_route()
        .ok()
    };
    let Some(route) = search(Relax::UnknownQuest).or_else(|| search(Relax::CarryWornUnknownQuest))
    else {
        return Ok(None);
    };
    let mut unresolved: Vec<QuestGate> = Vec::new();
    for leg in &route.legs {
        let Leg::Transport { edge } = leg else {
            continue;
        };
        let Some(gates) = &edge.quest_gates else {
            continue;
        };
        let evidence = match &state.quest_evidence {
            Some(evidence) if evidence.family() != gates.family() => {
                return Err(QuestFamilyMismatch {
                    pack: Some(*gates.family()),
                    expected: *evidence.family(),
                });
            }
            evidence => evidence.as_ref(),
        };
        for gate in gates.gates() {
            let truth = evidence.map_or(Truth::Unknown, |evidence| evidence.test(gate));
            if truth == Truth::Unknown && !unresolved.contains(gate) {
                unresolved.push(gate.clone());
            }
        }
    }
    Ok((!unresolved.is_empty()).then(|| unresolved.into()))
}

/// [`find`] with an explicit per-search cost model (the run-vs-walk rate).
pub fn find_with_model(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        model,
        NODE_BUDGET,
        false,
        false,
        &WorldState::empty(),
        None,
        &[],
        ZoneExempt::NONE,
    )
}

/// [`find`] with the any-tile teleport layer unioned in: every edge in
/// `graph.teleports` is usable from **any** node at cost `edge.ticks`, so
/// a teleport can take the route across a wall, a level boundary, or half
/// the map. Requirements stay on the edges and are gated by `state` like
/// every other transport edge. Default [`find`]/[`find_with_model`] never
/// see teleports. Wilderness stays refused (`allow_wilderness` off).
pub fn find_allow_teleports(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with(
        collision,
        graph,
        from,
        to,
        FindOptions {
            allow_teleports: true,
            allow_wilderness: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        state,
    )
}

/// [`find_allow_teleports`] with an explicit per-search cost model.
pub fn find_allow_teleports_with_model(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        model,
        NODE_BUDGET,
        true,
        false,
        state,
        None,
        &[],
        ZoneExempt::NONE,
    )
}

/// [`find`] with an explicit cost model and node-expansion budget (the
/// search gives up with [`RouteError::BudgetExhausted`] once `budget`
/// tiles are settled). Teleports, wilderness, and the gating state stay
/// the [`find`] defaults (empty). Test-only: the budget knob has no
/// production caller yet.
#[cfg(test)]
fn find_bounded(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
) -> Result<Route, RouteError> {
    find_bounded_impl(
        collision,
        graph,
        from,
        to,
        model,
        budget,
        false,
        false,
        &WorldState::empty(),
        None,
        &[],
        ZoneExempt::NONE,
    )
}

/// True when `tile` sits in any avoidance rect.
fn tile_in_any_avoid(tile: WorldTile, avoid: &[AvoidRect]) -> bool {
    avoid.iter().any(|r| r.contains(tile))
}

enum FirstTargets<'a> {
    Inline(&'a [WorldTile]),
    Indexed(HashSet<WorldTile>),
}

impl<'a> FirstTargets<'a> {
    fn new(targets: &'a [WorldTile]) -> Self {
        if targets.len() <= 8 {
            Self::Inline(targets)
        } else {
            Self::Indexed(targets.iter().copied().collect())
        }
    }
    async fn new_cooperative(targets: &'a [WorldTile]) -> Result<Self, SearchCursorError> {
        if targets.len() <= 8 {
            return Ok(Self::Inline(targets));
        }
        let mut indexed = HashSet::new();
        indexed
            .try_reserve(targets.len())
            .map_err(|_| SearchCursorError::Allocation)?;
        for &target in targets {
            work_unit().await;
            indexed.insert(target);
        }
        Ok(Self::Indexed(indexed))
    }
    fn len(&self) -> usize {
        match self {
            Self::Inline(targets) => targets.len(),
            Self::Indexed(targets) => targets.len(),
        }
    }

    fn contains(&self, tile: &WorldTile) -> bool {
        match self {
            Self::Inline(targets) => targets.contains(tile),
            Self::Indexed(targets) => targets.contains(tile),
        }
    }
}

enum Goals<'a> {
    Single {
        to: WorldTile,
        cost: Option<TargetCost>,
    },
    /// A preferred and a fallback goal set, each with its own budget and
    /// backward proof ([`FirstGoals`]); boxed, as its two proofs' inline
    /// state dwarfs the other shapes.
    First(Box<FirstGoals<'a>>),
    Many {
        unique: &'a HashMap<WorldTile, usize>,
        costs: &'a mut [Result<TargetCost, TargetError>],
        remaining: usize,
    },
    /// Exhausts the source-reachable graph while recording zone-blocked
    /// transitions; used only for refusal attribution.
    Frontier {
        targets: FirstTargets<'a>,
        blockers: &'a mut HashSet<u16>,
    },
}

/// What one settled node decided for the search.
enum Settle {
    Continue,
    /// Stop with the node counted: it decided the goals.
    Stop,
    /// Stop without counting the node: no goal set may spend it.
    Spent,
}

impl Goals<'_> {
    fn target_count(&self) -> Option<usize> {
        match self {
            Self::Single { .. } => Some(1),
            Self::First(first) => first
                .preferred
                .targets
                .len()
                .checked_add(first.fallback.targets.len()),
            Self::Many { unique, .. } => Some(unique.len()),
            Self::Frontier { targets, .. } => Some(targets.len()),
        }
    }
    fn is_target(&self, tile: WorldTile) -> bool {
        match self {
            Self::Single { to, .. } => *to == tile,
            Self::First(first) => {
                first.preferred.targets.contains(&tile) || first.fallback.targets.contains(&tile)
            }
            Self::Many { unique, .. } => unique.contains_key(&tile),
            Self::Frontier { targets, .. } => targets.contains(&tile),
        }
    }
    fn is_frontier(&self) -> bool {
        matches!(self, Self::Frontier { .. })
    }

    async fn record_blockers(&mut self, blockers: impl Iterator<Item = u16>) {
        if let Self::Frontier {
            blockers: recorded, ..
        } = self
        {
            for blocker in blockers {
                work_unit().await;
                recorded.insert(blocker);
            }
        }
    }

    // These shapes cannot yield. Keep the proof-backed First future out of
    // their per-node path: its inline state would otherwise be copied twice
    // even when the selected shape never polls it.
    fn settle_non_first(&mut self, tile: WorldTile, cost: TargetCost) -> Settle {
        let done = match self {
            Goals::Single { to, cost: found } => {
                if *to == tile {
                    *found = Some(cost);
                    true
                } else {
                    false
                }
            }
            Goals::First(_) => unreachable!("first goals settle with their cooperative proof"),
            Goals::Many {
                unique,
                costs,
                remaining,
            } => {
                if let Some(&index) = unique.get(&tile) {
                    if costs[index] == Err(TargetError::NotSettled) {
                        costs[index] = Ok(cost);
                        *remaining -= 1;
                    }
                }
                *remaining == 0
            }
            Goals::Frontier { .. } => return Settle::Continue,
        };
        if done {
            Settle::Stop
        } else {
            Settle::Continue
        }
    }

    /// Everything reachable has settled, `settled` nodes in all.
    async fn exhausted(&mut self, settled: usize) -> Result<(), SearchCursorError> {
        if let Goals::First(first) = self {
            first.exhausted(settled).await?;
        }
        Ok(())
    }

    /// The backward proofs' final report, dropping their scratch.
    fn finish_proofs(&mut self) -> ReverseReport {
        match self {
            Goals::First(first) => first.finish_proofs(),
            Goals::Single { .. } | Goals::Many { .. } | Goals::Frontier { .. } => {
                ReverseReport::default()
            }
        }
    }
}

/// One goal set of a first-goal search: its first goal to settle, the
/// settles it may spend, and its backward proof.
struct GoalSet<'a> {
    targets: FirstTargets<'a>,
    seeds: &'a [WorldTile],
    found: Option<(WorldTile, TargetCost)>,
    ordered: bool,
    found_rank: Option<usize>,
    /// The unproven budget until the proof shows a goal reachable, then the
    /// reachable budget.
    budget: usize,
    proof: ReverseRun<'a>,
    /// Why no goal of the set can settle within its budget.
    closed: Option<RouteError>,
}

impl<'a> GoalSet<'a> {
    /// An empty set is closed from the start.
    async fn new(
        targets: &'a [WorldTile],
        budget: usize,
        ordered: bool,
    ) -> Result<Self, SearchCursorError> {
        Ok(Self {
            targets: FirstTargets::new_cooperative(targets).await?,
            seeds: targets,
            found: None,
            ordered,
            found_rank: None,
            budget,
            proof: ReverseRun::default(),
            closed: targets.is_empty().then_some(RouteError::NoPath),
        })
    }

    fn live(&self) -> bool {
        self.found.is_none() && self.closed.is_none()
    }

    fn close(&mut self, error: RouteError) {
        self.closed = Some(error);
        self.proof.finish();
    }

    /// Record the first settled goal for the ordinary first-goal policy.
    fn accept(&mut self, tile: WorldTile, cost: TargetCost) -> bool {
        if !self.live() || !self.targets.contains(&tile) {
            return false;
        }
        self.found = Some((tile, cost));
        self.proof.finish();
        true
    }

    /// Resolve input rank only when an ordered search settles a goal.
    async fn accept_ordered(&mut self, tile: WorldTile, cost: TargetCost) -> bool {
        let mut rank = 0;
        for &target in self.seeds {
            work_unit().await;
            if target == tile {
                break;
            }
            rank += 1;
        }
        if let Some((_, old)) = self.found {
            if old.ticks < cost.ticks
                || (old.ticks == cost.ticks && self.found_rank.is_some_and(|old| old <= rank))
            {
                return false;
            }
        }
        self.found_rank = Some(rank);
        self.found = Some((tile, cost));
        self.proof.finish();
        true
    }

    /// Runs the proof to one backward step per settle so far: reachability
    /// lifts the budget, unreachability closes the set.
    async fn prove_to(
        &mut self,
        gates: ProofGates<'a>,
        steps: usize,
        reachable_budget: usize,
    ) -> Result<(), SearchCursorError> {
        match self.proof.run_to(gates, self.seeds, steps).await? {
            Some(ReverseProof::Reachable) => self.budget = self.budget.max(reachable_budget),
            Some(ReverseProof::Unreachable) => self.close(RouteError::NoPath),
            _ => {}
        }
        Ok(())
    }
}

/// A first-goal search's preferred and fallback goal sets. The preferred
/// set runs the search: its proof steps once per settle and lifts or ends
/// it, and its first goal to settle ends it. The fallback set records its
/// first goal on the way, decided at every point exactly as a search over
/// the fallback alone would decide it (Dijkstra's settle order does not
/// depend on the goals). Within its unproven budget that needs no proof;
/// when one of its goals settles past that budget, or the preferred set
/// stops without a goal, its proof starts, caught up to one step per settle
/// spent. The preferred proof has decided by then, so only one proof's
/// scratch is live at a time.
struct FirstGoals<'a> {
    gates: ProofGates<'a>,
    reachable_budget: usize,
    preferred: GoalSet<'a>,
    fallback: GoalSet<'a>,
}

impl<'a> FirstGoals<'a> {
    async fn new_cooperative(
        gates: ProofGates<'a>,
        targets: &'a [WorldTile],
        fallback: &'a [WorldTile],
        budget: usize,
        reachable_budget: usize,
        ordered: bool,
    ) -> Result<Self, SearchCursorError> {
        let budget = if ordered { reachable_budget } else { budget };
        let mut preferred = GoalSet::new(targets, budget, ordered).await?;
        preferred.prove_to(gates, 0, reachable_budget).await?;
        Ok(Self {
            gates,
            reachable_budget,
            preferred,
            fallback: GoalSet::new(fallback, budget, false).await?,
        })
    }

    async fn settle(
        &mut self,
        tile: WorldTile,
        cost: TargetCost,
    ) -> Result<Settle, SearchCursorError> {
        let settled = cost.settled_at;
        if self.preferred.ordered {
            if let Some((_, found)) = self.preferred.found {
                if cost.ticks > found.ticks || settled > self.preferred.budget {
                    return Ok(Settle::Stop);
                }
                if self.preferred.targets.contains(&tile) {
                    self.preferred.accept_ordered(tile, cost).await;
                }
                return Ok(if self.preferred.found_rank == Some(0) {
                    Settle::Stop
                } else {
                    Settle::Continue
                });
            }
        }
        if settled > self.preferred.budget {
            self.preferred.close(RouteError::BudgetExhausted);
            self.decide_fallback(settled - 1, settled).await?;
            return Ok(Settle::Spent);
        }
        let accepted = if self.preferred.ordered {
            self.preferred.live()
                && self.preferred.targets.contains(&tile)
                && self.preferred.accept_ordered(tile, cost).await
        } else {
            self.preferred.accept(tile, cost)
        };
        if accepted {
            return Ok(
                if self.preferred.ordered && self.preferred.found_rank != Some(0) {
                    Settle::Continue
                } else {
                    Settle::Stop
                },
            );
        }
        // A late fallback goal counts only if its own proof lifts the budget.
        // Other nodes need no fallback future.
        if self.fallback.live() && self.fallback.targets.contains(&tile) {
            if settled > self.fallback.budget {
                self.decide_fallback(settled - 1, settled).await?;
            }
            self.fallback.accept(tile, cost);
        }
        self.preferred
            .prove_to(self.gates, settled, self.reachable_budget)
            .await?;
        if self.preferred.live() {
            return Ok(Settle::Continue);
        }
        self.decide_fallback(settled, settled).await?;
        Ok(Settle::Stop)
    }

    /// Decides the fallback set, where that needs no further settle, after
    /// `spent` settles with settle ordinal `known` known to exist. Its proof
    /// is caught up to one step per settle spent, up to its budget, as a
    /// search over it alone would have stepped it: shown unreachable, the
    /// set has failed; not shown reachable while a settle past its budget
    /// exists, that search stopped at the budget. Otherwise it stays
    /// undecided.
    async fn decide_fallback(
        &mut self,
        spent: usize,
        known: usize,
    ) -> Result<(), SearchCursorError> {
        if !self.fallback.live() {
            return Ok(());
        }
        let steps = spent.min(self.fallback.budget);
        self.fallback
            .prove_to(self.gates, steps, self.reachable_budget)
            .await?;
        if self.fallback.live() && known > self.fallback.budget {
            self.fallback.close(RouteError::BudgetExhausted);
        }
        Ok(())
    }

    /// Everything reachable settled, `settled` nodes, without a preferred
    /// goal; a fallback set not stopped earlier has no goal to settle.
    async fn exhausted(&mut self, settled: usize) -> Result<(), SearchCursorError> {
        self.preferred.close(RouteError::NoPath);
        self.decide_fallback(settled, settled).await?;
        if self.fallback.live() {
            self.fallback.close(RouteError::NoPath);
        }
        Ok(())
    }

    /// The preferred proof's outcome, and the larger scratch of the two
    /// proofs (the fallback's starts only after the preferred one's is
    /// dropped).
    fn finish_proofs(&mut self) -> ReverseReport {
        let preferred = self.preferred.proof.finish();
        let fallback = self.fallback.proof.finish();
        ReverseReport {
            proof: preferred.proof,
            seen: preferred.seen.max(fallback.seen),
            queue: preferred.queue.max(fallback.queue),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchStop {
    Completed,
    Exhausted,
    Budget,
    Allocation,
    Deadline,
}

struct KernelOutcome {
    came_from: Predecessors,
    settled: usize,
    stop: SearchStop,
    capacities: SearchCapacities,
    proof: ReverseProof,
}

impl KernelOutcome {
    fn empty() -> Self {
        Self {
            came_from: Predecessors::empty(),
            settled: 0,
            stop: SearchStop::Completed,
            capacities: SearchCapacities::default(),
            proof: ReverseProof::NotRun,
        }
    }

    #[allow(clippy::too_many_arguments)] // the kernel's scratch tables plus its stop facts
    fn finish<B: Budget>(
        came_from: PackedSearchTable,
        heap: &PackedMinMaxHeap,
        settled: usize,
        stop: SearchStop,
        record_capacities: bool,
        reverse: ReverseReport,
        zone_mask_words: usize,
        resources: B,
        reached: HashMap<WorldTile, u32>,
    ) -> Self {
        let table_capacity = came_from.capacity();
        let capacities = if record_capacities {
            SearchCapacities {
                distances: table_capacity,
                predecessors: table_capacity,
                settled: table_capacity,
                heap: heap.capacity(),
                reverse: reverse.seen,
                reverse_queue: reverse.queue,
                zone_mask_words,
            }
        } else {
            SearchCapacities::default()
        };
        Self {
            came_from: resources.finish(came_from, reached),
            settled,
            stop,
            capacities,
            proof: reverse.proof,
        }
    }
}

/// The gates a diagnosis search may cross; every routing search is strict.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Relax {
    /// Every requirement holds, quest-stage gates only when `True`.
    Strict,
    /// The BankBudget diagnosis ([`find_missing_item_reqs`]): only the
    /// `item_req`/`worn_req` gates are ignored; `worn_all_req` stays strict.
    CarryWorn,
    /// The quest-evidence diagnosis ([`find_unresolved_quest_gates`]): a
    /// stage gate the evidence leaves `Unknown` is crossed; `False` closes.
    UnknownQuest,
    /// Both diagnosis arms' fallback: `item_req`/`worn_req` gates are ignored
    /// and `Unknown` stage gates crossed, so a crossing that needs a fetched
    /// item and a journal read together is visible to each arm. Every other
    /// requirement, including `worn_all_req`, stays strict.
    CarryWornUnknownQuest,
}

/// Whether `state` proves `edge`'s requirements under `relax`.
fn edge_allowed(state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool {
    match relax {
        Relax::Strict => state.allows(edge),
        Relax::CarryWorn => state.allows_without_carry_worn(edge),
        Relax::UnknownQuest => {
            state.snapshot_allows(edge) && state.quest_gates(edge) != Truth::False
        }
        Relax::CarryWornUnknownQuest => {
            state.fixed_reqs_allow(edge)
                && state.worn_all_req_allows(edge)
                && state.quest_gates(edge) != Truth::False
        }
    }
}

// Backward reachability is an over-approximation. A charge downgrade can
// supply a variant absent at departure; only forward resource labels can
// decide its carried gate. Worn, fixed and quest gates remain exact.
fn proof_edge_allowed(
    state: &WorldState,
    edge: &TransportEdge,
    relax: Relax,
    inventory_can_grow: bool,
) -> bool {
    if inventory_can_grow && matches!(relax, Relax::Strict | Relax::UnknownQuest) {
        resources::fixed_allowed(state, edge, relax)
    } else {
        edge_allowed(state, edge, relax)
    }
}

/// Slashable webs declare both knife `oplocu` and worn-blade `oploc1` edges.
/// When both pass the live state, mirror `slash_checker` by suppressing the
/// knife choice in routing, independent of pack edge order.
fn prefer_worn_slash_for_web(
    graph: &TransportGraph,
    edge: &TransportEdge,
    state: &WorldState,
    relax: Relax,
) -> bool {
    if relax != Relax::Strict
        || !edge.is_slashable_web()
        || edge.option != 0
        || edge.item_req.is_empty()
    {
        return false;
    }
    graph.at.get(&edge.at).is_some_and(|indices| {
        select_web_action(
            indices.iter().map(|&index| &graph.edges[index]),
            edge,
            |candidate| edge_allowed(state, candidate, relax),
        )
        .is_some_and(|selected| selected.option == 1)
    })
}

/// The outcome and peak scratch of one search's backward proof.
#[derive(Clone, Copy, Default)]
struct ReverseReport {
    proof: ReverseProof,
    seen: usize,
    queue: usize,
}

/// The forward search's gates, as its backward proofs borrow them.
#[derive(Clone, Copy)]
struct ProofGates<'a> {
    collision: &'a WorldCollision,
    graph: &'a TransportGraph,
    state: &'a WorldState,
    essence: Option<&'a EssenceSession>,
    from: WorldTile,
    use_teleports: bool,
    allow_wilderness: bool,
    relax: Relax,
    inventory_can_grow: bool,
    cooperative_storage: bool,
}

/// One goal set's backward proof: not yet started, then live closure
/// scratch until the proof decides, then only its report.
#[derive(Default)]
struct ReverseRun<'a> {
    started: bool,
    /// Backward steps taken since the proof started.
    steps: usize,
    closure: Option<ReverseClosure<'a>>,
    report: ReverseReport,
}

impl<'a> ReverseRun<'a> {
    /// Starts the proof from `seeds` if it has not started, then steps it
    /// until it has taken `steps` backward steps. A decision drops the
    /// closure's scratch and is returned once.
    async fn run_to(
        &mut self,
        gates: ProofGates<'a>,
        seeds: &[WorldTile],
        steps: usize,
    ) -> Result<Option<ReverseProof>, SearchCursorError> {
        if !self.started {
            self.started = true;
            let mut closure = ReverseClosure::new(gates).await?;
            for &seed in seeds {
                work_unit().await;
                let proof = {
                    let admission = std::pin::pin!(closure.admit(seed));
                    admission.await?
                };
                if let Some(proof) = proof {
                    self.report = closure.report(proof);
                    return Ok(Some(proof));
                }
            }
            self.closure = Some(closure);
        }
        while self.steps < steps {
            let Some(closure) = self.closure.as_mut() else {
                return Ok((self.report.proof != ReverseProof::Open).then_some(self.report.proof));
            };
            self.steps += 1;
            if let Some(proof) = closure.step().await? {
                self.report = closure.report(proof);
                self.closure = None;
                return Ok(Some(proof));
            }
        }
        Ok(None)
    }
    /// The final report, dropping live scratch: a closure still running
    /// when its set or the search stops is [`ReverseProof::Open`].
    fn finish(&mut self) -> ReverseReport {
        if let Some(closure) = self.closure.take() {
            self.report = closure.report(ReverseProof::Open);
        }
        self.report
    }
}

/// Backward closure of a goal set over a superset of the forward search's
/// moves. Walk predecessors are exact: `p` precedes `q` when the forward
/// [`step_ok`] and wilderness entry gate admit the step `p -> q`. Transport
/// and essence-return predecessors (the standable take-offs within
/// [`INTERACT_RADIUS`] of each usable edge landing in the closure) are added
/// whenever the walk frontier empties, by one scan of the packed edge list (a
/// few thousand edges) rather than a per-search index. Avoidance rectangles
/// only remove forward moves and are ignored.
///
/// Admitting the origin, or the landing of a teleport the forward search can
/// take from the origin, proves a goal reachable. Any other usable teleport
/// landing in the closure makes every tile a potential predecessor, so a
/// closure holding one cannot prove anything unreachable. Otherwise a closure
/// that empties without meeting the origin is closed under every forward
/// predecessor, so no forward path reaches the goal set.
struct ReverseClosure<'a> {
    collision: &'a WorldCollision,
    graph: &'a TransportGraph,
    state: &'a WorldState,
    essence: Option<&'a EssenceSession>,
    from: WorldTile,
    use_teleports: bool,
    allow_wilderness: bool,
    relax: Relax,
    inventory_can_grow: bool,
    zones: Option<&'a ZoneFilter<'a>>,
    targets: &'a [WorldTile],
    /// Landings of the gated teleports usable from `from` itself.
    landings: HashSet<WorldTile>,
    seen: HashSet<WorldTile>,
    queue: VecDeque<WorldTile>,
}

impl<'a> ReverseClosure<'a> {
    async fn new(gates: ProofGates<'a>) -> Result<Self, SearchCursorError> {
        let ProofGates {
            collision,
            graph,
            state,
            essence,
            from,
            use_teleports,
            allow_wilderness,
            relax,
            inventory_can_grow,
            cooperative_storage,
        } = gates;
        let mut landings = HashSet::new();
        if use_teleports {
            landings
                .try_reserve(graph.teleports.len())
                .map_err(|_| SearchCursorError::Allocation)?;
            let level = graph.wilderness.level(from);
            for edge in &graph.teleports {
                work_unit().await;
                if proof_edge_allowed(state, edge, relax, inventory_can_grow)
                    && TransportGraph::teleport_legal_at_level(level, edge)
                    && wildy_step_ok(graph, from, edge.to, allow_wilderness)
                {
                    landings.insert(edge.to);
                }
            }
        }
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        // Cooperative proofs reserve once so a slice never grows or rehashes
        // the full proof. Legacy calls retain the base's demand-driven storage.
        if cooperative_storage {
            seen.try_reserve(REVERSE_PROOF_BUDGET + 1)
                .map_err(|_| SearchCursorError::Allocation)?;
            queue
                .try_reserve(REVERSE_PROOF_BUDGET + 1)
                .map_err(|_| SearchCursorError::Allocation)?;
        }
        Ok(Self {
            collision,
            graph,
            state,
            essence,
            from,
            use_teleports,
            allow_wilderness,
            relax,
            inventory_can_grow,
            zones: None,
            targets: &[],
            landings,
            seen,
            queue,
        })
    }

    fn report(&self, proof: ReverseProof) -> ReverseReport {
        ReverseReport {
            proof,
            seen: self.seen.capacity(),
            queue: self.queue.capacity(),
        }
    }

    async fn zone_step_ok(&self, from: WorldTile, to: WorldTile) -> bool {
        let Some(zones) = self.zones else {
            return true;
        };
        let mut blocked = zones.blocking_transition_at(&self.graph.wilderness, from, to, &|| {
            self.targets.contains(&to)
        });
        let mut is_blocked = false;
        while blocked.next().is_some() {
            work_unit().await;
            is_blocked = true;
        }
        !is_blocked
    }

    /// Admit a predecessor. `Some` ends the proof: the origin (or a teleport
    /// landing it can reach directly) precedes the goals, or the closure
    /// outgrew [`REVERSE_PROOF_BUDGET`].
    async fn admit(&mut self, tile: WorldTile) -> Result<Option<ReverseProof>, SearchCursorError> {
        if !self.seen.insert(tile) {
            return Ok(None);
        }
        if tile == self.from
            || (!self.landings.is_empty()
                && self.landings.contains(&tile)
                && self.zone_step_ok(self.from, tile).await)
        {
            return Ok(Some(ReverseProof::Reachable));
        }
        if self.seen.len() > REVERSE_PROOF_BUDGET {
            return Ok(Some(ReverseProof::Abandoned));
        }
        self.queue.push_back(tile);
        Ok(None)
    }

    /// Expand the oldest frontier tile's walk predecessors, or cross
    /// transports once the walk frontier is empty.
    async fn step(&mut self) -> Result<Option<ReverseProof>, SearchCursorError> {
        let Some(tile) = self.queue.pop_front() else {
            return self.cross_transports().await;
        };
        for d in STEPS {
            work_unit().await;
            let before = WorldTile {
                x: tile.x - d.0,
                z: tile.z - d.1,
                level: tile.level,
            };
            if step_ok(self.collision, before, d)
                && wildy_step_ok(self.graph, before, tile, self.allow_wilderness)
                && self.zone_step_ok(before, tile).await
            {
                if let Some(proof) = std::pin::pin!(self.admit(before)).await? {
                    return Ok(Some(proof));
                }
            }
        }
        Ok(None)
    }

    /// Admit every transport and essence-return take-off whose landing is in
    /// the closure. Nothing new means the closure is complete.
    async fn cross_transports(&mut self) -> Result<Option<ReverseProof>, SearchCursorError> {
        let graph = self.graph;
        let state = self.state;
        if self.use_teleports {
            for edge in &graph.teleports {
                work_unit().await;
                if self.seen.contains(&edge.to)
                    && edge
                        .takeoff
                        .is_none_or(|takeoff| self.seen.contains(&takeoff))
                    && proof_edge_allowed(state, edge, self.relax, self.inventory_can_grow)
                {
                    return Ok(Some(ReverseProof::Abandoned));
                }
            }
        }
        let admitted = self.seen.len();
        for index in 0..graph.edges.len() {
            work_unit().await;
            let edge = &graph.edges[index];
            if (edge.player_delta.is_some() || self.seen.contains(&edge.to))
                && proof_edge_allowed(state, edge, self.relax, self.inventory_can_grow)
            {
                if let Some(proof) = self.admit_edge_takeoffs(index).await? {
                    return Ok(Some(proof));
                }
            }
        }
        if let Some(session) = self.essence {
            if self.seen.contains(&session.return_tile) {
                for &portal in ESSENCE_MINE_PORTALS {
                    work_unit().await;
                    if let Some(proof) = self.admit_takeoffs(portal, session.return_tile).await? {
                        return Ok(Some(proof));
                    }
                }
            }
        }
        Ok((self.seen.len() == admitted).then_some(ReverseProof::Unreachable))
    }

    /// Mirror forward footprint admission, including rotated force-approach
    /// sides and source wall faces. Radius-only predecessors are not a proof.
    async fn admit_edge_takeoffs(
        &mut self,
        index: usize,
    ) -> Result<Option<ReverseProof>, SearchCursorError> {
        let (min, max) = self.graph.takeoff_bounds(index);
        for x in min.x..=max.x {
            for z in min.z..=max.z {
                work_unit().await;
                let takeoff = WorldTile {
                    x,
                    z,
                    level: min.level,
                };
                if !self.graph.admissible_from(self.collision, index, takeoff) {
                    continue;
                }
                let edge = &self.graph.edges[index];
                let Some(to) = edge.landing_from(takeoff) else {
                    continue;
                };
                if (edge.player_delta.is_some() && !self.seen.contains(&to))
                    || !wildy_step_ok(self.graph, takeoff, to, self.allow_wilderness)
                    || !self.zone_step_ok(takeoff, to).await
                {
                    continue;
                }
                if let Some(proof) = std::pin::pin!(self.admit(takeoff)).await? {
                    return Ok(Some(proof));
                }
            }
        }
        Ok(None)
    }

    /// The forward search takes an edge at `at` from any standable tile
    /// within [`INTERACT_RADIUS`] on `at`'s level.
    async fn admit_takeoffs(
        &mut self,
        at: WorldTile,
        to: WorldTile,
    ) -> Result<Option<ReverseProof>, SearchCursorError> {
        for dx in -INTERACT_RADIUS..=INTERACT_RADIUS {
            for dz in -INTERACT_RADIUS..=INTERACT_RADIUS {
                work_unit().await;
                let takeoff = WorldTile {
                    x: at.x + dx,
                    z: at.z + dz,
                    level: at.level,
                };
                if self.collision.standable(takeoff)
                    && wildy_step_ok(self.graph, takeoff, to, self.allow_wilderness)
                    && self.zone_step_ok(takeoff, to).await
                {
                    if let Some(proof) = std::pin::pin!(self.admit(takeoff)).await? {
                        return Ok(Some(proof));
                    }
                }
            }
        }
        Ok(None)
    }
}

/// Run one single-target kernel with the supplied zone filter and rebuild
/// its route when the selected goal settles.
#[allow(clippy::too_many_arguments)]
fn run_single_search(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
    state: &WorldState,
    essence: Option<&EssenceSession>,
    avoid: &[AvoidRect],
    zones: Option<&ZoneFilter<'_>>,
) -> Result<Route, RouteError> {
    let mut goals = Goals::Single { to, cost: None };
    let search = drive_to_completion(search_kernel(
        collision,
        graph,
        from,
        model,
        budget,
        use_teleports,
        allow_wilderness,
        state,
        essence,
        Relax::Strict,
        avoid,
        zones,
        &mut goals,
        None,
        false,
        None,
    ));
    match goals {
        Goals::Single {
            cost: Some(cost), ..
        } => {
            let (legs, ticks) = reconstruct(to, &search.came_from, graph, model, essence);
            debug_assert_eq!(ticks, cost.ticks);
            Ok(Route {
                legs,
                dest: to,
                ticks,
            })
        }
        _ if matches!(search.stop, SearchStop::Budget | SearchStop::Allocation) => {
            Err(RouteError::BudgetExhausted)
        }
        _ => Err(RouteError::NoPath),
    }
}

/// The shared single-target Dijkstra behind [`find`]/[`find_with`]:
/// `use_teleports` unions the any-tile teleport layer into the relaxation
/// from every settled node. Transport edges are relaxed from any standable
/// tile within [`INTERACT_RADIUS`] of their `at` (never from `at` itself
/// when it is blocked); walk steps are the strict directional [`step_ok`]
/// test throughout. `allow_wilderness` gates stepping into (or landing in)
/// the wilderness zone; `state` gates every transport edge (walked or
/// teleported) on its requirements — an edge the state cannot prove is not
/// relaxed.
#[allow(clippy::too_many_arguments)]
fn find_bounded_impl(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
    state: &WorldState,
    essence: Option<&EssenceSession>,
    avoid: &[AvoidRect],
    exemptions: ZoneExempt,
) -> Result<Route, RouteError> {
    if from == to {
        return Ok(Route {
            legs: vec![Leg::Walk { tiles: vec![from] }],
            dest: to,
            ticks: 0.0,
        });
    }
    let zones = zone_filter(graph, state, &[from], &ZoneExempt::NONE);

    let run = |zones: Option<&ZoneFilter<'_>>| {
        run_single_search(
            collision,
            graph,
            from,
            to,
            model,
            budget,
            use_teleports,
            allow_wilderness,
            state,
            essence,
            avoid,
            zones,
        )
    };
    let safe_impossible = zones.as_ref().is_some_and(|filter| {
        zone_goal_proof(
            collision,
            graph,
            from,
            &[to],
            FindOptions {
                allow_teleports: use_teleports,
                allow_wilderness,
                essence: essence.copied(),
                ..FindOptions::default()
            },
            state,
            Relax::Strict,
            filter,
        )
        .proof
            == ReverseProof::Unreachable
    });
    let mut result = if safe_impossible {
        Err(RouteError::NoPath)
    } else {
        run(zones.as_ref())
    };
    if let Some(filter) = zones.as_ref().filter(|_| {
        matches!(
            result,
            Err(RouteError::NoPath | RouteError::BudgetExhausted)
        )
    }) {
        let goal_completion = filter
            .blocking_at(&graph.wilderness, to)
            .any(|index| !filter.origin_active(&graph.wilderness, index));
        for granted in [ZoneExempt::NONE, exemptions]
            .into_iter()
            .take(1 + usize::from(exemptions != ZoneExempt::NONE))
        {
            if granted == ZoneExempt::NONE && !goal_completion {
                continue;
            }
            let completion = zone_filter(graph, state, &[from, to], &granted);
            result = run(completion.as_ref());
            if !matches!(
                result,
                Err(RouteError::NoPath | RouteError::BudgetExhausted)
            ) {
                break;
            }
        }
    }
    result
}

/// Count actual kernel invocations in a request, including endpoint and
/// diagnostic passes. Thread-local scope follows the route worker, not a
/// process-wide counter affected by other bots. Production has no counter.
#[cfg(any(test, feature = "test-support"))]
pub fn count_kernel_searches<T>(work: impl FnOnce() -> T) -> (T, usize) {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            KERNEL_SEARCHES.with(|count| count.set(self.0));
        }
    }
    let restore = Restore(KERNEL_SEARCHES.with(|count| count.replace(Some(0))));
    let result = work();
    let searches = KERNEL_SEARCHES.with(|count| count.get().unwrap_or(0));
    drop(restore);
    (result, searches)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static KERNEL_SEARCHES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The Dijkstra kernel shared by every search shape. `relax` is
/// [`Relax::Strict`] for every routing search; the diagnosis arms relax
/// only the carry/wear and `Unknown` stage gates a session can supply, so a
/// caller can tell what is missing from a skill/quest/varp gate, a `False`
/// stage gate or a hole in the graph. `budget` caps every
/// search; within it, `goals` decide at each settled node whether the
/// search goes on (a first-goal search's sets run their own budgets and
/// backward proofs).
#[allow(clippy::too_many_arguments)]
async fn search_kernel(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
    state: &WorldState,
    essence: Option<&EssenceSession>,
    relax: Relax,
    avoid: &[AvoidRect],
    zones: Option<&ZoneFilter<'_>>,
    goals: &mut Goals<'_>,
    deadline: Option<Instant>,
    // Cursor jobs reserve lazy-zeroed full backing; legacy facades grow a small table.
    cooperative_table_init: bool,
    progress: Option<&AtomicUsize>,
) -> KernelOutcome {
    #[cfg(any(test, feature = "test-support"))]
    KERNEL_SEARCHES.with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
    work_unit().await;
    match ResourceBudget::new(graph, state, use_teleports, relax).await {
        Ok(Some(resources)) => {
            search_kernel_budget(
                collision,
                graph,
                from,
                model,
                budget,
                use_teleports,
                allow_wilderness,
                state,
                essence,
                relax,
                avoid,
                zones,
                goals,
                deadline,
                cooperative_table_init,
                progress,
                resources,
            )
            .await
        }
        Ok(None) => {
            search_kernel_budget(
                collision,
                graph,
                from,
                model,
                budget,
                use_teleports,
                allow_wilderness,
                state,
                essence,
                relax,
                avoid,
                zones,
                goals,
                deadline,
                cooperative_table_init,
                progress,
                Unmetered,
            )
            .await
        }
        Err(()) => KernelOutcome {
            stop: SearchStop::Budget,
            ..KernelOutcome::empty()
        },
    }
}

#[inline(always)]
#[expect(
    clippy::manual_async_fn,
    reason = "the explicit block must force-inline its poll body on Rust 1.98.1"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the compact relaxation updates the shared table and heap without copying state"
)]
fn relax_packed<'table, 'heap, 'pending, B: Budget>(
    table: &'table mut PackedSearchTable,
    heap: &'heap mut PackedMinMaxHeap,
    maximum_table_capacity: usize,
    key: B::Key,
    cost: f64,
    parent: u32,
    action: u8,
    action_index: usize,
    pending_limit: usize,
    pending_after_limit: &'pending mut bool,
) -> impl Future<Output = Result<(), SearchStop>> + use<'table, 'heap, 'pending, B> {
    // The pinned compiler weakens an async-fn attribute to an inline hint.
    // Put the attribute on the poll body to retain forced inlining.
    #[inline(always)]
    async move {
        let balance = key.tie();
        if balance > 0x3fff {
            return Err(SearchStop::Budget);
        }
        let action_index = u32::try_from(action_index).map_err(|_| SearchStop::Budget)?;
        if action_index > 0x00ff_ffff {
            return Err(SearchStop::Budget);
        }
        let lookup = loop {
            // Legacy searches do not construct and poll a lookup future per edge.
            // Cooperative searches retain their collision-by-collision suspension.
            let result = if SYNCHRONOUS_WORK.get() {
                table.locate_for_insert(key.tile(), balance)
            } else {
                table.locate(key.tile(), balance).await
            };
            match result {
                Ok(lookup) => break lookup,
                Err(()) if pending_limit == 0 => {
                    *pending_after_limit = true;
                    return Ok(());
                }
                Err(()) => match table.try_grow(heap, maximum_table_capacity) {
                    Ok(true) => {}
                    Ok(false) => return Err(SearchStop::Budget),
                    Err(_) => return Err(SearchStop::Allocation),
                },
            }
        };
        match lookup {
            PackedLookup::Found(id) => {
                if table.is_settled(id) || table.slot(id).cost() <= cost {
                    return Ok(());
                }
                table.update(id, action, action_index, cost);
                heap.decrease_key(table, id, parent);
            }
            PackedLookup::Vacant(id) => {
                let mut id = id;
                if pending_limit == 0 {
                    *pending_after_limit = true;
                    return Ok(());
                }
                if heap.len() >= pending_limit {
                    let worst = heap.worst_id(table).await.ok_or(SearchStop::Budget)?;
                    if !table
                        .compare_candidate(cost, key.tile(), balance, worst)
                        .is_lt()
                    {
                        return Ok(());
                    }
                    let removed = heap.remove_worst(table).await.ok_or(SearchStop::Budget)?;
                    table.tombstone(removed).await;
                    id = table.vacant_id().ok_or(SearchStop::Budget)?;
                }
                table.insert(id, key.tile(), balance, action, action_index, cost);
                heap.push(table, id, parent);
            }
        }
        Ok(())
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the budgeted kernel keeps its shared search inputs explicit"
)]
async fn search_kernel_budget<B: Budget>(
    collision: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    model: CostModel,
    budget: usize,
    use_teleports: bool,
    allow_wilderness: bool,
    state: &WorldState,
    essence: Option<&EssenceSession>,
    relax: Relax,
    avoid: &[AvoidRect],
    zones: Option<&ZoneFilter<'_>>,
    goals: &mut Goals<'_>,
    deadline: Option<Instant>,
    cooperative_table_init: bool,
    progress: Option<&AtomicUsize>,
    mut resources: B,
) -> KernelOutcome {
    let record_capacities = matches!(goals, Goals::First(_) | Goals::Many { .. });
    let Some(maximum_table_capacity) = PackedSearchTable::capacity_for(budget) else {
        return KernelOutcome {
            stop: SearchStop::Budget,
            ..KernelOutcome::empty()
        };
    };
    let initial_settle_budget = if cooperative_table_init {
        budget
    } else {
        budget.min(SYNC_FIRST_TABLE_INITIAL_SETTLES)
    };
    let mut came_from = match PackedSearchTable::try_new(initial_settle_budget) {
        Ok(table) => table,
        Err(_) => {
            return KernelOutcome {
                stop: SearchStop::Allocation,
                ..KernelOutcome::empty()
            };
        }
    };
    let Some(mut heap) = PackedMinMaxHeap::try_new(came_from.capacity()) else {
        return KernelOutcome {
            stop: SearchStop::Budget,
            ..KernelOutcome::empty()
        };
    };
    let mut reached = HashMap::new();
    if B::METERED {
        let Some(target_count) = goals.target_count() else {
            return KernelOutcome {
                stop: SearchStop::Budget,
                ..KernelOutcome::empty()
            };
        };
        if reached.try_reserve(target_count).is_err() {
            return KernelOutcome {
                stop: SearchStop::Budget,
                ..KernelOutcome::empty()
            };
        }
    }

    // Teleports leave from every settled tile. Resolve their immutable gates
    // once, keeping original indexes for reconstruction; metered carried
    // requirements are checked against each label before the hop is debited.
    let mut allowed_teleports = Vec::new();
    if use_teleports {
        if allowed_teleports
            .try_reserve_exact(graph.teleports.len())
            .is_err()
        {
            return KernelOutcome {
                stop: SearchStop::Budget,
                ..KernelOutcome::empty()
            };
        }
        for (index, edge) in graph.teleports.iter().enumerate() {
            work_unit().await;
            if resources.allowed(state, edge, relax) {
                allowed_teleports.push(index);
            }
        }
    }

    let origin = resources.start(from);
    let origin_id = match came_from.locate(origin.tile(), origin.tie()).await {
        Ok(PackedLookup::Vacant(id)) => id,
        _ => {
            return KernelOutcome {
                stop: SearchStop::Budget,
                ..KernelOutcome::empty()
            };
        }
    };
    came_from.insert(origin_id, from, origin.tie(), 0, 0, 0.0);
    heap.push(&mut came_from, origin_id, PACKED_ROOT_ID);

    let mut expanded = 0usize;
    let mut heap_pops = 0usize;
    let expired = || deadline.is_some_and(|end| Instant::now() >= end);
    let mut budget_spent = false;
    let mut budget_stop = SearchStop::Budget;
    let mut pending_after_limit = false;
    'search: while heap.len() != 0 {
        work_unit().await;
        if heap_pops & 255 == 0 && expired() {
            return KernelOutcome::finish(
                came_from,
                &heap,
                expanded,
                SearchStop::Deadline,
                record_capacities,
                goals.finish_proofs(),
                zones.map_or(0, ZoneFilter::mask_words),
                resources,
                reached,
            );
        }
        heap_pops += 1;
        let (node_id, parent_id) = heap.pop_min(&mut came_from).expect("nonempty packed heap");
        let node = *came_from.slot(node_id);
        let cur = node.tile();
        let current = B::Key::from_parts(cur, node.balance());
        let current_cost = node.cost();
        came_from.settle(node_id, parent_id);
        if let Some(progress) = progress {
            progress.fetch_add(1, AtomicOrdering::Relaxed);
        }
        expanded += 1;
        if B::METERED && goals.is_target(cur) {
            reached.entry(cur).or_insert(node.balance());
        }
        if expanded > budget && !matches!(goals, Goals::First(_)) {
            return KernelOutcome::finish(
                came_from,
                &heap,
                budget,
                SearchStop::Budget,
                record_capacities,
                goals.finish_proofs(),
                zones.map_or(0, ZoneFilter::mask_words),
                resources,
                reached,
            );
        }
        let cost = TargetCost {
            ticks: current_cost,
            settled_at: expanded,
        };
        let decision = match goals {
            Goals::First(first) => first.settle(cur, cost).await,
            _ => Ok(goals.settle_non_first(cur, cost)),
        };
        let stop = match decision {
            Ok(Settle::Continue) => None,
            Ok(Settle::Stop) => Some((expanded, SearchStop::Completed)),
            Ok(Settle::Spent) => Some((expanded - 1, SearchStop::Budget)),
            Err(_) => Some((expanded - 1, SearchStop::Budget)),
        };
        if let Some((settled, stop)) = stop {
            return KernelOutcome::finish(
                came_from,
                &heap,
                settled,
                if expired() {
                    SearchStop::Deadline
                } else {
                    stop
                },
                record_capacities,
                goals.finish_proofs(),
                zones.map_or(0, ZoneFilter::mask_words),
                resources,
                reached,
            );
        }
        if expanded > budget {
            return KernelOutcome::finish(
                came_from,
                &heap,
                budget,
                SearchStop::Budget,
                record_capacities,
                goals.finish_proofs(),
                zones.map_or(0, ZoneFilter::mask_words),
                resources,
                reached,
            );
        }
        if zones.is_some_and(|filter| {
            goals.is_target(cur) && filter.destination_only_at(&graph.wilderness, cur)
        }) {
            continue;
        }

        let escaping = !avoid.is_empty() && tile_in_any_avoid(cur, avoid);
        // One additional label observes whether work exists past the logical
        // budget. FirstGoals needs that ordinal to distinguish a spent
        // fallback from an exhausted component. It is never expanded.
        let pending_limit = budget.saturating_add(1).saturating_sub(expanded);

        // Eight fixed collision steps share this expansion's work unit.
        // Variable-length hash chains and transport lists still yield.
        for d in STEPS {
            if step_ok(collision, cur, d) {
                let nb = WorldTile {
                    x: cur.x + d.0,
                    z: cur.z + d.1,
                    level: cur.level,
                };
                if !avoid.is_empty() && !escaping && tile_in_any_avoid(nb, avoid) {
                    continue;
                }
                if !wildy_step_ok(graph, cur, nb, allow_wilderness) {
                    continue;
                }
                if let Some(filter) = zones {
                    let mut blocked =
                        filter.blocking_transition_at(&graph.wilderness, cur, nb, &|| {
                            goals.is_target(nb)
                        });
                    if let Some(first) = blocked.next() {
                        goals
                            .record_blockers(std::iter::once(first).chain(blocked))
                            .await;
                        continue;
                    }
                }
                let next = current.at(nb);
                if let Err(stop) = relax_packed::<B>(
                    &mut came_from,
                    &mut heap,
                    maximum_table_capacity,
                    next,
                    current_cost + model.run_per_step,
                    node_id,
                    0,
                    0,
                    pending_limit,
                    &mut pending_after_limit,
                )
                .await
                {
                    budget_spent = true;
                    budget_stop = stop;
                    break 'search;
                }
            }
        }
        // Footprint edges are indexed at each admissible stand; process those
        // only at cur, not again through neighbouring index entries. Other
        // transports remain indexed at their radius-one target anchor.
        if collision.standable(cur) {
            for dx in -INTERACT_RADIUS..=INTERACT_RADIUS {
                for dz in -INTERACT_RADIUS..=INTERACT_RADIUS {
                    let at = WorldTile {
                        x: cur.x + dx,
                        z: cur.z + dz,
                        level: cur.level,
                    };
                    let Some(idxs) = graph.at.get(&at) else {
                        continue;
                    };
                    for &ei in idxs {
                        work_unit().await;
                        if at != cur && graph.approaches.get(ei).is_some_and(Option::is_some) {
                            continue;
                        }
                        let edge = &graph.edges[ei];
                        if !graph.admissible_from(collision, ei, cur) {
                            continue;
                        }
                        if !resources.allowed(state, edge, relax) {
                            continue;
                        }
                        if prefer_worn_slash_for_web(graph, edge, state, relax) {
                            continue;
                        }
                        let Some(to) = edge.landing_from(cur) else {
                            continue;
                        };
                        if !avoid.is_empty() && !escaping && tile_in_any_avoid(to, avoid) {
                            continue;
                        }
                        if !wildy_step_ok(graph, cur, to, allow_wilderness) {
                            continue;
                        }
                        if let Some(filter) = zones {
                            let mut blocked =
                                filter.blocking_transition_at(&graph.wilderness, cur, to, &|| {
                                    goals.is_target(to)
                                });
                            if let Some(first) = blocked.next() {
                                if goals.is_frontier() {
                                    match resources.cross(current, edge, state, relax).await {
                                        Ok(Some(_)) => {
                                            goals
                                                .record_blockers(
                                                    std::iter::once(first).chain(blocked),
                                                )
                                                .await
                                        }
                                        Ok(None) => {}
                                        Err(()) => {
                                            budget_spent = true;
                                            break 'search;
                                        }
                                    }
                                }
                                continue;
                            }
                        }
                        let next = match resources.cross(current, edge, state, relax).await {
                            Ok(Some(next)) => next.at(to),
                            Ok(None) => continue,
                            Err(()) => {
                                budget_spent = true;
                                break 'search;
                            }
                        };
                        if let Err(stop) = relax_packed::<B>(
                            &mut came_from,
                            &mut heap,
                            maximum_table_capacity,
                            next,
                            current_cost + edge.ticks as f64,
                            node_id,
                            1,
                            ei,
                            pending_limit,
                            &mut pending_after_limit,
                        )
                        .await
                        {
                            budget_spent = true;
                            budget_stop = stop;
                            break 'search;
                        }
                    }
                }
            }
            // The essence-mine return: never packed — the pack carries
            // wizard → mine-pad entry edges only. When a session is
            // latched, each mine exit portal placement is relaxed from any
            // standable tile within the interact radius, landing on the
            // entry wizard's overworld anchor. Without a session the mine
            // is a sealed dead end, so `find` never treats it as a
            // corridor between arbitrary overworld tiles.
            if let Some(session) = essence {
                for (portal, &at) in ESSENCE_MINE_PORTALS.iter().enumerate() {
                    work_unit().await;
                    if cur.level != at.level
                        || (cur.x - at.x).abs().max((cur.z - at.z).abs()) > INTERACT_RADIUS
                    {
                        continue;
                    }
                    if !avoid.is_empty()
                        && !escaping
                        && tile_in_any_avoid(session.return_tile, avoid)
                    {
                        continue;
                    }
                    if !wildy_step_ok(graph, cur, session.return_tile, allow_wilderness) {
                        continue;
                    }
                    if let Some(filter) = zones {
                        let mut blocked = filter.blocking_transition_at(
                            &graph.wilderness,
                            cur,
                            session.return_tile,
                            &|| goals.is_target(session.return_tile),
                        );
                        if let Some(first) = blocked.next() {
                            goals
                                .record_blockers(std::iter::once(first).chain(blocked))
                                .await;
                            continue;
                        }
                    }
                    if let Err(stop) = relax_packed::<B>(
                        &mut came_from,
                        &mut heap,
                        maximum_table_capacity,
                        current.at(session.return_tile),
                        current_cost + ESSENCE_MINE_EXIT_TICKS as f64,
                        node_id,
                        3,
                        portal,
                        pending_limit,
                        &mut pending_after_limit,
                    )
                    .await
                    {
                        budget_spent = true;
                        budget_stop = stop;
                        break 'search;
                    }
                }
            }
        }
        // The any-tile teleport layer: every teleport edge leaves from the
        // current node, wherever it is. The landing is trusted like every
        // other transport `to` (no walkability filter — the content
        // declares it).
        if !allowed_teleports.is_empty() {
            let wildy_level = graph.wilderness.level(cur);
            for &ti in &allowed_teleports {
                work_unit().await;
                let edge = &graph.teleports[ti];
                if edge.takeoff.is_some_and(|takeoff| takeoff != cur) {
                    continue;
                }
                if !avoid.is_empty() && !escaping && tile_in_any_avoid(edge.to, avoid) {
                    continue;
                }
                if !wildy_step_ok(graph, cur, edge.to, allow_wilderness)
                    || !TransportGraph::teleport_legal_at_level(wildy_level, edge)
                {
                    continue;
                }
                if let Some(filter) = zones {
                    let mut blocked =
                        filter.blocking_transition_at(&graph.wilderness, cur, edge.to, &|| {
                            goals.is_target(edge.to)
                        });
                    if let Some(first) = blocked.next() {
                        if goals.is_frontier() {
                            match resources.cross(current, edge, state, relax).await {
                                Ok(Some(_)) => {
                                    goals
                                        .record_blockers(std::iter::once(first).chain(blocked))
                                        .await
                                }
                                Ok(None) => {}
                                Err(()) => {
                                    budget_spent = true;
                                    break 'search;
                                }
                            }
                        }
                        continue;
                    }
                }
                let next = match resources.cross(current, edge, state, relax).await {
                    Ok(Some(next)) => next.at(edge.to),
                    Ok(None) => continue,
                    Err(()) => {
                        budget_spent = true;
                        break 'search;
                    }
                };
                if let Err(stop) = relax_packed::<B>(
                    &mut came_from,
                    &mut heap,
                    maximum_table_capacity,
                    next,
                    current_cost + edge.ticks as f64,
                    node_id,
                    2,
                    ti,
                    pending_limit,
                    &mut pending_after_limit,
                )
                .await
                {
                    budget_spent = true;
                    budget_stop = stop;
                    break 'search;
                }
            }
        }
    }
    let stop = if budget_stop == SearchStop::Allocation {
        SearchStop::Allocation
    } else if expired() {
        SearchStop::Deadline
    } else if budget_spent || pending_after_limit {
        SearchStop::Budget
    } else {
        match goals.exhausted(expanded).await {
            Ok(()) => SearchStop::Exhausted,
            Err(_) => SearchStop::Budget,
        }
    };
    KernelOutcome::finish(
        came_from,
        &heap,
        expanded,
        stop,
        record_capacities,
        goals.finish_proofs(),
        zones.map_or(0, ZoneFilter::mask_words),
        resources,
        reached,
    )
}

/// Whether the search may move from `cur` onto `next`: without
/// `allow_wilderness` a non-wilderness node may not relax into a
/// wilderness tile (a walk step or a transport landing). Once inside the
/// wilderness the search walks freely — only the entry is gated. Membership
/// is the packed [`TransportGraph::wilderness`] table; empty zones gate
/// nothing.
fn wildy_step_ok(
    graph: &TransportGraph,
    cur: WorldTile,
    next: WorldTile,
    allow_wilderness: bool,
) -> bool {
    allow_wilderness || graph.wilderness.contains(cur) || !graph.wilderness.contains(next)
}

/// Whether a one-tile step from `cur` by `d` is allowed — the client's
/// `tryMove` movement test against the collision bake's derived walkable
/// word, not the blanket `walkable()`. A step into a neighbour clears that
/// neighbour's `PL_WALK_*` mask for the face/corner the step enters
/// through; a diagonal step additionally clears both orthogonal neighbours'
/// cardinal masks, exactly like `tryMove`'s BFS. The derived word carries
/// the `SQ_BLOCKED` base on any wall/scenery/ground tile, so those tiles
/// reject entry from every direction. Every step stays inside the bake's
/// x/z grid (the whole-world mapsquare bbox); each level tests its own
/// plane (an unstamped plane is unrestricted within it).
pub fn step_ok(collision: &WorldCollision, cur: WorldTile, d: (i32, i32)) -> bool {
    let nb = WorldTile {
        x: cur.x + d.0,
        z: cur.z + d.1,
        level: cur.level,
    };
    let lx = nb.x - collision.origin.x;
    let lz = nb.z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return false;
    }
    if lx as usize >= collision.width || lz as usize >= collision.height {
        return false;
    }
    let f = |x: i32, z: i32| collision.walkable_word(x, z, nb.level);
    if let Some(mask) = cardinal_entry_mask(d) {
        // Cardinal: the destination's face toward `cur`.
        return f(nb.x, nb.z) & mask == 0;
    }
    match (d.0, d.1) {
        // Diagonal: the destination's corner mask plus both orthogonals.
        (-1, -1) => {
            f(nb.x, nb.z) & MASK_NE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (1, -1) => {
            f(nb.x, nb.z) & MASK_NW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (-1, 1) => {
            f(nb.x, nb.z) & MASK_SE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        (1, 1) => {
            f(nb.x, nb.z) & MASK_SW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        _ => false,
    }
}

/// The `PL_WALK_*` mask a cardinal step by `d` tests on the tile it enters
/// (the face toward the tile it leaves); `None` for any other delta.
pub(crate) fn cardinal_entry_mask(d: (i32, i32)) -> Option<u32> {
    match d {
        (0, 1) => Some(MASK_S),
        (0, -1) => Some(MASK_N),
        (1, 0) => Some(MASK_W),
        (-1, 0) => Some(MASK_E),
        _ => None,
    }
}

/// Return the component reachable from `origin` using baked client-style
/// walk steps within the requested Chebyshev radius. This deliberately omits
/// transports and validates its own origin so callers cannot turn it into an
/// unbounded or cross-plane flood.
pub fn local_step_component(
    collision: &WorldCollision,
    origin: WorldTile,
    radius: i32,
) -> HashSet<WorldTile> {
    let mut visited = HashSet::new();
    let r = radius.clamp(0, 104);
    if !collision.standable(origin) {
        return visited;
    }
    let mut queue = std::collections::VecDeque::from([origin]);
    visited.insert(origin);
    while let Some(cur) = queue.pop_front() {
        for d in STEPS {
            let next = WorldTile {
                x: cur.x + d.0,
                z: cur.z + d.1,
                level: origin.level,
            };
            if (next.x - origin.x).abs().max((next.z - origin.z).abs()) > r
                || next.level != origin.level
                || !collision.standable(next)
                || !step_ok(collision, cur, d)
                || !visited.insert(next)
            {
                continue;
            }
            queue.push_back(next);
        }
    }
    visited
}

/// Split the backtrack from `to` back to the entry-less origin into legs:
/// consecutive walk tiles collapse into one `Walk` leg per run, and each
/// transport edge is its own `Transport` leg. Returns `(legs, ticks)`
/// where `ticks` is the total: `(walk tiles − 1) × run rate` per walk leg
/// plus each transport edge's ticks.
fn reconstruct(
    to: WorldTile,
    came_from: &Predecessors,
    graph: &TransportGraph,
    model: CostModel,
    essence: Option<&EssenceSession>,
) -> (Vec<Leg>, f64) {
    came_from.reconstruct(to, graph, model, essence)
}

fn reconstruct_packed(
    came_from: &Predecessors,
    to: WorldTile,
    graph: &TransportGraph,
    model: CostModel,
    essence: Option<&EssenceSession>,
) -> (Vec<Leg>, f64) {
    let mut trace = came_from
        .backtrace(to)
        .expect("successful search retains the target predecessor");
    let mut walk_rev = vec![to];
    let mut legs_rev = Vec::new();
    let mut ticks = 0.0;
    while let Some(step) = trace.next() {
        match step {
            BacktraceStep::Walk { .. } => walk_rev.push(trace.current_tile()),
            BacktraceStep::Transport { tile, index } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let mut edge = graph.edges[index].clone();
                if edge.player_delta.is_some() {
                    edge.to = tile;
                }
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport {
                    edge: Box::new(edge),
                });
                walk_rev = vec![trace.current_tile()];
            }
            BacktraceStep::Teleport { index, .. } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let edge = graph.teleports[index].clone();
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport {
                    edge: Box::new(edge),
                });
                walk_rev = vec![trace.current_tile()];
            }
            BacktraceStep::EssenceReturn { portal, .. } => {
                ticks += walk_ticks(&walk_rev, model);
                walk_rev.reverse();
                legs_rev.push(Leg::Walk { tiles: walk_rev });
                let session = essence.expect("essence return implies a latched session");
                let edge =
                    crate::essence::essence_return_edge(ESSENCE_MINE_PORTALS[portal], session);
                ticks += edge.ticks as f64;
                legs_rev.push(Leg::Transport {
                    edge: Box::new(edge),
                });
                walk_rev = vec![trace.current_tile()];
            }
        }
    }
    ticks += walk_ticks(&walk_rev, model);
    walk_rev.reverse();
    legs_rev.push(Leg::Walk { tiles: walk_rev });
    legs_rev.reverse();
    (legs_rev, ticks)
}

/// The tick cost of one collapsed walk leg: `tiles.len() − 1` steps at the
/// model's run rate (the origin tile is stood on, not walked to).
fn walk_ticks(tiles: &[WorldTile], model: CostModel) -> f64 {
    tiles.len().saturating_sub(1) as f64 * model.run_per_step
}

mod grid;
pub use grid::{find_on_grid, GridLeg, GridRoute, NoPath};

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
