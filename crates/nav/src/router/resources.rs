//! Resource labels are shared by walk nodes and change only at paid hops.
//! Generous inventories keep the original tile-only search after a conservative
//! proof that no simple path can exhaust a stack.
//! Hash a complete search identity in one write, retaining the original scalar
//! bytes so larger policy dispatch does not stream each field on the hot path.
use super::*;
use std::hash::{Hash, Hasher};

const RESOURCE_LIMIT: usize = 64;
const BALANCE_LIMIT: usize = 4096;

pub(super) trait SearchKey: Copy + Eq + Hash {
    fn tile(self) -> WorldTile;
    fn at(self, tile: WorldTile) -> Self;
    fn tie(self) -> u32;
}

#[inline]
fn hash_key<H: Hasher>(tile: WorldTile, balance: Option<u32>, state: &mut H) {
    let mut bytes = [0; 16];
    bytes[..4].copy_from_slice(&tile.x.to_ne_bytes());
    bytes[4..8].copy_from_slice(&tile.z.to_ne_bytes());
    bytes[8..12].copy_from_slice(&tile.level.to_ne_bytes());
    if let Some(balance) = balance {
        bytes[12..].copy_from_slice(&balance.to_ne_bytes());
        state.write(&bytes);
    } else {
        state.write(&bytes[..12]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(super) struct TileKey(WorldTile);

impl Hash for TileKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_key(self.0, None, state);
    }
}

impl SearchKey for TileKey {
    fn tile(self) -> WorldTile {
        self.0
    }
    fn at(self, tile: WorldTile) -> Self {
        Self(tile)
    }
    fn tie(self) -> u32 {
        0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ResourceKey {
    tile: WorldTile,
    balance: u32,
}

impl Hash for ResourceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_key(self.tile, Some(self.balance), state);
    }
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
    type Key = TileKey;
    fn start(&self, tile: WorldTile) -> TileKey {
        TileKey(tile)
    }
    const METERED: bool = false;
    fn allowed(&self, state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool {
        edge_allowed(state, edge, relax)
    }
    fn cross(
        &mut self,
        key: TileKey,
        _: &TransportEdge,
        _: &WorldState,
        _: Relax,
    ) -> Result<Option<TileKey>, ()> {
        Ok(Some(key))
    }
    fn finish(
        self,
        tree: HashMap<TileKey, Back<TileKey>>,
        _: HashMap<WorldTile, TileKey>,
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
        && (edge.worn_req.is_empty() || edge.worn_req.iter().any(|id| state.worn.contains(id)))
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
    Tiles(HashMap<TileKey, Back<TileKey>>),
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
            Self::Tiles(tree) => reconstruct_key(TileKey(to), tree, graph, model, essence),
            Self::Resources { tree, reached } => {
                reconstruct_key(reached[&to], tree, graph, model, essence)
            }
        }
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;
    use std::hash::Hasher;

    #[derive(Default)]
    struct Writes {
        count: usize,
        len: usize,
        bytes: [u8; 16],
    }

    impl Hasher for Writes {
        fn finish(&self) -> u64 {
            0
        }
        fn write(&mut self, bytes: &[u8]) {
            self.count += 1;
            self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
    }

    fn sip_hash(value: &impl Hash) -> u64 {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut state);
        state.finish()
    }

    #[test]
    fn unmetered_key_hashes_the_full_signed_tile_in_one_write() {
        let tile = WorldTile {
            x: i32::MIN,
            z: i32::MAX,
            level: -1,
        };
        let key = Unmetered.start(tile);
        let mut writes = Writes::default();
        key.hash(&mut writes);
        assert_eq!((writes.count, writes.len), (1, 12));
        let mut original = Writes::default();
        tile.hash(&mut original);
        assert_eq!(writes.bytes, original.bytes);
        assert_eq!(sip_hash(&key), sip_hash(&tile));
        assert_eq!(
            std::mem::size_of_val(&key),
            std::mem::size_of::<WorldTile>()
        );
    }

    #[test]
    fn metered_key_hashes_the_tile_and_balance_in_one_write() {
        let key = ResourceKey {
            tile: WorldTile {
                x: -1,
                z: 0,
                level: i32::MIN,
            },
            balance: u32::MAX,
        };
        let mut writes = Writes::default();
        key.hash(&mut writes);
        assert_eq!((writes.count, writes.len), (1, 16));
        let mut original = Writes::default();
        (key.tile, key.balance).hash(&mut original);
        assert_eq!(writes.bytes, original.bytes);
        assert_eq!(sip_hash(&key), sip_hash(&(key.tile, key.balance)));
        assert_eq!(std::mem::size_of::<ResourceKey>(), 16);
    }
}
