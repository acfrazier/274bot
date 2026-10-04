//! Resource labels are shared by walk nodes and change only at paid hops.
//! Generous inventories keep the original tile-only search after a conservative
//! proof that no simple path can exhaust a stack.
use super::*;
use std::hash::Hash;

const RESOURCE_LIMIT: usize = 64;
const BALANCE_LIMIT: usize = 4096;

pub(super) trait SearchKey: Copy + Eq + Hash {
    fn tile(self) -> WorldTile;
    fn at(self, tile: WorldTile) -> Self;
    fn tie(self) -> u32;
}

impl SearchKey for WorldTile {
    fn tile(self) -> WorldTile {
        self
    }
    fn at(self, tile: WorldTile) -> Self {
        tile
    }
    fn tie(self) -> u32 {
        0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ResourceKey {
    tile: WorldTile,
    balance: u32,
}

impl SearchKey for ResourceKey {
    fn tile(self) -> WorldTile {
        self.tile
    }
    fn at(self, tile: WorldTile) -> Self {
        Self { tile, ..self }
    }
    fn tie(self) -> u32 {
        self.balance
    }
}

pub(super) trait Budget {
    type Key: SearchKey;
    const METERED: bool;
    fn start(&self, tile: WorldTile) -> Self::Key;
    fn allowed(&self, state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool;
    fn cross(
        &mut self,
        key: Self::Key,
        edge: &TransportEdge,
        state: &WorldState,
        relax: Relax,
    ) -> Result<Option<Self::Key>, ()>;
    fn finish(
        self,
        tree: HashMap<Self::Key, Back<Self::Key>>,
        reached: HashMap<WorldTile, Self::Key>,
    ) -> Predecessors;
}

pub(super) struct Unmetered;
impl Budget for Unmetered {
    type Key = WorldTile;
    fn start(&self, tile: WorldTile) -> WorldTile {
        tile
    }
    const METERED: bool = false;
    fn allowed(&self, state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool {
        edge_allowed(state, edge, relax)
    }
    fn cross(
        &mut self,
        key: WorldTile,
        _: &TransportEdge,
        _: &WorldState,
        _: Relax,
    ) -> Result<Option<WorldTile>, ()> {
        Ok(Some(key))
    }
    fn finish(
        self,
        tree: HashMap<WorldTile, Back>,
        _: HashMap<WorldTile, WorldTile>,
    ) -> Predecessors {
        Predecessors::Tiles(tree)
    }
}

pub(super) struct ResourceBudget {
    ids: Vec<i32>,
    balances: Vec<[i32; RESOURCE_LIMIT]>,
    intern: HashMap<[i32; RESOURCE_LIMIT], u32>,
}

impl ResourceBudget {
    pub(super) fn new(
        graph: &TransportGraph,
        state: &WorldState,
        teleports: bool,
        relax: Relax,
    ) -> Result<Option<Self>, ()> {
        if matches!(relax, Relax::CarryWorn | Relax::CarryWornUnknownQuest) {
            return Ok(None);
        }
        let edges = || {
            graph
                .edges
                .iter()
                .chain(graph.teleports.iter().filter(move |_| teleports))
        };
        // No paid hop can be the first one: no returns can then become available.
        if !edges().any(|edge| !edge.consumed_req.is_empty() && edge_allowed(state, edge, relax)) {
            return Ok(None);
        }
        // Unavailable consumables cannot become usable merely by spending
        // other items. Follow possible returns from initially usable hops
        // before budgeting gates; otherwise absent passes and charge families
        // force a metered search even with ample supply for every usable hop.
        let can_carry = |edge: &TransportEdge, returned: &[i32]| {
            edge.item_req.iter().all(|&(id, count)| {
                state.inv.get(&id).copied().unwrap_or(0) >= count || returned.contains(&id)
            }) && edge.consumed_req.iter().all(|&(id, packed_count)| {
                let carried = state.inv.get(&id).copied().unwrap_or(0);
                carried >= edge.consumption_count(id, packed_count, carried)
                    || returned.contains(&id)
            })
        };
        let mut possible_returns = Vec::new();
        loop {
            let mut changed = false;
            for edge in edges().filter(|edge| fixed_allowed(state, edge, relax)) {
                if !can_carry(edge, &possible_returns) {
                    continue;
                }
                for &(id, _) in &edge.item_returns {
                    if !possible_returns.contains(&id) {
                        possible_returns.push(id);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut totals = HashMap::<i32, i64>::new();
        let mut held = HashMap::<i32, i32>::new();
        let mut ids = Vec::new();
        for (index, edge) in edges().enumerate().filter(|(_, edge)| {
            fixed_allowed(state, edge, relax) && can_carry(edge, &possible_returns)
        }) {
            let multiplicity = if edge.player_delta.is_some() && index < graph.edges.len() {
                let (min, max) = graph.takeoff_bounds(index);
                (i64::from(max.x) - i64::from(min.x) + 1)
                    * (i64::from(max.z) - i64::from(min.z) + 1)
            } else {
                1
            };
            for &(id, packed_count) in &edge.consumed_req {
                let count = edge.consumption_count(id, packed_count, i32::MAX);
                let total = totals.entry(id).or_default();
                *total = total.saturating_add(i64::from(count).saturating_mul(multiplicity));
                ids.push(id);
            }
            ids.extend(edge.item_returns.iter().map(|&(id, _)| id));
            for &(id, count) in &edge.item_req {
                let peak = held.entry(id).or_default();
                *peak = (*peak).max(count);
            }
        }
        // Every shortest unconstrained path is simple (positive hop costs).
        // Each absolute landing is visited at most once; relative edges can
        // be used once per admissible takeoff. Ignoring credits gives an upper
        // bound, including a held gate after all spend.
        // A returned variant can unlock a held gate even with ample inputs.
        if totals.iter().all(|(&id, &total)| {
            i64::from(state.inv.get(&id).copied().unwrap_or(0))
                >= total.saturating_add(i64::from(held.get(&id).copied().unwrap_or(0)))
        }) && held.iter().all(|(&id, &count)| {
            state.inv.get(&id).copied().unwrap_or(0) >= count || !ids.contains(&id)
        }) {
            return Ok(None);
        }
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > RESOURCE_LIMIT {
            return Err(());
        }
        let mut initial = [0; RESOURCE_LIMIT];
        for (index, id) in ids.iter().enumerate() {
            initial[index] = state.inv.get(id).copied().unwrap_or(0).max(0);
        }
        Ok(Some(Self {
            ids,
            balances: vec![initial],
            intern: HashMap::from([(initial, 0)]),
        }))
    }

    fn count(&self, balance: &[i32; RESOURCE_LIMIT], state: &WorldState, id: i32) -> i32 {
        self.ids.binary_search(&id).map_or_else(
            |_| state.inv.get(&id).copied().unwrap_or(0),
            |index| balance[index],
        )
    }
}

pub(super) fn fixed_allowed(state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool {
    state.fixed_reqs_allow(edge)
        && state.worn_req_allows(edge)
        && state.worn_all_req_allows(edge)
        && match relax {
            Relax::Strict => state.quest_gates(edge) == Truth::True,
            Relax::UnknownQuest => state.quest_gates(edge) != Truth::False,
            Relax::CarryWorn | Relax::CarryWornUnknownQuest => unreachable!("unmetered diagnosis"),
        }
}

impl Budget for ResourceBudget {
    type Key = ResourceKey;
    fn start(&self, tile: WorldTile) -> ResourceKey {
        ResourceKey { tile, balance: 0 }
    }
    const METERED: bool = true;
    fn allowed(&self, state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool {
        fixed_allowed(state, edge, relax)
    }
    fn cross(
        &mut self,
        key: ResourceKey,
        edge: &TransportEdge,
        state: &WorldState,
        _: Relax,
    ) -> Result<Option<ResourceKey>, ()> {
        let mut balance = self.balances[key.balance as usize];
        if !edge
            .item_req
            .iter()
            .all(|&(id, count)| self.count(&balance, state, id) >= count)
        {
            return Ok(None);
        }
        if edge.consumed_req.is_empty() && edge.item_returns.is_empty() {
            return Ok(Some(key));
        }
        for &(id, packed_count) in &edge.consumed_req {
            let Ok(index) = self.ids.binary_search(&id) else {
                return Ok(None);
            };
            let count = edge.consumption_count(id, packed_count, balance[index]);
            if balance[index] < count {
                return Ok(None);
            }
            balance[index] -= count;
        }
        for &(id, count) in &edge.item_returns {
            let index = self.ids.binary_search(&id).map_err(|_| ())?;
            balance[index] = balance[index].saturating_add(count);
        }
        let index = if let Some(&index) = self.intern.get(&balance) {
            index
        } else {
            if self.balances.len() == BALANCE_LIMIT {
                return Err(());
            }
            let index = self.balances.len() as u32;
            self.balances.push(balance);
            self.intern.insert(balance, index);
            index
        };
        Ok(Some(ResourceKey {
            balance: index,
            ..key
        }))
    }
    fn finish(
        self,
        tree: HashMap<ResourceKey, Back<ResourceKey>>,
        reached: HashMap<WorldTile, ResourceKey>,
    ) -> Predecessors {
        Predecessors::Resources { tree, reached }
    }
}

pub(super) enum Predecessors {
    Tiles(HashMap<WorldTile, Back>),
    Resources {
        tree: HashMap<ResourceKey, Back<ResourceKey>>,
        reached: HashMap<WorldTile, ResourceKey>,
    },
}

impl Predecessors {
    pub(super) fn empty() -> Self {
        Self::Tiles(HashMap::new())
    }
    pub(super) fn capacity(&self) -> usize {
        match self {
            Self::Tiles(tree) => tree.capacity(),
            Self::Resources { tree, reached } => tree.capacity() + reached.capacity(),
        }
    }
    pub(super) fn reconstruct(
        &self,
        to: WorldTile,
        graph: &TransportGraph,
        model: CostModel,
        essence: Option<&EssenceSession>,
    ) -> (Vec<Leg>, f64) {
        match self {
            Self::Tiles(tree) => reconstruct_key(to, tree, graph, model, essence),
            Self::Resources { tree, reached } => {
                reconstruct_key(reached[&to], tree, graph, model, essence)
            }
        }
    }
}
