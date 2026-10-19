//! Cooperative temporary BankBudget facts for a first-goal search.
//!
//! The caller transfers its sole owned pre-state into a search Arc. This cursor
//! applies the same first-row-per-id overlay as [`super::fetchable_state`],
//! then restores those exact facts in place after the search returns the Arc.
//! It never clones a [`WorldState`].

use std::collections::HashSet;

use crate::transport::TransportGraph;
use crate::world_state::{map_storage_bound, WorldState};

/// Why a cooperative BankBudget fact cursor could not be prepared or advanced.
#[derive(Debug, thiserror::Error)]
pub enum FetchableCursorError {
    /// Checked retained-byte arithmetic overflowed.
    #[error("fetchable-state cursor size arithmetic overflowed")]
    SizeOverflow,
    /// A fallible cursor or world-state reservation failed.
    #[error("fetchable-state cursor allocation failed")]
    Allocation(#[from] std::collections::TryReserveError),
}

/// Incremental BankBudget facts over one owned state.
///
/// Each call to [`Self::advance`] processes at most one bank row. The map is
/// reserved before the first insertion, so later row steps do not trigger a
/// rehash. `undo` records only ids that the first eligible bank row changes;
/// [`Self::restore_next`] restores them one at a time.
#[derive(Debug)]
pub struct FetchableStateCursor {
    rows: Vec<(i32, i32)>,
    seen: HashSet<i32>,
    undo: Vec<(i32, Option<i32>)>,
    row: usize,
    restore: usize,
    original_allow_fetchable_worn: bool,
    has_stands: bool,
    phase: StatePhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatePhase {
    Reserve,
    Overlay,
    Ready,
    Restore,
    Restored,
    Taken,
}

impl FetchableStateCursor {
    /// Checked scratch peak excluding the input bank rows and owned state.
    ///
    /// `growth_bound` covers the new inventory table beside the captured table
    /// during a possible rehash. The hash set and undo vector use the bank-row
    /// count as their maximum distinct length.
    pub fn heap_bound(state: &WorldState, bank_rows: usize, has_stands: bool) -> Option<usize> {
        if !has_stands {
            return Some(0);
        }
        let growth = state.growth_bound(bank_rows, 0)?;
        let set = map_storage_bound(
            bank_rows,
            std::mem::size_of::<i32>(),
            std::mem::align_of::<i32>(),
        )?;
        let undo = bank_rows.checked_mul(std::mem::size_of::<(i32, Option<i32>)>())?;
        growth
            .inventory_new_bytes
            .checked_add(set)?
            .checked_add(undo)
    }

    /// Take ownership of the bank rows to be overlaid. The same allocation is
    /// transferred back by [`Self::take_rows`] after restoration.
    pub fn new(
        state: &WorldState,
        rows: Vec<(i32, i32)>,
        has_stands: bool,
    ) -> Result<Self, FetchableCursorError> {
        let mut seen = HashSet::new();
        let mut undo = Vec::new();
        if has_stands {
            seen.try_reserve(rows.len())?;
            undo.try_reserve_exact(rows.len())?;
        }
        Ok(Self {
            rows,
            seen,
            undo,
            row: 0,
            restore: 0,
            original_allow_fetchable_worn: state.allow_fetchable_worn,
            has_stands,
            phase: StatePhase::Reserve,
        })
    }

    /// Apply at most one bank row, returning `true` when the overlay is ready.
    /// `allow_fetchable_worn` is set even when there are no bank stands, as in
    /// [`super::fetchable_state`].
    pub fn advance(&mut self, state: &mut WorldState) -> Result<bool, FetchableCursorError> {
        match self.phase {
            StatePhase::Reserve => {
                if self.has_stands {
                    state.inv.try_reserve(self.rows.len())?;
                }
                state.allow_fetchable_worn = true;
                self.phase = StatePhase::Overlay;
            }
            StatePhase::Overlay => {
                let Some(&(id, count)) = self.rows.get(self.row) else {
                    self.phase = StatePhase::Ready;
                    return Ok(true);
                };
                self.row += 1;
                // Match `fetchable_state`: the first row claims the id even
                // when its count is below one.
                if self.has_stands && self.seen.insert(id) && count >= 1 {
                    let previous = state.inv.get(&id).copied();
                    self.undo.push((id, previous));
                    let held = state.inv.entry(id).or_insert(0);
                    *held = held.saturating_add(count);
                }
            }
            StatePhase::Ready => return Ok(true),
            StatePhase::Restore | StatePhase::Restored | StatePhase::Taken => return Ok(false),
        }
        Ok(false)
    }

    /// Begin restoration after the search has returned the unique shared
    /// state handle. Returns `false` when the overlay is not ready.
    pub fn begin_restore(&mut self) -> bool {
        if self.phase != StatePhase::Ready {
            return false;
        }
        self.restore = self.undo.len();
        self.phase = StatePhase::Restore;
        true
    }

