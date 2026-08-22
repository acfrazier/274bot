//! Generation-stamped snapshot families. `GameSnapshot` owns per-family
//! views rebuilt only when that family's gen moved; reads borrow the last
//! rebuild instead of deep-copying the world on every read.

use client::client::{Client, ClientGens, ClientNpc};

/// A family of world state, mirroring the `ClientGens` counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Npc,
    Player,
    Inv,
    Varp,
    Stat,
    Chat,
    Scene,
}

/// Owned view of one live NPC slot, keyed by its slot index in
/// `Client.npc`. The reader's copy is independent of later in-place walk
/// mutations, so identity (the slot index) is stable across rebuilds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcView {
    pub index: usize,
    pub r#type: Option<usize>,
    pub x: i32,
    pub z: i32,
    pub yaw: i32,
    pub size: i32,
    pub health: i32,
    pub total_health: i32,
    pub face_entity: i32,
}

impl NpcView {
    fn from_slot(index: usize, npc: &ClientNpc) -> Self {
        let entity = &npc.entity;
        NpcView {
            index,
            r#type: npc.r#type,
            x: entity.x,
            z: entity.z,
            yaw: entity.yaw,
            size: entity.size,
            health: entity.health,
            total_health: entity.total_health,
            face_entity: entity.face_entity,
        }
    }
}

/// Generation-stamped read model. `rebuild_family` copies only the family
/// whose gen moved; `npcs()` returns the last rebuild without allocating.
#[derive(Default)]
pub struct GameSnapshot {
    /// World generations the snapshot has been rebuilt up to.
    gens: ClientGens,
    npc: Vec<NpcView>,
    runenergy: i32,
}

impl GameSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// The generation counters this snapshot reflects.
    pub fn gens(&self) -> ClientGens {
        self.gens
    }

    /// Rebuild `family` from `client` iff its gen moved since the last
    /// rebuild of that family. Returns true iff the gen moved. Only `Npc`
    /// has a view cache today; the other families track their counter so a
    /// later view can detect movement.
    pub fn rebuild_family(&mut self, client: &Client, family: Family) -> bool {
        match family {
            Family::Npc => self.rebuild_npcs(client),
            Family::Player => track(client.gens.player, &mut self.gens.player),
            Family::Inv => track(client.gens.inv, &mut self.gens.inv),
            Family::Varp => track(client.gens.varp, &mut self.gens.varp),
            Family::Stat => self.rebuild_stat(client),
            Family::Chat => track(client.gens.chat, &mut self.gens.chat),
            Family::Scene => track(client.gens.scene, &mut self.gens.scene),
        }
    }

    /// NPC views from the last npc rebuild, in `npc_ids` order (not sorted
    /// slots).
    pub fn npcs(&self) -> &[NpcView] {
        &self.npc
    }

    /// Last rebuilt run energy (stat family). `0` until a stat rebuild.
    pub fn runenergy(&self) -> i32 {
        self.runenergy
    }

    fn rebuild_stat(&mut self, client: &Client) -> bool {
        if !track(client.gens.stat, &mut self.gens.stat) {
            return false;
        }
        self.runenergy = client.runenergy;
        true
    }

    fn rebuild_npcs(&mut self, client: &Client) -> bool {
        if client.gens.npc == self.gens.npc {
            return false;
        }
        self.gens.npc = client.gens.npc;
        self.npc.clear();
        self.npc.reserve(client.npc_count as usize);
        for i in 0..client.npc_count as usize {
            let index = client.npc_ids[i] as usize;
            if let Some(npc) = client.npc.get(index).and_then(|n| n.as_ref()) {
                self.npc.push(NpcView::from_slot(index, npc));
            }
        }
        true
    }
}

fn track(world: u64, tracked: &mut u64) -> bool {
    if *tracked == world {
        return false;
    }
    *tracked = world;
    true
}
