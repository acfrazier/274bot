//! Generation-stamped snapshot families. `GameSnapshot` owns per-family
//! views rebuilt only when that family's gen moved; reads borrow the last
//! rebuild instead of deep-copying the world on every read.

use client::client::{Client, ClientGens, ClientNpc, Skill};
use client::config::if_type::ComponentType;
use client::config::Cache;
use client::dash3d::client_entity::ClientEntity;
use serde::Serialize;

/// A world tile: absolute `x`/`z` plus the plane (`level`). The key type
/// loc/ground-item/player families are positioned by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct WorldTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// A tile relative to the scene origin (`level` is implicit in the world
/// build). The loc/ground-item families' scene coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalTile {
    pub lx: i32,
    pub lz: i32,
}

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

/// The kind of entity an actor is facing (`ActorTargetView::kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ActorKind {
    Npc,
    Player,
}

/// A resolved `face_entity`: the entity kind and slot index an actor is
/// facing, or `None` when not facing anyone. Slots are the client's own
/// (`npc`/`players` table indexes), decoded like `entity_face` in
/// `client.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActorTargetView {
    pub kind: ActorKind,
    pub index: usize,
}

/// The shared actor view every entity-family view embeds (the m8aq
/// `ActorSnapshot`): name/actions plus the entity's live pose state.
#[derive(Debug, Clone, Serialize)]
pub struct ActorView {
    pub name: Option<String>,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
    pub animation: i32,
    pub pose_animation: i32,
    pub orientation: i32,
    pub target_orientation: i32,
    pub overhead_text: Option<String>,
    pub spot_animation: i32,
    pub health: i32,
    pub total_health: i32,
    pub face_entity: i32,
    pub target: Option<ActorTargetView>,
    pub moving: bool,
    pub running: bool,
    pub in_combat: bool,
}

/// Owned view of one live NPC slot, keyed by its slot index in
/// `Client.npc`. The reader's copy is independent of later in-place walk
/// mutations, so identity (the slot index) is stable across rebuilds.
#[derive(Debug, Clone, Serialize)]
pub struct NpcView {
    pub index: usize,
    pub r#type: Option<usize>,
    pub name: Option<String>,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
    pub animation: i32,
    pub pose_animation: i32,
    pub orientation: i32,
    pub target_orientation: i32,
    pub overhead_text: Option<String>,
    pub spot_animation: i32,
    pub health: i32,
    pub total_health: i32,
    pub face_entity: i32,
    pub target: Option<ActorTargetView>,
    pub moving: bool,
    pub running: bool,
    pub in_combat: bool,
    pub level: i32,
    pub size: i32,
    /// Legacy position aliases (the pre-v2 `NpcView` surface; `query::npcs_at`
    /// and the old tests read them).
    pub x: i32,
    pub z: i32,
    pub yaw: i32,
}

impl NpcView {
    fn from_slot(
        index: usize,
        npc: &ClientNpc,
        level: i32,
        distance: i32,
        cache: &Cache,
    ) -> Self {
        let entity = &npc.entity;
        let (name, actions, npc_level, size) = match npc.r#type.and_then(|t| cache.npcs.get(t)) {
            Some(t) => (
                Some(t.name.clone()),
                t.op.clone(),
                t.vislevel.max(0),
                t.size,
            ),
            None => (None, Vec::new(), 0, entity.size),
        };
        NpcView {
            index,
            r#type: npc.r#type,
            name,
            actions,
            tile: WorldTile { x: entity.x, z: entity.z, level },
            distance,
            animation: entity.primary_anim,
            pose_animation: entity.secondary_anim,
            orientation: entity.yaw,
            target_orientation: entity.dst_yaw,
            overhead_text: entity.chat_message.clone(),
            spot_animation: entity.spotanim_id,
            health: entity.health,
            total_health: entity.total_health,
            face_entity: entity.face_entity,
            target: decode_target(entity.face_entity),
            moving: entity.route_length > 0,
            running: entity.primary_anim == entity.runanim,
            in_combat: entity.combat_cycle > 0,
            level: npc_level,
            size,
            x: entity.x,
            z: entity.z,
            yaw: entity.yaw,
        }
    }
}

/// A remote player, keyed by its `players` table slot index.
#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    pub index: usize,
    pub actor: ActorView,
    pub combat_level: i32,
    pub skill_level: i32,
}

/// The local player: a `PlayerView` at `self_slot` plus the run/weight
/// stats. `distance` is always 0.
#[derive(Debug, Clone, Serialize)]
pub struct LocalPlayerView {
    pub player: PlayerView,
    pub energy: i32,
    pub weight: i32,
}

