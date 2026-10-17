//! Resource labels are shared by walk nodes and change only at paid hops.
//! Generous inventories keep the original tile-only search after a conservative
//! proof that no simple path can exhaust a stack.
//! World tiles hash their scalar identity in one write. Metered labels include
//! the balance in the same write, retaining the original scalar bytes.
use super::*;
use std::hash::{Hash, Hasher};

pub(super) const RESOURCE_LIMIT: usize = 64;
pub(super) const BALANCE_LIMIT: usize = 4096;

pub(super) trait SearchKey: Copy + Eq + Hash {
    fn tile(self) -> WorldTile;
    fn at(self, tile: WorldTile) -> Self;
    fn tie(self) -> u32;
    fn from_parts(tile: WorldTile, tie: u32) -> Self;
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
    fn from_parts(tile: WorldTile, _: u32) -> Self {
        tile
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ResourceKey {
    tile: WorldTile,
    balance: u32,
}

impl Hash for ResourceKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut bytes = [0; 16];
        bytes[..4].copy_from_slice(&self.tile.x.to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.tile.z.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.tile.level.to_ne_bytes());
        bytes[12..].copy_from_slice(&self.balance.to_ne_bytes());
        state.write(&bytes);
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
    fn from_parts(tile: WorldTile, balance: u32) -> Self {
        Self { tile, balance }
    }
}
const PACKED_ID_BITS: u32 = 22;
const PACKED_ID_MASK: u32 = (1 << PACKED_ID_BITS) - 1;
pub(super) const PACKED_ROOT_ID: u32 = PACKED_ID_MASK;
const PACKED_STATUS_SHIFT: u32 = PACKED_ID_BITS;
const PACKED_OPEN: u32 = 1;
const PACKED_SETTLED: u32 = 2;
const PACKED_TOMBSTONE: u32 = 3;
const PACKED_BALANCE_MASK: u16 = 0x3fff;
const PACKED_ACTION_SHIFT: u32 = 14;
// Primitive fields let Vec use its zeroed-allocation specialization.
// The fields are tile coordinates, cost bits, balance/action, edge ID and state.
pub(super) type PackedSearchSlot = ([i32; 3], [u8; 8], u16, [u8; 3], [u8; 3]);
const _: () = assert!(
    std::mem::size_of::<PackedSearchSlot>() == 28 && std::mem::align_of::<PackedSearchSlot>() == 4
);

pub(super) trait PackedSearchSlotExt: Copy {
    fn empty() -> Self;
    fn cost(self) -> f64;
    fn set_cost(&mut self, cost: f64);
    fn balance(self) -> u32;
    fn action(self) -> u8;
    fn action_index(self) -> u32;
    fn tile(self) -> WorldTile;
    fn set_key(&mut self, tile: WorldTile, balance: u32, action: u8, action_index: u32, cost: f64);
    fn set_action(&mut self, action: u8, action_index: u32);
    fn state(self) -> u32;
    fn state_payload(self) -> u32;
    fn set_open(&mut self, position: usize);
    fn set_settled(&mut self, parent: u32);
    fn set_tombstone(&mut self);
    fn heap_position(self) -> usize;
    fn parent(self) -> u32;
}

impl PackedSearchSlotExt for PackedSearchSlot {
    fn empty() -> Self {
        ([0; 3], [0; 8], 0, [0; 3], [0; 3])
    }

    fn cost(self) -> f64 {
        f64::from_ne_bytes(self.1)
    }

    fn set_cost(&mut self, cost: f64) {
        self.1 = cost.to_ne_bytes();
    }

    fn balance(self) -> u32 {
        u32::from(self.2 & PACKED_BALANCE_MASK)
    }

    fn action(self) -> u8 {
        (self.2 >> PACKED_ACTION_SHIFT) as u8
    }

    fn action_index(self) -> u32 {
        read_u24(self.3)
    }

    fn tile(self) -> WorldTile {
        WorldTile {
            x: self.0[0],
            z: self.0[1],
            level: self.0[2],
        }
    }

    fn set_key(&mut self, tile: WorldTile, balance: u32, action: u8, action_index: u32, cost: f64) {
        self.0 = [tile.x, tile.z, tile.level];
        self.2 = balance as u16 & PACKED_BALANCE_MASK;
        self.set_action(action, action_index);
        self.set_cost(cost);
    }

    fn set_action(&mut self, action: u8, action_index: u32) {
        self.2 = (self.2 & PACKED_BALANCE_MASK) | (u16::from(action) << PACKED_ACTION_SHIFT);
        self.3 = write_u24(action_index);
    }

    fn state(self) -> u32 {
        read_u24(self.4) >> PACKED_STATUS_SHIFT
    }

    fn state_payload(self) -> u32 {
        read_u24(self.4) & PACKED_ID_MASK
    }

    fn set_open(&mut self, position: usize) {
        self.4 = write_u24(
            (PACKED_OPEN << PACKED_STATUS_SHIFT)
                | u32::try_from(position).expect(
                    "table capacity is at most PACKED_ID_MASK, so a heap position fits in 22 bits",
                ),
        );
    }

    fn set_settled(&mut self, parent: u32) {
        self.4 = write_u24((PACKED_SETTLED << PACKED_STATUS_SHIFT) | (parent & PACKED_ID_MASK));
    }

    fn set_tombstone(&mut self) {
        self.4 = write_u24(PACKED_TOMBSTONE << PACKED_STATUS_SHIFT);
    }

    fn heap_position(self) -> usize {
        self.state_payload() as usize
    }