    /// Restore at most one changed id and the original worn-fact mode.
    /// Returns `true` when the owned pre-state is restored.
    pub fn restore_next(&mut self, state: &mut WorldState) -> bool {
        if self.phase == StatePhase::Restore {
            if self.restore > 0 {
                self.restore -= 1;
                let (id, previous) = self.undo[self.restore];
                if let Some(count) = previous {
                    state.inv.insert(id, count);
                } else {
                    state.inv.remove(&id);
                }
                return false;
            }
            state.allow_fetchable_worn = self.original_allow_fetchable_worn;
            self.phase = StatePhase::Restored;
        }
        self.phase == StatePhase::Restored
    }

    /// Transfer the original rows after restoration, exactly once.
    pub fn take_rows(&mut self) -> Option<Vec<(i32, i32)>> {
        if self.phase != StatePhase::Restored {
            return None;
        }
        self.phase = StatePhase::Taken;
        Some(std::mem::take(&mut self.rows))
    }

    /// Scratch retained by this cursor, excluding the input bank rows and the
    /// mutated `WorldState` tables.
    pub fn retained_bytes(&self) -> Option<usize> {
        let set = map_storage_bound(
            self.seen.capacity(),
            std::mem::size_of::<i32>(),
            std::mem::align_of::<i32>(),
        )?;
        let undo = self
            .undo
            .capacity()
            .checked_mul(std::mem::size_of::<(i32, Option<i32>)>())?;
        set.checked_add(undo)
    }

    /// The state has been overlaid and is ready for a cooperative search.
    pub fn ready(&self) -> bool {
        self.phase == StatePhase::Ready
    }
}

/// Incremental exact check for whether the temporary BankBudget state opens
/// any transport edge that the original state could not cross.
///
/// The first pass stores the original `WorldState::allows` predicate in a
/// packed bitset; the second pass checks the overlaid state in the same edges,
/// then optional teleports order as `fetchable_state`'s caller.
#[derive(Debug)]
pub struct FetchableEdgeCursor {
    strict: Box<[u64]>,
    unit: usize,
    phase: EdgePhase,
    include_teleports: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgePhase {
    Strict,
    Fetchable,
    Done(bool),
}

impl FetchableEdgeCursor {
    /// Checked bytes for the strict-edge bitset.
    pub fn heap_bound(graph: &TransportGraph, include_teleports: bool) -> Option<usize> {
        let count = graph.edges.len().checked_add(if include_teleports {
            graph.teleports.len()
        } else {
            0
        })?;
        count.div_ceil(64).checked_mul(std::mem::size_of::<u64>())
    }

    /// Allocate the strict-edge bitset before either scan begins.
    pub fn new(
        graph: &TransportGraph,
        include_teleports: bool,
    ) -> Result<Self, FetchableCursorError> {
        let count = graph
            .edges
            .len()
            .checked_add(if include_teleports {
                graph.teleports.len()
            } else {
                0
            })
            .ok_or(FetchableCursorError::SizeOverflow)?;
        let words = count.div_ceil(64);
        let mut strict = Vec::new();
        strict.try_reserve_exact(words)?;
        strict.resize(words, 0);
        Ok(Self {
            strict: strict.into_boxed_slice(),
            unit: 0,
            phase: EdgePhase::Strict,
            include_teleports,
        })
    }

    /// Visit at most one edge using the original state. Returns true after the
    /// strict predicate has been recorded for all considered edges.
    pub fn capture_strict(&mut self, graph: &TransportGraph, state: &WorldState) -> bool {
        if self.phase != EdgePhase::Strict {
            return true;
        }
        let count = self.edge_count(graph);
        if self.unit >= count {
            self.unit = 0;
            self.phase = EdgePhase::Fetchable;
            return true;
        }
        let allowed = self
            .edge(graph, self.unit)
            .is_some_and(|edge| state.allows(edge));
        if allowed {
            self.strict[self.unit / 64] |= 1_u64 << (self.unit % 64);
        }
        self.unit += 1;
        false
    }

    /// Visit at most one edge using the overlay, returning `Some(true)` on
    /// the first newly opened edge, `Some(false)` after all edges, or `None`
    /// while more units remain.
    pub fn opens_next(&mut self, graph: &TransportGraph, state: &WorldState) -> Option<bool> {
        if let EdgePhase::Done(opened) = self.phase {
            return Some(opened);
        }
        if self.phase != EdgePhase::Fetchable {
            return None;
        }
        let count = self.edge_count(graph);
        if self.unit >= count {
            self.phase = EdgePhase::Done(false);
            return Some(false);
        }
        let at = self.unit;
        let opened = self.edge(graph, at).is_some_and(|edge| {
            state.allows(edge) && self.strict[at / 64] & (1_u64 << (at % 64)) == 0
        });
        self.unit += 1;
        if opened {
            self.phase = EdgePhase::Done(true);
            Some(true)
        } else {
            None
        }
    }

    /// Strict predicate bitset bytes retained by this cursor.
    pub fn retained_bytes(&self) -> usize {
        self.strict.len() * std::mem::size_of::<u64>()
    }