/// One skill slot of the client's 25-entry table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatView {
    pub index: i32,
    pub name: String,
    pub effective: i32,
    pub base: i32,
    pub xp: i32,
    pub used: bool,
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
    player: Option<LocalPlayerView>,
    players: Vec<PlayerView>,
    stats: Vec<StatView>,
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
    /// rebuild of that family. Returns true iff the gen moved. The npc/
    /// player/stat families rebuild their view caches; the rest track
    /// their counter so a later view can detect movement.
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

    /// The local player view from the last player rebuild; `None` before
    /// the first `PLAYER_INFO` decodes one.
    pub fn local_player(&self) -> Option<&LocalPlayerView> {
        self.player.as_ref()
    }

    /// Remote player views from the last player rebuild, in `player_ids`
    /// order (the local player lives on `local_player()`).
    pub fn players(&self) -> &[PlayerView] {
        &self.players
    }

    /// All 25 skill slots from the last stat rebuild.
    pub fn stats(&self) -> &[StatView] {
        &self.stats
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
        self.stats = (0..Skill::count)
            .map(|i| StatView {
                index: i as i32,
                name: Skill::names[i].to_string(),
                effective: client.stat_effective_level[i],
                base: client.stat_base_level[i],
                xp: client.stat_xp[i],
                used: Skill::used[i],
            })
            .collect();
        true
    }

    /// Player-family rebuild: the scene origin, the local player's world
    /// tile (base + route head), the `LocalPlayerView`, and the remote
    /// `players` list. `REBUILD_NORMAL` bumps every gen, so a new world
    /// origin re-arms this too.
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
        let level = client.minusedlevel;
        self.player = client.local_player.as_ref().map(|lp| LocalPlayerView {
            player: PlayerView {
                index: client.self_slot.max(0) as usize,
                actor: actor_view(
                    &lp.entity,
                    level,
                    0, // the local player's distance to itself
                    lp.name.clone(),
                    client.player_op.to_vec(),
                ),
                combat_level: lp.combat_level,
                skill_level: lp.skill_level,
            },
            energy: client.runenergy,
            weight: client.runweight,
        });
        self.players.clear();
        self.players.reserve(client.player_count as usize);
        let local_tile = local_world_tile(client);
        for i in 0..client.player_count as usize {
            let index = client.player_ids[i] as usize;
            if let Some(player) = client.players.get(index).and_then(|p| p.as_ref()) {
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(player.x, player.z, lx, lz))
                    .unwrap_or(0);
                self.players.push(PlayerView {
                    index,
                    actor: actor_view(
                        &player.entity,
                        level,
                        distance,
                        player.name.clone(),
                        client.player_op.to_vec(),
                    ),
                    combat_level: player.combat_level,
                    skill_level: player.skill_level,
                });
            }
        }
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
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        for i in 0..client.npc_count as usize {
            let index = client.npc_ids[i] as usize;
            if let Some(npc) = client.npc.get(index).and_then(|n| n.as_ref()) {
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(npc.x, npc.z, lx, lz))
                    .unwrap_or(0);
                self.npc
                    .push(NpcView::from_slot(index, npc, level, distance, &client.cache));
            }
        }
        true
    }
}

/// The local player's world tile `(x, z)` from the live client (build
/// base + route head); `None` before the first `PLAYER_INFO`.
fn local_world_tile(client: &Client) -> Option<(i32, i32)> {
    client.local_player.as_ref().map(|lp| {
        (
            client.map_build_base_x + lp.route_x[0],
            client.map_build_base_z + lp.route_z[0],
        )
    })
}

fn chebyshev(ax: i32, az: i32, bx: i32, bz: i32) -> i32 {
    (ax - bx).abs().max((az - bz).abs())
}

/// Resolve `face_entity` with the client's own scheme (`entity_face` in
/// `client.rs`): slots below 32768 are NPC table indexes, at or above are
/// player slots offset by 32768. The player slot stays the raw server
/// slot (the client only rewrites `self_slot` → 2047 for its internal
/// turn lookup), so `targetingPlayer(self_slot)` matches the local player.
fn decode_target(face_entity: i32) -> Option<ActorTargetView> {
    if face_entity == -1 {
        return None;
    }
    if face_entity < 32768 {
        Some(ActorTargetView { kind: ActorKind::Npc, index: face_entity as usize })
    } else {
        Some(ActorTargetView { kind: ActorKind::Player, index: (face_entity - 32768) as usize })
    }
}

/// The shared actor fields from one entity, as the m8aq `ActorSnapshot`.
fn actor_view(
    entity: &ClientEntity,
    level: i32,
    distance: i32,
    name: Option<String>,
    actions: Vec<Option<String>>,
) -> ActorView {
    ActorView {
        name,
        actions,
        tile: WorldTile { x: entity.x, z: entity.z, level },
        distance,
        animation: entity.primary_anim,
        pose_animation: entity.secondary_anim,
        orientation: entity.yaw,
        target_orientation: entity.dst_yaw,
        overhead_text: entity.chat_message.clone(),
        spot_animation: entity.spotanim_id,
        health: entity.health,
        total_health: entity.total_health,
        face_entity: entity.face_entity,
        target: decode_target(entity.face_entity),
        moving: entity.route_length > 0,
        running: entity.primary_anim == entity.runanim,
        in_combat: entity.combat_cycle > 0,
    }
}

fn track(world: u64, tracked: &mut u64) -> bool {
    if *tracked == world {
        return false;
    }
    *tracked = world;
    true
}