    fn parent(self) -> u32 {
        self.state_payload()
    }
}

fn read_u24(bytes: [u8; 3]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn write_u24(value: u32) -> [u8; 3] {
    [value as u8, (value >> 8) as u8, (value >> 16) as u8]
}

fn read_link(bytes: [u8; 3]) -> u32 {
    read_u24(bytes).wrapping_sub(1) & PACKED_ID_MASK
}

fn write_link(id: u32) -> [u8; 3] {
    write_u24((id + 1) & PACKED_ID_MASK)
}

pub(super) enum PackedLookup {
    Found(u32),
    Vacant(u32),
}

pub(super) struct PackedSearchTable {
    slots: Vec<PackedSearchSlot>,
    // Bucket heads and slot links share one allocation. Chaining keeps
    // misses bounded by their bucket, even when every record slot is used.
    links: Vec<[u8; 3]>,
    buckets: usize,
    next_unused: usize,
    free_head: u32,
}

impl PackedSearchTable {
    pub(super) fn empty() -> Self {
        Self {
            slots: Vec::new(),
            links: Vec::new(),
            buckets: 0,
            next_unused: 0,
            free_head: PACKED_ROOT_ID,
        }
    }

    pub(super) fn try_new(settle_budget: usize) -> Option<Self> {
        Self::try_new_capacity(Self::capacity_for(settle_budget)?)
    }

    pub(super) fn try_new_capacity(capacity: usize) -> Option<Self> {
        if capacity == 0 {
            return Some(Self::empty());
        }
        if capacity > PACKED_ID_MASK as usize {
            return None;
        }
        let buckets = Self::bucket_count_for(capacity)?;
        let link_count = buckets.checked_add(capacity)?;
        std::alloc::Layout::array::<PackedSearchSlot>(capacity).ok()?;
        std::alloc::Layout::array::<[u8; 3]>(link_count).ok()?;
        let slots = vec![PackedSearchSlot::empty(); capacity];
        let links = vec![[0; 3]; link_count];
        Some(Self {
            slots,
            links,
            buckets,
            next_unused: 0,
            free_head: PACKED_ROOT_ID,
        })
    }

    pub(super) fn capacity_for(settle_budget: usize) -> Option<usize> {
        let capacity = settle_budget.checked_add(2)?;
        if capacity > PACKED_ID_MASK as usize {
            return None;
        }
        Some(capacity)
    }

    pub(super) fn capacity(&self) -> usize {
        self.slots.len()
    }

    fn bucket_count_for(capacity: usize) -> Option<usize> {
        if capacity == 0 {
            Some(0)
        } else {
            capacity.div_ceil(16).checked_next_power_of_two()
        }
    }

    fn next(&self, id: u32) -> u32 {
        read_link(self.links[self.buckets + id as usize])
    }

    fn set_next(&mut self, id: u32, next: u32) {
        self.links[self.buckets + id as usize] = write_link(next);
    }

    pub(super) fn vacant_id(&self) -> Option<u32> {
        if self.free_head != PACKED_ROOT_ID {
            Some(self.free_head)
        } else if self.next_unused < self.slots.len() {
            u32::try_from(self.next_unused).ok()
        } else {
            None
        }
    }

    pub(super) fn try_grow(
        &mut self,
        heap: &mut PackedMinMaxHeap,
        maximum_capacity: usize,
    ) -> Option<bool> {
        let old_capacity = self.slots.len();
        if old_capacity >= maximum_capacity {
            return Some(false);
        }
        if maximum_capacity > PACKED_ID_MASK as usize {
            return None;
        }
        let new_capacity = old_capacity.checked_mul(2)?.min(maximum_capacity);
        if new_capacity <= old_capacity {
            return Some(false);
        }
        let buckets = Self::bucket_count_for(new_capacity)?;
        let link_count = buckets.checked_add(new_capacity)?;
        let mut links = Vec::new();
        links.try_reserve_exact(link_count).ok()?;
        links.resize(link_count, [0; 3]);
        for (id, slot) in self.slots[..self.next_unused].iter().enumerate() {
            if matches!(slot.state(), PACKED_OPEN | PACKED_SETTLED) {
                let bucket = packed_hash_index(slot.tile(), slot.balance(), buckets);
                links[buckets + id] = links[bucket];
                links[bucket] = write_link(id as u32);
            } else {
                links[buckets + id] = self.links[self.buckets + id];
            }
        }
        self.slots
            .try_reserve_exact(new_capacity - old_capacity)
            .ok()?;
        heap.entries
            .try_reserve_exact(new_capacity.saturating_sub(heap.entries.len()))
            .ok()?;
        self.slots.resize(new_capacity, PackedSearchSlot::empty());
        self.links = links;
        self.buckets = buckets;
        // Record ids never move. Settled and queued parents, including the
        // caller's currently expanding node, remain valid after growth.
        Some(true)
    }

    pub(super) fn slot(&self, id: u32) -> &PackedSearchSlot {
        &self.slots[id as usize]
    }

    pub(super) fn slot_mut(&mut self, id: u32) -> &mut PackedSearchSlot {
        &mut self.slots[id as usize]
    }

    pub(super) fn is_settled(&self, id: u32) -> bool {
        self.slots[id as usize].state() == PACKED_SETTLED
    }

    pub(super) async fn locate(&self, tile: WorldTile, balance: u32) -> Result<PackedLookup, ()> {
        if self.buckets == 0 {
            return Err(());
        }
        let bucket = packed_hash_index(tile, balance, self.buckets);
        let mut id = read_link(self.links[bucket]);
        while id != PACKED_ROOT_ID {
            let slot = &self.slots[id as usize];
            if slot.tile() == tile && slot.balance() == balance {
                return Ok(PackedLookup::Found(id));
            }
            id = self.next(id);
            // The first fixed key check belongs to the caller's expansion
            // or transport unit. Every additional collision yields.
            if id != PACKED_ROOT_ID {
                work_unit().await;
            }
        }
        self.vacant_id().map(PackedLookup::Vacant).ok_or(())
    }

    pub(super) fn locate_sync(&self, tile: WorldTile, balance: u32) -> Option<u32> {
        if self.buckets == 0 {
            return None;
        }
        let bucket = packed_hash_index(tile, balance, self.buckets);
        let mut id = read_link(self.links[bucket]);
        while id != PACKED_ROOT_ID {
            let slot = &self.slots[id as usize];
            if slot.tile() == tile && slot.balance() == balance {
                return Some(id);
            }
            id = self.next(id);
        }
        None
    }

    pub(super) fn insert(
        &mut self,
        id: u32,
        tile: WorldTile,
        balance: u32,
        action: u8,
        action_index: u32,
        cost: f64,
    ) {
        if id == self.free_head {
            self.free_head = self.next(id);
        } else {
            debug_assert_eq!(id as usize, self.next_unused);
            self.next_unused += 1;
        }
        let bucket = packed_hash_index(tile, balance, self.buckets);
        self.set_next(id, read_link(self.links[bucket]));
        self.links[bucket] = write_link(id);
        let slot = &mut self.slots[id as usize];
        slot.set_key(tile, balance, action, action_index, cost);
        slot.set_open(0);
    }

    pub(super) fn update(&mut self, id: u32, action: u8, action_index: u32, cost: f64) {
        let slot = &mut self.slots[id as usize];
        slot.set_action(action, action_index);
        slot.set_cost(cost);
    }

    pub(super) fn settle(&mut self, id: u32, parent: u32) {
        self.slots[id as usize].set_settled(parent);
    }

    pub(super) async fn tombstone(&mut self, id: u32) {
        let slot = self.slots[id as usize];
        let bucket = packed_hash_index(slot.tile(), slot.balance(), self.buckets);
        let mut current = read_link(self.links[bucket]);
        let mut previous = PACKED_ROOT_ID;
        while current != id {
            work_unit().await;
            assert_ne!(current, PACKED_ROOT_ID, "removed packed key is indexed");
            previous = current;
            current = self.next(current);
        }
        let next = self.next(id);
        if previous == PACKED_ROOT_ID {
            self.links[bucket] = write_link(next);
        } else {
            self.set_next(previous, next);
        }
        self.slots[id as usize].set_tombstone();
        self.set_next(id, self.free_head);
        self.free_head = id;
    }

    pub(super) fn compare_candidate(
        &self,
        cost: f64,
        tile: WorldTile,
        balance: u32,
        other: u32,
    ) -> Ordering {
        let other = self.slot(other);
        cost.total_cmp(&other.cost())
            .then_with(|| other.tile().x.cmp(&tile.x))
            .then_with(|| other.tile().z.cmp(&tile.z))
            .then_with(|| other.tile().level.cmp(&tile.level))
            .then_with(|| other.balance().cmp(&balance))
    }
}

fn packed_hash_index(tile: WorldTile, balance: u32, capacity: usize) -> usize {
    let mut hash = (u64::from(tile.x as u32) << 32) | u64::from(tile.z as u32);
    hash ^= u64::from(tile.level as u32).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    hash ^= u64::from(balance).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;
    let hash32 = (hash ^ (hash >> 32)) as u32;
    ((u64::from(hash32) * capacity as u64) >> 32) as usize
}

pub(super) fn packed_predecessor_storage_bound(settle_budget: usize) -> Option<usize> {
    let capacity = settle_budget.checked_add(2)?;
    if capacity > PACKED_ID_MASK as usize {
        return None;
    }
    let links = capacity.checked_add(PackedSearchTable::bucket_count_for(capacity)?)?;
    capacity
        .checked_mul(std::mem::size_of::<PackedSearchSlot>())?
        .checked_add(links.checked_mul(std::mem::size_of::<[u8; 3]>())?)
}

pub(super) fn packed_search_storage_bound(settle_budget: usize) -> Option<usize> {
    packed_predecessor_storage_bound(settle_budget)?.checked_add(
        settle_budget
            .checked_add(2)?
            .checked_mul(std::mem::size_of::<PackedHeapEntry>())?,
    )
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PackedHeapEntry {
    node: [u8; 3],
    parent: [u8; 3],
}

impl PackedHeapEntry {
    fn new(node: u32, parent: u32) -> Self {
        Self {
            node: write_u24(node),
            parent: write_u24(parent),
        }
    }

    fn node(self) -> u32 {
        read_u24(self.node)
    }

    fn parent(self) -> u32 {
        read_u24(self.parent)
    }
}

pub(super) struct PackedMinMaxHeap {
    entries: Vec<PackedHeapEntry>,
    // Ordinary searches need only the minimum. Build the min-max ordering
    // in place only when bounded-frontier admission first needs eviction.
    min_max: bool,
}

impl PackedMinMaxHeap {
    pub(super) fn try_new(capacity: usize) -> Option<Self> {
        let mut entries = Vec::new();
        entries.try_reserve_exact(capacity).ok()?;
        Some(Self {
            entries,
            min_max: false,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    // Each sift has at most 22 levels. Ordinary heap work belongs to the
    // caller's bounded expansion or transport unit.
    pub(super) fn push(&mut self, table: &mut PackedSearchTable, node: u32, parent: u32) {
        let position = self.entries.len();
        self.entries.push(PackedHeapEntry::new(node, parent));
        table.slot_mut(node).set_open(position);
        if self.min_max {
            self.bubble_up(position, table);
        } else {
            self.bubble_up_binary(position, table);
        }
    }

    pub(super) fn pop_min(&mut self, table: &mut PackedSearchTable) -> Option<(u32, u32)> {
        let first = *self.entries.first()?;
        let last = self.entries.pop().expect("nonempty packed heap");
        if !self.entries.is_empty() {
            self.entries[0] = last;
            table.slot_mut(last.node()).set_open(0);
            if self.min_max {
                self.trickle_down_min(0, table);
            } else {
                self.trickle_down_binary(0, table);
            }
        }
        Some((first.node(), first.parent()))
    }

    pub(super) async fn worst_id(&mut self, table: &mut PackedSearchTable) -> Option<u32> {
        self.enable_min_max(table).await;
        let index = self.maximum_index(table)?;
        Some(self.entries[index].node())
    }

    pub(super) async fn remove_worst(&mut self, table: &mut PackedSearchTable) -> Option<u32> {
        self.enable_min_max(table).await;
        let index = self.maximum_index(table)?;
        work_unit().await;
        let removed = self.entries[index].node();
        let last = self.entries.pop().expect("nonempty packed heap");
        if index < self.entries.len() {
            self.entries[index] = last;
            table.slot_mut(last.node()).set_open(index);
            self.repair_at(index, table);
        }
        Some(removed)
    }

    pub(super) fn decrease_key(&mut self, table: &mut PackedSearchTable, node: u32, parent: u32) {
        let position = table.slot(node).heap_position();
        self.entries[position].parent = write_u24(parent);
        if !self.min_max {
            self.bubble_up_binary(position, table);
            return;
        }
        if is_min_level(position) {
            self.bubble_up_min(position, table);
            return;
        }
        let parent_position = (position - 1) / 2;
        if self
            .compare_positions(position, parent_position, table)
            .is_lt()
        {
            self.swap(position, parent_position, table);
            self.bubble_up_min(parent_position, table);
            self.trickle_down_max(position, table);
        } else {
            self.trickle_down_max(position, table);
        }
    }

    fn bubble_up_binary(&mut self, mut position: usize, table: &mut PackedSearchTable) {
        let entry = self.entries[position];
        let slot = *table.slot(entry.node());
        while position != 0 {
            let parent = (position - 1) / 2;
            let parent_entry = self.entries[parent];
            if !table
                .compare_candidate(
                    slot.cost(),
                    slot.tile(),
                    slot.balance(),
                    parent_entry.node(),
                )
                .is_lt()
            {
                break;
            }
            self.entries[position] = parent_entry;
            table.slot_mut(parent_entry.node()).set_open(position);
            position = parent;
        }
        self.entries[position] = entry;
        table.slot_mut(entry.node()).set_open(position);
    }

    fn trickle_down_binary(&mut self, mut position: usize, table: &mut PackedSearchTable) {
        let entry = self.entries[position];
        let slot = *table.slot(entry.node());
        while position * 2 + 1 < self.entries.len() {
            let left = position * 2 + 1;
            let right = left + 1;
            let child = if right < self.entries.len()
                && self.compare_positions(right, left, table).is_lt()
            {
                right
            } else {
                left
            };
            let child_entry = self.entries[child];
            if !table
                .compare_candidate(slot.cost(), slot.tile(), slot.balance(), child_entry.node())
                .is_gt()
            {
                break;
            }
            self.entries[position] = child_entry;
            table.slot_mut(child_entry.node()).set_open(position);
            position = child;
        }
        self.entries[position] = entry;
        table.slot_mut(entry.node()).set_open(position);
    }

    async fn enable_min_max(&mut self, table: &mut PackedSearchTable) {
        if self.min_max {
            return;
        }
        for position in (0..self.entries.len() / 2).rev() {
            work_unit().await;
            if is_min_level(position) {
                self.trickle_down_min(position, table);
            } else {
                self.trickle_down_max(position, table);
            }
        }
        self.min_max = true;
    }

    fn bubble_up(&mut self, position: usize, table: &mut PackedSearchTable) {
        if position == 0 {
            return;
        }
        let parent = (position - 1) / 2;
        if is_min_level(position) {
            if self.compare_positions(position, parent, table).is_gt() {
                self.swap(position, parent, table);
                self.bubble_up_max(parent, table);
            } else {
                self.bubble_up_min(position, table);
            }
        } else if self.compare_positions(position, parent, table).is_lt() {
            self.swap(position, parent, table);
            self.bubble_up_min(parent, table);
        } else {
            self.bubble_up_max(position, table);
        }
    }

    fn bubble_up_min(&mut self, mut position: usize, table: &mut PackedSearchTable) {
        while position >= 3 {
            let grandparent = (position - 3) / 4;
            if self.compare_positions(position, grandparent, table).is_lt() {
                self.swap(position, grandparent, table);
                position = grandparent;
            } else {
                break;
            }
        }
    }

    fn bubble_up_max(&mut self, mut position: usize, table: &mut PackedSearchTable) {
        while position >= 3 {
            let grandparent = (position - 3) / 4;
            if self.compare_positions(position, grandparent, table).is_gt() {
                self.swap(position, grandparent, table);
                position = grandparent;
            } else {
                break;
            }
        }
    }

    fn trickle_down_min(&mut self, mut position: usize, table: &mut PackedSearchTable) {
        loop {
            let Some(candidate) = self.extreme_descendant(position, table, true) else {
                return;
            };
            if is_grandchild(position, candidate) {
                if self.compare_positions(candidate, position, table).is_lt() {
                    self.swap(candidate, position, table);
                    let parent = (candidate - 1) / 2;
                    if self.compare_positions(candidate, parent, table).is_gt() {
                        self.swap(candidate, parent, table);
                    }
                    position = candidate;
                } else {
                    return;
                }
            } else {
                if self.compare_positions(candidate, position, table).is_lt() {
                    self.swap(candidate, position, table);
                }
                return;
            }
        }
    }

    fn trickle_down_max(&mut self, mut position: usize, table: &mut PackedSearchTable) {
        loop {
            let Some(candidate) = self.extreme_descendant(position, table, false) else {
                return;
            };
            if is_grandchild(position, candidate) {
                if self.compare_positions(candidate, position, table).is_gt() {
                    self.swap(candidate, position, table);
                    let parent = (candidate - 1) / 2;
                    if self.compare_positions(candidate, parent, table).is_lt() {
                        self.swap(candidate, parent, table);
                    }
                    position = candidate;
                } else {
                    return;
                }
            } else {
                if self.compare_positions(candidate, position, table).is_gt() {
                    self.swap(candidate, position, table);
                }
                return;
            }
        }
    }

    fn repair_at(&mut self, position: usize, table: &mut PackedSearchTable) {
        if position == 0 {
            self.trickle_down_min(position, table);
            return;
        }
        let parent = (position - 1) / 2;
        if is_min_level(position) {
            if self.compare_positions(position, parent, table).is_gt() {
                self.swap(position, parent, table);
                self.bubble_up_max(parent, table);
                self.trickle_down_max(parent, table);
                self.trickle_down_min(position, table);
            } else if position >= 3
                && self
                    .compare_positions(position, (position - 3) / 4, table)
                    .is_lt()
            {
                let grandparent = (position - 3) / 4;
                self.swap(position, grandparent, table);
                self.bubble_up_min(grandparent, table);
                self.trickle_down_min(position, table);
            } else {
                self.trickle_down_min(position, table);
            }
        } else if self.compare_positions(position, parent, table).is_lt() {
            self.swap(position, parent, table);
            self.bubble_up_min(parent, table);
            self.trickle_down_min(parent, table);
            self.trickle_down_max(position, table);
        } else if position >= 3
            && self
                .compare_positions(position, (position - 3) / 4, table)
                .is_gt()
        {
            let grandparent = (position - 3) / 4;
            self.swap(position, grandparent, table);
            self.bubble_up_max(grandparent, table);
            self.trickle_down_max(position, table);
        } else {
            self.trickle_down_max(position, table);
        }
    }

    fn maximum_index(&self, table: &PackedSearchTable) -> Option<usize> {
        match self.entries.len() {
            0 => None,
            1 => Some(0),
            2 => Some(1),
            _ if self.compare_positions(1, 2, table).is_gt() => Some(1),
            _ => Some(2),
        }
    }

    fn extreme_descendant(
        &self,
        position: usize,
        table: &PackedSearchTable,
        minimum: bool,
    ) -> Option<usize> {
        let mut best = None;
        let first_child = position * 2 + 1;
        let second_child = first_child + 1;
        for candidate in [first_child, second_child] {
            if candidate < self.entries.len()
                && best.is_none_or(|best| {
                    let order = self.compare_positions(candidate, best, table);
                    if minimum {
                        order.is_lt()
                    } else {
                        order.is_gt()
                    }
                })
            {
                best = Some(candidate);
            }
        }
        let first_grandchild = position * 4 + 3;
        for candidate in first_grandchild..first_grandchild + 4 {
            if candidate < self.entries.len()
                && best.is_none_or(|best| {
                    let order = self.compare_positions(candidate, best, table);
                    if minimum {
                        order.is_lt()
                    } else {
                        order.is_gt()
                    }
                })
            {
                best = Some(candidate);
            }
        }
        best
    }

    fn compare_positions(&self, left: usize, right: usize, table: &PackedSearchTable) -> Ordering {
        let left = table.slot(self.entries[left].node());
        let right = table.slot(self.entries[right].node());
        left.cost()
            .total_cmp(&right.cost())
            .then_with(|| right.tile().x.cmp(&left.tile().x))
            .then_with(|| right.tile().z.cmp(&left.tile().z))
            .then_with(|| right.tile().level.cmp(&left.tile().level))
            .then_with(|| right.balance().cmp(&left.balance()))
    }

    fn swap(&mut self, left: usize, right: usize, table: &mut PackedSearchTable) {
        self.entries.swap(left, right);
        let left_id = self.entries[left].node();
        let right_id = self.entries[right].node();
        table.slot_mut(left_id).set_open(left);
        table.slot_mut(right_id).set_open(right);
    }
}

fn is_min_level(position: usize) -> bool {
    ((usize::BITS - (position + 1).leading_zeros() - 1) & 1) == 0
}

fn is_grandchild(parent: usize, candidate: usize) -> bool {
    candidate >= parent * 4 + 3
}

pub(super) trait Budget {
    type Key: SearchKey;
    const METERED: bool;
    fn start(&self, tile: WorldTile) -> Self::Key;
    fn allowed(&self, state: &WorldState, edge: &TransportEdge, relax: Relax) -> bool;
    async fn cross(
        &mut self,
        key: Self::Key,
        edge: &TransportEdge,
        state: &WorldState,
        relax: Relax,
    ) -> Result<Option<Self::Key>, ()>;
    fn finish(
        self,
        tree: PackedSearchTable,
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
    async fn cross(
        &mut self,
        key: WorldTile,
        _: &TransportEdge,
        _: &WorldState,
        _: Relax,
    ) -> Result<Option<WorldTile>, ()> {
        Ok(Some(key))
    }
    fn finish(self, tree: PackedSearchTable, _: HashMap<WorldTile, WorldTile>) -> Predecessors {
        Predecessors::Tiles(tree)
    }
}

pub(super) struct ResourceBudget {
    ids: Vec<i32>,
    balances: Vec<[i32; RESOURCE_LIMIT]>,
    intern: HashMap<[i32; RESOURCE_LIMIT], u32>,
}

async fn can_carry(edge: &TransportEdge, state: &WorldState, returned: &HashSet<i32>) -> bool {
    for &(id, count) in &edge.item_req {
        work_unit().await;
        if state.inv.get(&id).copied().unwrap_or(0) < count && !returned.contains(&id) {
            return false;
        }
    }
    for &(id, packed_count) in &edge.consumed_req {
        work_unit().await;
        let carried = state.inv.get(&id).copied().unwrap_or(0);
        if carried < edge.consumption_count(id, packed_count, carried) && !returned.contains(&id) {
            return false;
        }
    }
    true
}
impl ResourceBudget {
    pub(super) async fn new(
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
        let mut has_paid_hop = false;
        for edge in edges() {
            work_unit().await;
            if !edge.consumed_req.is_empty() && edge_allowed(state, edge, relax) {
                has_paid_hop = true;
                break;
            }
        }
        // No paid hop can be the first one: no returns can then become available.
        if !has_paid_hop {
            return Ok(None);
        }

        let mut carry_rows = 0usize;
        let mut return_rows = 0usize;
        for edge in edges() {
            work_unit().await;
            carry_rows = carry_rows
                .checked_add(edge.item_req.len())
                .and_then(|rows| rows.checked_add(edge.consumed_req.len()))
                .and_then(|rows| rows.checked_add(edge.item_returns.len()))
                .ok_or(())?;
            return_rows = return_rows.checked_add(edge.item_returns.len()).ok_or(())?;
        }
        let mut possible_returns = HashSet::new();
        possible_returns.try_reserve(return_rows).map_err(|_| ())?;
        loop {
            let mut changed = false;
            for edge in edges() {
                work_unit().await;
                if !fixed_allowed(state, edge, relax)
                    || !can_carry(edge, state, &possible_returns).await
                {
                    continue;
                }
                for &(id, _) in &edge.item_returns {
                    work_unit().await;
                    if possible_returns.insert(id) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let mut totals = HashMap::<i32, i64>::new();
        totals.try_reserve(carry_rows).map_err(|_| ())?;
        let mut held = HashMap::<i32, i32>::new();
        held.try_reserve(carry_rows).map_err(|_| ())?;
        let mut ids = Vec::new();
        ids.try_reserve_exact(carry_rows).map_err(|_| ())?;
        for (index, edge) in edges().enumerate() {
            work_unit().await;
            if !fixed_allowed(state, edge, relax)
                || !can_carry(edge, state, &possible_returns).await
            {
                continue;
            }
            let multiplicity = if edge.player_delta.is_some() && index < graph.edges.len() {
                let (min, max) = graph.takeoff_bounds(index);
                (i64::from(max.x) - i64::from(min.x) + 1)
                    * (i64::from(max.z) - i64::from(min.z) + 1)
            } else {
                1
            };
            for &(id, packed_count) in &edge.consumed_req {
                work_unit().await;
                let count = edge.consumption_count(id, packed_count, i32::MAX);
                let total = totals.entry(id).or_default();
                *total = total.saturating_add(i64::from(count).saturating_mul(multiplicity));
                ids.push(id);
            }
            for &(id, _) in &edge.item_returns {
                work_unit().await;
                ids.push(id);
            }
            for &(id, count) in &edge.item_req {
                work_unit().await;
                let peak = held.entry(id).or_default();
                *peak = (*peak).max(count);
            }
        }

        // Every shortest unconstrained path is simple (positive hop costs).
        // Each absolute landing is visited at most once; relative edges can
        // be used once per admissible takeoff. Ignoring credits gives an upper
        // bound, including a held gate after all spend.
        // A returned variant can unlock a held gate even with ample inputs.
        let mut enough = true;
        for (&id, &total) in &totals {
            work_unit().await;
            if i64::from(state.inv.get(&id).copied().unwrap_or(0))
                < total.saturating_add(i64::from(held.get(&id).copied().unwrap_or(0)))
            {
                enough = false;
                break;
            }
        }
        if enough {
            for (&id, &count) in &held {
                work_unit().await;
                if state.inv.get(&id).copied().unwrap_or(0) < count
                    && (totals.contains_key(&id) || possible_returns.contains(&id))
                {
                    enough = false;
                    break;
                }
            }
        }
        if enough {
            return Ok(None);
        }

        sort_unstable_by_cooperative(&mut ids, |left, right| left.cmp(right)).await;
        dedup_sorted_by_cooperative(&mut ids).await;
        if ids.len() > RESOURCE_LIMIT {
            return Err(());
        }
        let mut initial = [0; RESOURCE_LIMIT];
        for (index, id) in ids.iter().enumerate() {
            work_unit().await;
            initial[index] = state.inv.get(id).copied().unwrap_or(0).max(0);
        }
        let mut balances = Vec::new();
        balances.try_reserve_exact(1).map_err(|_| ())?;
        balances.push(initial);
        let mut intern = HashMap::new();
        intern.try_reserve(1).map_err(|_| ())?;
        intern.insert(initial, 0);
        Ok(Some(Self {
            ids,
            balances,
            intern,
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
    async fn cross(
        &mut self,
        key: ResourceKey,
        edge: &TransportEdge,
        state: &WorldState,
        _: Relax,
    ) -> Result<Option<ResourceKey>, ()> {
        let mut balance = self.balances[key.balance as usize];
        for &(id, count) in &edge.item_req {
            work_unit().await;
            if self.count(&balance, state, id) < count {
                return Ok(None);
            }
        }
        if edge.consumed_req.is_empty() && edge.item_returns.is_empty() {
            return Ok(Some(key));
        }
        for &(id, packed_count) in &edge.consumed_req {
            work_unit().await;
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
            work_unit().await;
            let index = self.ids.binary_search(&id).map_err(|_| ())?;
            balance[index] = balance[index].saturating_add(count);
        }
        let index = if let Some(&index) = self.intern.get(&balance) {
            index
        } else {
            if self.balances.len() == BALANCE_LIMIT {
                return Err(());
            }
            self.balances.try_reserve(1).map_err(|_| ())?;
            self.intern.try_reserve(1).map_err(|_| ())?;
            let index = u32::try_from(self.balances.len()).map_err(|_| ())?;
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
        tree: PackedSearchTable,
        reached: HashMap<WorldTile, ResourceKey>,
    ) -> Predecessors {
        Predecessors::Resources { tree, reached }
    }
}

pub(super) enum Predecessors {
    Tiles(PackedSearchTable),
    Resources {
        tree: PackedSearchTable,
        reached: HashMap<WorldTile, ResourceKey>,
    },
}

pub(super) enum BacktraceStep {
    Walk { tile: WorldTile },
    Transport { tile: WorldTile, index: usize },
    Teleport { tile: WorldTile, index: usize },
    EssenceReturn { tile: WorldTile, portal: usize },
}

pub(super) struct RouteBacktrace<'a> {
    tree: &'a PackedSearchTable,
    current: Option<u32>,
    tile: WorldTile,
}

impl RouteBacktrace<'_> {
    pub(super) fn current_tile(&self) -> WorldTile {
        self.tile
    }

    pub(super) fn next(&mut self) -> Option<BacktraceStep> {
        let current = self.current?;
        let slot = *self.tree.slot(current);
        let parent = slot.parent();
        if parent == PACKED_ROOT_ID {
            self.current = None;
            return None;
        }
        let tile = self.tile;
        self.tile = self.tree.slot(parent).tile();
        self.current = Some(parent);
        let index = slot.action_index() as usize;
        Some(match slot.action() {
            0 => BacktraceStep::Walk { tile },
            1 => BacktraceStep::Transport { tile, index },
            2 => BacktraceStep::Teleport { tile, index },
            _ => BacktraceStep::EssenceReturn {
                tile,
                portal: index,
            },
        })
    }
}

impl Predecessors {
    pub(super) fn empty() -> Self {
        Self::Tiles(PackedSearchTable::empty())
    }

    pub(super) fn capacity(&self) -> usize {
        match self {
            Self::Tiles(tree) => tree.capacity(),
            Self::Resources { tree, reached } => tree.capacity() + reached.capacity(),
        }
    }

    pub(super) fn backtrace(&self, to: WorldTile) -> Option<RouteBacktrace<'_>> {
        Some(match self {
            Self::Tiles(tree) => RouteBacktrace {
                tree,
                current: tree.locate_sync(to, 0),
                tile: to,
            },
            Self::Resources { tree, reached } => {
                let key = reached.get(&to)?;
                RouteBacktrace {
                    tree,
                    current: tree.locate_sync(key.tile, key.balance),
                    tile: to,
                }
            }
        })
    }

    pub(super) fn reconstruct(
        &self,
        to: WorldTile,
        graph: &TransportGraph,
        model: CostModel,
        essence: Option<&EssenceSession>,
    ) -> (Vec<Leg>, f64) {
        super::reconstruct_packed(self, to, graph, model, essence)
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

#[cfg(test)]
mod packed_search_tests {
    use super::*;

    fn tile(x: i32) -> WorldTile {
        WorldTile { x, z: -3, level: 0 }
    }

    #[test]
    fn zeroed_packed_storage_has_empty_records_and_null_links() {
        let table = PackedSearchTable::try_new(4).expect("zeroed table");
        assert!(table.links.iter().all(|link| *link == [0; 3]));
        for slot in &table.slots {
            assert_eq!(
                slot.tile(),
                WorldTile {
                    x: 0,
                    z: 0,
                    level: 0
                }
            );
            assert_eq!(*slot, PackedSearchSlot::empty());
        }
        assert_eq!(read_link([0; 3]), PACKED_ROOT_ID);
        for id in [0, 1, PACKED_ID_MASK - 1, PACKED_ROOT_ID] {
            assert_eq!(read_link(write_link(id)), id);
        }
        assert_eq!(
            PackedSearchTable::try_new_capacity(0)
                .expect("empty storage")
                .capacity(),
            0
        );
    }

    #[test]
    fn primitive_packed_slot_round_trips_all_scalar_fields() {
        let mut slot = PackedSearchSlot::empty();
        let tile = WorldTile {
            x: i32::MIN,
            z: i32::MAX,
            level: -1,
        };
        let cost = f64::from_bits(0x7ff8_0000_0000_0042);
        slot.set_key(tile, u32::from(PACKED_BALANCE_MASK), 3, 0x00ff_fffe, cost);
        assert_eq!(slot.tile(), tile);
        assert_eq!(slot.balance(), u32::from(PACKED_BALANCE_MASK));
        assert_eq!(slot.action(), 3);
        assert_eq!(slot.action_index(), 0x00ff_fffe);
        assert_eq!(slot.cost().to_bits(), cost.to_bits());
        slot.set_open((PACKED_ID_MASK - 1) as usize);
        assert_eq!(slot.state(), PACKED_OPEN);
        assert_eq!(slot.heap_position(), (PACKED_ID_MASK - 1) as usize);
        slot.set_settled(PACKED_ROOT_ID);
        assert_eq!(slot.state(), PACKED_SETTLED);
        assert_eq!(slot.parent(), PACKED_ROOT_ID);
    }

    #[test]
    fn packed_table_reuses_tombstones_and_finds_live_keys() {
        super::super::drive_to_completion(async {
            let mut table = PackedSearchTable::try_new(4).expect("small packed table");
            let first_tile = tile(7);
            let first_balance = 3;
            let first = match table.locate(first_tile, first_balance).await.unwrap() {
                PackedLookup::Vacant(id) => id,
                PackedLookup::Found(_) => panic!("empty table found a key"),
            };
            table.insert(first, first_tile, first_balance, 1, 17, 4.5);
            match table.locate(first_tile, first_balance).await.unwrap() {
                PackedLookup::Found(id) => assert_eq!(id, first),
                PackedLookup::Vacant(_) => panic!("live packed key was not found"),
            }

            let bucket = packed_hash_index(first_tile, first_balance, table.buckets);
            let live_balance = 4;
            let live_tile = (8..100)
                .map(tile)
                .find(|candidate| {
                    packed_hash_index(*candidate, live_balance, table.buckets) == bucket
                })
                .expect("a second key with the same probe sequence");
            let live = match table.locate(live_tile, live_balance).await.unwrap() {
                PackedLookup::Vacant(id) => id,
                PackedLookup::Found(_) => panic!("distinct key was already present"),
            };
            table.insert(live, live_tile, live_balance, 2, 23, 8.0);
            match table.locate(live_tile, live_balance).await.unwrap() {
                PackedLookup::Found(id) => assert_eq!(id, live),
                PackedLookup::Vacant(_) => panic!("second live packed key was not found"),
            }

            table.tombstone(first).await;
            let replacement_balance = 5;
            let replacement_tile = (8..100)
                .map(tile)
                .find(|candidate| {
                    packed_hash_index(*candidate, replacement_balance, table.buckets) == bucket
                })
                .expect("a replacement key with the same probe sequence");
            let reused = match table
                .locate(replacement_tile, replacement_balance)
                .await
                .unwrap()
            {
                PackedLookup::Vacant(id) => id,
                PackedLookup::Found(_) => panic!("tombstone probe found another key"),
            };
            assert_eq!(reused, first);
            table.insert(reused, replacement_tile, replacement_balance, 3, 29, 6.0);
            match table.locate(live_tile, live_balance).await.unwrap() {
                PackedLookup::Found(id) => assert_eq!(id, live),
                PackedLookup::Vacant(_) => panic!("live key after tombstone reuse was lost"),
            }
            match table
                .locate(replacement_tile, replacement_balance)
                .await
                .unwrap()
            {
                PackedLookup::Found(id) => assert_eq!(id, first),
                PackedLookup::Vacant(_) => panic!("replacement key was not found"),
            }
        });
    }

    #[test]
    fn packed_min_max_heap_orders_decrease_key_and_worst_removal() {
        super::super::drive_to_completion(async {
            let costs = [7.0, 1.0, 9.0, 3.0, 5.0, 2.0, 8.0, 4.0, 6.0];
            let mut table = PackedSearchTable::try_new(costs.len()).expect("packed table for test");
            let mut heap = PackedMinMaxHeap::try_new(costs.len()).expect("packed heap for test");
            for (id, &cost) in costs.iter().enumerate() {
                let id = id as u32;
                table.insert(id, tile(id as i32), 0, 0, 0, cost);
                heap.push(&mut table, id, PACKED_ROOT_ID);
            }
            let worst = heap.worst_id(&mut table).await.expect("nonempty heap");
            assert_eq!(table.slot(worst).cost(), 9.0);
            assert_eq!(heap.remove_worst(&mut table).await, Some(worst));
            table.tombstone(worst).await;

            table.update(1, 0, 0, 0.0);
            heap.decrease_key(&mut table, 1, PACKED_ROOT_ID);
            let mut popped_costs = Vec::new();
            while let Some((id, parent)) = heap.pop_min(&mut table) {
                assert_eq!(parent, PACKED_ROOT_ID);
                popped_costs.push(table.slot(id).cost());
            }
            assert_eq!(popped_costs, [0.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        });
    }

    #[test]
    fn packed_heap_preserves_total_order_before_and_after_min_max_conversion() {
        super::super::drive_to_completion(async {
            for conversion in [None, Some(1), Some(17), Some(511)] {
                let mut table = PackedSearchTable::try_new(2048).unwrap();
                let mut heap = PackedMinMaxHeap::try_new(2048).unwrap();
                for id in 0..2048u32 {
                    let cost = f64::from((id * 73) % 257);
                    table.insert(id, tile(id as i32), 0, 0, 0, cost);
                    heap.push(&mut table, id, PACKED_ROOT_ID);
                    if conversion == Some(id + 1) {
                        heap.enable_min_max(&mut table).await;
                    }
                }
                for id in (0..2048u32).step_by(11) {
                    table.update(id, 0, 0, -0.25);
                    heap.decrease_key(&mut table, id, PACKED_ROOT_ID);
                }
                let mut expected: Vec<_> = (0..2048u32).collect();
                expected.sort_by(|&left, &right| {
                    let left = table.slot(left);
                    let right = table.slot(right);
                    left.cost()
                        .total_cmp(&right.cost())
                        .then_with(|| right.tile().x.cmp(&left.tile().x))
                        .then_with(|| right.tile().z.cmp(&left.tile().z))
                        .then_with(|| right.tile().level.cmp(&left.tile().level))
                        .then_with(|| right.balance().cmp(&left.balance()))
                });
                if conversion.is_some() {
                    for _ in 0..10 {
                        let worst = expected.pop().unwrap();
                        assert_eq!(heap.worst_id(&mut table).await, Some(worst));
                        assert_eq!(heap.remove_worst(&mut table).await, Some(worst));
                    }
                } else {
                    assert!(!heap.min_max);
                }
                for expected in expected {
                    assert_eq!(heap.pop_min(&mut table), Some((expected, PACKED_ROOT_ID)));
                }
                assert_eq!(heap.pop_min(&mut table), None);
            }
        });
    }

    #[test]
    fn packed_growth_keeps_the_expanding_node_and_queued_parents_stable() {
        super::super::drive_to_completion(async {
            let mut table = PackedSearchTable::try_new_capacity(32).expect("small table");
            let mut heap = PackedMinMaxHeap::try_new(32).expect("small heap");
            let origin = (0..100)
                .map(tile)
                .find(|&candidate| {
                    packed_hash_index(candidate, 0, 2) != packed_hash_index(candidate, 0, 4)
                })
                .expect("a key that changes bucket on growth");
            let current = match table.locate(origin, 0).await.unwrap() {
                PackedLookup::Vacant(id) => id,
                PackedLookup::Found(_) => panic!("empty table"),
            };
            table.insert(current, origin, 0, 0, 0, 0.0);
            table.settle(current, PACKED_ROOT_ID);
            let child_tile = tile(2000);
            let child = match table.locate(child_tile, 0).await.unwrap() {
                PackedLookup::Vacant(id) => id,
                PackedLookup::Found(_) => panic!("distinct child key"),
            };
            table.insert(child, child_tile, 0, 1, 7, 1.0);
            heap.push(&mut table, child, current);
            let old_current = current;
            assert_eq!(table.try_grow(&mut heap, 64), Some(true));
            assert_eq!(current, old_current, "the local expanding id stays valid");
            assert_eq!(table.slot(current).tile(), origin);
            assert_eq!(table.slot(current).parent(), PACKED_ROOT_ID);
            let (child, parent) = heap.pop_min(&mut table).expect("queued child");
            assert_eq!(table.slot(child).tile(), child_tile);
            assert_eq!(parent, current);
            table.settle(child, parent);
            let predecessors = Predecessors::Tiles(table);
            let mut trace = predecessors.backtrace(child_tile).expect("child path");
            assert!(matches!(
                trace.next(),
                Some(BacktraceStep::Transport { index: 7, .. })
            ));
            assert_eq!(trace.current_tile(), origin);
            assert!(trace.next().is_none());
        });
    }

    #[test]
    fn packed_full_table_missing_keys_do_not_scan_the_entire_store() {
        let mut table = PackedSearchTable::try_new(4096).unwrap();
        super::super::drive_to_completion(async {
            for x in 0..table.capacity() {
                let key = tile(x as i32);
                let id = match table.locate(key, 0).await.unwrap() {
                    PackedLookup::Vacant(id) => id,
                    PackedLookup::Found(_) => panic!("distinct test key"),
                };
                table.insert(id, key, 0, 0, 0, 0.0);
            }
        });
        super::super::drive_to_completion(table.tombstone(0));
        let mut probes = 0usize;
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        for x in 10_000..10_064 {
            let mut future = std::pin::pin!(table.locate(tile(x), 0));
            loop {
                match future.as_mut().poll(&mut context) {
                    std::task::Poll::Pending => probes += 1,
                    std::task::Poll::Ready(result) => {
                        assert!(matches!(result, Ok(PackedLookup::Vacant(_))));
                        break;
                    }
                }
            }
        }
        assert!(
            probes < table.capacity(),
            "{probes} probes for 64 misses in a {}-slot store",
            table.capacity()
        );
    }

    #[test]
    fn packed_lookup_bundles_the_first_key_and_yields_between_further_collisions() {
        let mut table = PackedSearchTable::try_new_capacity(32).unwrap();
        let keys: Vec<_> = (0..1024)
            .map(tile)
            .filter(|&key| packed_hash_index(key, 0, table.buckets) == 0)
            .take(16)
            .collect();
        assert_eq!(keys.len(), 16);
        for (id, &key) in keys.iter().enumerate() {
            table.insert(u32::try_from(id).unwrap(), key, 0, 0, 0, 0.0);
        }
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        for (id, expected_pending) in [(15, 0), (0, 15)] {
            let mut pending = 0;
            let mut future = std::pin::pin!(table.locate(keys[id], 0));
            loop {
                match future.as_mut().poll(&mut context) {
                    std::task::Poll::Pending => pending += 1,
                    std::task::Poll::Ready(result) => {
                        assert!(matches!(
                            result,
                            Ok(PackedLookup::Found(found)) if found == u32::try_from(id).unwrap()
                        ));
                        break;
                    }
                }
            }
            assert_eq!(pending, expected_pending);
        }
    }

    #[test]
    fn packed_storage_bound_covers_full_table_and_heap_capacity() {
        let capacity = 6usize;
        assert_eq!(
            packed_search_storage_bound(4),
            Some(
                capacity * std::mem::size_of::<PackedSearchSlot>()
                    + capacity * std::mem::size_of::<PackedHeapEntry>()
                    + (capacity + 1) * std::mem::size_of::<[u8; 3]>()
            )
        );
        assert_eq!(
            packed_search_storage_bound(NODE_BUDGET),
            Some(148_786_506),
            "cooperative backing keeps the existing full-budget reservation"
        );
        assert!(packed_search_storage_bound(usize::MAX).is_none());
    }
}
