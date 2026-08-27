//! Generation-stamped snapshot families. `GameSnapshot` owns per-family
//! views rebuilt only when that family's gen moved; reads borrow the last
//! rebuild instead of deep-copying the world on every read.

use client::client::{Client, ClientGens, ClientNpc};
use client::config::if_type::ComponentType;
use serde::Serialize;

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
    Iface,
    Camera,
    MapFlag,
    World,
}

/// Owned view of one live NPC slot, keyed by its slot index in
/// `Client.npc`. The reader's copy is independent of later in-place walk
/// mutations, so identity (the slot index) is stable across rebuilds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
/// Serializes to the whole-window shot sidecar JSON (the terminal state).
#[derive(Default, Serialize)]
pub struct GameSnapshot {
    /// World generations the snapshot has been rebuilt up to.
    #[serde(skip)]
    gens: ClientGens,
    npc: Vec<NpcView>,
    runenergy: i32,
    /// The scene origin (`map_build_base_x/z`); `None` before a world
    /// build. The mainland-seed gate reads it (the tutorial island build
    /// origin stays below 3000).
    base: Option<(i32, i32)>,
    /// The local player's world tile `(x, z, level)`; `None` before the
    /// first `PLAYER_INFO`. Tile level is not decoded on the body yet
    /// (gaps.md), so this is always level 0.
    tile: Option<(i32, i32, i32)>,
    /// Inventory `(obj id, count)` from the TYPE_INV iface, rebuilt when
    /// the inv gen moves (the server's `UPDATE_INV_FULL` fills it each
    /// frame). Empty before the inv iface loads.
    inv: Vec<(i32, i32)>,
    /// The most recent chat line (`chat_text[0]` is the ring head).
    chat: Option<String>,
    ingame: bool,
    scene_state: i32,
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
            Family::Player => self.rebuild_player(client),
            Family::Inv => self.rebuild_inv(client),
            Family::Varp => track(client.gens.varp, &mut self.gens.varp),
            Family::Stat => self.rebuild_stat(client),
            Family::Chat => self.rebuild_chat(client),
            Family::Scene => self.rebuild_scene(client),
            Family::Iface => track(client.gens.iface, &mut self.gens.iface),
            Family::Camera => track(client.gens.camera, &mut self.gens.camera),
            Family::MapFlag => track(client.gens.map_flag, &mut self.gens.map_flag),
            Family::World => track(client.gens.world, &mut self.gens.world),
        }
    }

    /// Rebuild every family whose gen moved (the harness "one snapshot
    /// per tick" read). Returns true iff any family gen moved.
    pub fn rebuild(&mut self, client: &Client) -> bool {
        let mut dirty = false;
        dirty |= self.rebuild_family(client, Family::Npc);
        dirty |= self.rebuild_family(client, Family::Player);
        dirty |= self.rebuild_family(client, Family::Inv);
        dirty |= self.rebuild_family(client, Family::Varp);
        dirty |= self.rebuild_family(client, Family::Stat);
        dirty |= self.rebuild_family(client, Family::Chat);
        dirty |= self.rebuild_family(client, Family::Scene);
        dirty |= self.rebuild_family(client, Family::Iface);
        dirty |= self.rebuild_family(client, Family::Camera);
        dirty |= self.rebuild_family(client, Family::MapFlag);
        dirty |= self.rebuild_family(client, Family::World);
        dirty
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

    /// The world build origin `(x, z)` from the last player-family
    /// rebuild; `None` before any world built.
    pub fn base(&self) -> Option<(i32, i32)> {
        self.base
    }

    /// The local player's world tile `(x, z, level)` from the last
    /// player-family rebuild.
    pub fn tile(&self) -> Option<(i32, i32, i32)> {
        self.tile
    }

    /// The inventory view `(obj id, count)` from the last inv rebuild.
    pub fn inv(&self) -> &[(i32, i32)] {
        &self.inv
    }

    /// The stacked count of `id` across inventory slots (0 when absent).
    pub fn inv_count(&self, id: i32) -> i32 {
        self.inv
            .iter()
            .filter(|(oid, _)| *oid == id)
            .map(|(_, n)| *n)
            .sum()
    }

    /// The most recent chat line from the last chat rebuild.
    pub fn chat(&self) -> Option<&str> {
        self.chat.as_deref()
    }

    /// `Client.ingame` from the last scene rebuild.
    pub fn ingame(&self) -> bool {
        self.ingame
    }

    /// `Client.scene_state` from the last scene rebuild.
    pub fn scene_state(&self) -> i32 {
        self.scene_state
    }

    fn rebuild_stat(&mut self, client: &Client) -> bool {
        if !track(client.gens.stat, &mut self.gens.stat) {
            return false;
        }
        self.runenergy = client.runenergy;
        true
    }

    /// Player-family rebuild: the scene origin and the local player's
    /// world tile (base + route head). `REBUILD_NORMAL` bumps every gen,
    /// so a new world origin re-arms this too.
    fn rebuild_player(&mut self, client: &Client) -> bool {
        if !track(client.gens.player, &mut self.gens.player) {
            return false;
        }
        let base = (client.map_build_base_x, client.map_build_base_z);
        self.base = Some(base);
        self.tile = client.local_player.as_ref().map(|lp| {
            (
                base.0 + lp.route_x[0],
                base.1 + lp.route_z[0],
                0, // tile level is not decoded on the body yet (gaps.md)
            )
        });
        true
    }

    /// Inv-family rebuild: zip the TYPE_INV iface's obj ids/counts (the
    /// same view `host-play` hands a running script).
    fn rebuild_inv(&mut self, client: &Client) -> bool {
        if !track(client.gens.inv, &mut self.gens.inv) {
            return false;
        }
        self.inv.clear();
        if let Some(inv) = client
            .ifaces
            .iter()
            .flatten()
            .find(|f| f.r#type == ComponentType::TYPE_INV)
        {
            if let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) {
                self.inv = ids.iter().zip(counts).map(|(id, n)| (*id, *n)).collect();
            }
        }
        true
    }

    /// Chat-family rebuild: the ring head (`chat_text[0]`) is the most
    /// recent message.
    fn rebuild_chat(&mut self, client: &Client) -> bool {
        if !track(client.gens.chat, &mut self.gens.chat) {
            return false;
        }
        let latest = client.chat_text[0].clone();
        self.chat = (!latest.is_empty()).then_some(latest);
        true
    }

    /// Scene-family rebuild: `ingame` + `scene_state`. These flip locally
    /// (`check_scene` sets `scene_state = 2` on the SIM loop with no gen
    /// bump), so always copy the cheap fields — a gen-gated copy would
    /// pin the snapshot in a stale "loading" state. The return value
    /// still tracks the gen for the harness's dirty/tick semantics.
    fn rebuild_scene(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.scene, &mut self.gens.scene);
        self.ingame = client.ingame;
        self.scene_state = client.scene_state;
        moved
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