    fn edge_count(&self, graph: &TransportGraph) -> usize {
        graph.edges.len()
            + if self.include_teleports {
                graph.teleports.len()
            } else {
                0
            }
    }

    fn edge<'a>(
        &self,
        graph: &'a TransportGraph,
        at: usize,
    ) -> Option<&'a crate::transport::TransportEdge> {
        if let Some(edge) = graph.edges.get(at) {
            return Some(edge);
        }
        self.include_teleports
            .then(|| graph.teleports.get(at - graph.edges.len()))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::{FetchableEdgeCursor, FetchableStateCursor};
    use crate::bank_fetch::fetchable_state;
    use crate::pack::{BankAccess, BankStand};
    use crate::transport::{TransportEdge, TransportGraph, TransportKind};
    use crate::world_state::WorldState;
    use api::snapshot::WorldTile;

    #[test]
    fn overlay_matches_fetchable_state_and_restores_original_without_cloning() {
        let mut state = WorldState::empty();
        state.inv.insert(1, 3);
        state.inv.insert(2, 0);
        state.allow_fetchable_worn = false;
        let rows = vec![(1, 5), (2, 0), (2, 7), (3, 4)];
        let expected = fetchable_state(&state, &rows, &[bank_stand()]);
        let original = state.clone();
        let mut cursor = FetchableStateCursor::new(&state, rows.clone(), true).unwrap();
        while !cursor.advance(&mut state).unwrap() {}
        assert_eq!(state, expected);
        assert!(cursor.begin_restore());
        while !cursor.restore_next(&mut state) {}
        assert_eq!(state, original);
        assert_eq!(cursor.take_rows().unwrap(), rows);
        assert!(cursor.take_rows().is_none());
    }

    #[test]
    fn overlay_without_stands_only_enables_fetchable_worn() {
        let mut state = WorldState::empty();
        state.inv.insert(1, 3);
        state.allow_fetchable_worn = false;
        let original = state.clone();
        let mut cursor = FetchableStateCursor::new(&state, vec![(1, 5), (2, 7)], false).unwrap();
        while !cursor.advance(&mut state).unwrap() {}
        assert!(state.allow_fetchable_worn);
        assert_eq!(state.inv, original.inv);
        assert!(cursor.begin_restore());
        while !cursor.restore_next(&mut state) {}
        assert_eq!(state, original);
    }

    #[test]
    fn edge_probe_preserves_edges_then_optional_teleports_order() {
        let id = 7;
        let mut graph = TransportGraph::default();
        let mut gated = edge();
        gated.item_req.push((id, 1));
        graph.edges.push(gated);
        let strict = WorldState::empty();
        let mut overlay = WorldState::empty();
        overlay.inv.insert(id, 1);
        let mut cursor = FetchableEdgeCursor::new(&graph, false).unwrap();
        while !cursor.capture_strict(&graph, &strict) {}
        assert_eq!(cursor.opens_next(&graph, &overlay), Some(true));

        let mut graph = TransportGraph::default();
        graph.edges.push(edge());
        let mut cursor = FetchableEdgeCursor::new(&graph, false).unwrap();
        while !cursor.capture_strict(&graph, &strict) {}
        assert_eq!(cursor.opens_next(&graph, &overlay), None);
        assert_eq!(cursor.opens_next(&graph, &overlay), Some(false));
    }

    #[test]
    fn edge_probe_ignores_teleports_when_disabled() {
        let id = 8;
        let mut graph = TransportGraph::default();
        let mut gated = edge();
        gated.item_req.push((id, 1));
        graph.teleports.push(gated);
        let strict = WorldState::empty();
        let mut overlay = WorldState::empty();
        overlay.inv.insert(id, 1);
        let mut cursor = FetchableEdgeCursor::new(&graph, false).unwrap();
        assert!(cursor.capture_strict(&graph, &strict));
        assert_eq!(cursor.opens_next(&graph, &overlay), Some(false));
        let mut cursor = FetchableEdgeCursor::new(&graph, true).unwrap();
        assert!(!cursor.capture_strict(&graph, &strict));
        assert!(cursor.capture_strict(&graph, &strict));
        assert_eq!(cursor.opens_next(&graph, &overlay), Some(true));
    }

    fn bank_stand() -> BankStand {
        BankStand {
            name: "Bank".to_owned(),
            tile: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            access: BankAccess::Booth { op: 0 },
        }
    }

    fn edge() -> TransportEdge {
        let tile = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        TransportEdge { kind: TransportKind::Door,
        at: tile,
        to: tile,
        takeoff: None,
        player_delta: None,
        loc_id: 0,
        option: 0,
        ticks: 0,
        dir: None,
        open_loc_id: None,
        skill_req: Vec::new(),
        item_req: Vec::new(),
        consumed_req: Vec::new(),
        item_returns: Vec::new(),
        quest_req: Vec::new(),
        varp_req: Vec::new(),
        worn_req: Vec::new(), worn_all_req: Vec::new(), forbidden_held: Vec::new(), forbidden_worn: Vec::new(), members_req: false,
        wildy_cap: None,
        quest_gates: None, }
    }
}
