//! Generation-stamped snapshot families. `GameSnapshot` owns per-family
//! views rebuilt only when that family's gen moved; reads borrow the last
//! rebuild instead of deep-copying the world on every read.

use crate::obj_names::ItemDefView;
use client::client::{Client, ClientGens, ClientNpc, Skill};
use client::config::if_type::ComponentType;
use client::config::{Cache, ObjType};
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
/// `Loc`/`GroundItem` have no counters of their own: loc and ground-item
/// changes bump `gens.scene`, so both track it with a dedicated slot.
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
    Loc,
    GroundItem,
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
    /// and the old tests read them). These are the raw entity pixel coords,
    /// not the world `tile` above.
    pub x: i32,
    pub z: i32,
    pub yaw: i32,
}

impl NpcView {
    fn from_slot(
        index: usize,
        npc: &ClientNpc,
        base: (i32, i32),
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
            tile: entity_world_tile(entity, base, level),
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

/// The sim-world layer a placed loc occupies (the m8aq `LocSnapshot.layer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LocLayer {
    Wall,
    WallDecoration,
    Ground,
    GroundDecoration,
}

/// One placed loc: the packed typecodes plus the decoded shape/angle and
/// the resolved definition (name/actions/footprint/block flags) from the
/// loc table.
#[derive(Debug, Clone, Serialize)]
pub struct LocView {
    pub typecode: i32,
    pub info: i32,
    pub id: i32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
    pub layer: LocLayer,
    pub shape: i32,
    pub angle: i32,
    pub width: i32,
    pub length: i32,
    pub footprint_width: i32,
    pub footprint_length: i32,
    pub block_walk: bool,
    pub block_range: bool,
    pub active: bool,
    pub animation: i32,
    pub map_function: i32,
    pub map_scene: i32,
    pub force_approach: i32,
}

/// One object stack on the ground: the obj definition plus the stack count
/// and the ground menu ops.
#[derive(Debug, Clone, Serialize)]
pub struct GroundItemView {
    pub def: ItemDefView,
    pub count: i32,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
}

/// The built scene: the collision grid the nav/query surface reads, as a
/// flat row-major `x * width + z` flag list.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SceneView {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub collision_flags: Vec<i32>,
}

/// The client's world scalars (the m8aq `WorldStateSnapshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct WorldStateView {
    pub map_base_x: i32,
    pub map_base_z: i32,
    pub level: i32,
    pub members: bool,
    pub multi_combat: bool,
    pub player_count: i32,
    pub npc_count: i32,
    pub cycle: i32,
}

/// The camera state: the follow-camera eye plus the orbit target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct CameraView {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub pitch: i32,
    pub yaw: i32,
    pub orbit_pitch: i32,
    pub orbit_yaw: i32,
    pub cinematic: bool,
}

/// The minimap destination flag, in scene-local tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MapFlagView {
    pub lx: i32,
    pub lz: i32,
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
    /// Placed locs from the last loc rebuild (sweeps the sim world's four
    /// layers at `minusedlevel`).
    loc: Vec<LocView>,
    /// Ground-object stacks from the last ground-item rebuild.
    ground_item: Vec<GroundItemView>,
    /// The built scene's collision grid from the last scene rebuild.
    scene: SceneView,
    /// The client's world scalars from the last world rebuild.
    world: WorldStateView,
    /// The camera state from the last camera rebuild.
    camera: CameraView,
    /// The minimap flag from the last map-flag rebuild; `None` while no
    /// flag is set.
    map_flag: Option<MapFlagView>,
    /// Scene gen the loc/ground-item views were rebuilt up to. Loc and
    /// ground-item changes bump `gens.scene`, so both track it here
    /// (separately from the scene family's own counter).
    #[serde(skip)]
    loc_gen: u64,
    #[serde(skip)]
    ground_item_gen: u64,
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
    /// their counter so a later view can detect movement. `client` is
    /// mutable because the ground-item lists are iterated in place (the
    /// client's own `LinkList` cursor pattern).
    pub fn rebuild_family(&mut self, client: &mut Client, family: Family) -> bool {
        match family {
            Family::Npc => self.rebuild_npcs(client),
            Family::Player => self.rebuild_player(client),
            Family::Inv => self.rebuild_inv(client),
            Family::Varp => track(client.gens.varp, &mut self.gens.varp),
            Family::Stat => self.rebuild_stat(client),
            Family::Chat => self.rebuild_chat(client),
            Family::Scene => self.rebuild_scene(client),
            Family::Loc => self.rebuild_loc(client),
            Family::GroundItem => self.rebuild_ground_items(client),
            Family::Iface => track(client.gens.iface, &mut self.gens.iface),
            Family::Camera => self.rebuild_camera(client),
            Family::MapFlag => self.rebuild_map_flag(client),
            Family::World => self.rebuild_world(client),
        }
    }

    /// Rebuild every family whose gen moved (the harness "one snapshot
    /// per tick" read). Returns true iff any family gen moved.
    pub fn rebuild(&mut self, client: &mut Client) -> bool {
        let mut dirty = false;
        dirty |= self.rebuild_family(client, Family::Npc);
        dirty |= self.rebuild_family(client, Family::Player);
        dirty |= self.rebuild_family(client, Family::Inv);
        dirty |= self.rebuild_family(client, Family::Varp);
        dirty |= self.rebuild_family(client, Family::Stat);
        dirty |= self.rebuild_family(client, Family::Chat);
        dirty |= self.rebuild_family(client, Family::Scene);
        dirty |= self.rebuild_family(client, Family::Loc);
        dirty |= self.rebuild_family(client, Family::GroundItem);
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

    /// Placed locs from the last loc rebuild, in scene sweep order (per
    /// tile: wall, ground, ground decoration, wall decoration).
    pub fn locs(&self) -> &[LocView] {
        &self.loc
    }

    /// Ground-object stacks from the last ground-item rebuild.
    pub fn ground_items(&self) -> &[GroundItemView] {
        &self.ground_item
    }

    /// The built scene (base, level, collision flags) from the last scene
    /// rebuild; the default is "no scene available".
    pub fn scene(&self) -> &SceneView {
        &self.scene
    }

    /// The client's world scalars from the last world rebuild.
    pub fn world(&self) -> &WorldStateView {
        &self.world
    }

    /// The camera state from the last camera rebuild.
    pub fn camera(&self) -> &CameraView {
        &self.camera
    }

    /// The minimap flag from the last map-flag rebuild; `None` while no
    /// flag is set.
    pub fn map_flag(&self) -> Option<&MapFlagView> {
        self.map_flag.as_ref()
    }

    fn rebuild_stat(&mut self, client: &mut Client) -> bool {
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
    fn rebuild_player(&mut self, client: &mut Client) -> bool {
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
                    base,
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
                let tile = entity_world_tile(&player.entity, base, level);
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                self.players.push(PlayerView {
                    index,
                    actor: actor_view(
                        &player.entity,
                        base,
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
    fn rebuild_inv(&mut self, client: &mut Client) -> bool {
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
    fn rebuild_chat(&mut self, client: &mut Client) -> bool {
        if !track(client.gens.chat, &mut self.gens.chat) {
            return false;
        }
        let latest = client.chat_text[0].clone();
        self.chat = (!latest.is_empty()).then_some(latest);
        true
    }

    /// Scene-family rebuild: `ingame` + `scene_state`, always fresh —
    /// these flip locally (`check_scene` sets `scene_state = 2` on the SIM
    /// loop with no gen bump), so a gen-gated copy would pin the snapshot
    /// in a stale "loading" state. The `SceneView` (build base, level and
    /// the collision grid) only changes on a world build, so it rebuilds
    /// when the scene gen moves. The return value still tracks the gen for
    /// the harness's dirty/tick semantics.
    fn rebuild_scene(&mut self, client: &mut Client) -> bool {
        let moved = track(client.gens.scene, &mut self.gens.scene);
        self.ingame = client.ingame;
        self.scene_state = client.scene_state;
        if moved {
            let level = client.minusedlevel;
            match client.collision.get(level as usize) {
                Some(cmap) => {
                    self.scene = SceneView {
                        available: true,
                        base_x: client.map_build_base_x,
                        base_z: client.map_build_base_z,
                        level,
                        width: cmap.size_x,
                        height: cmap.size_z,
                        collision_flags: cmap.flags.iter().flatten().copied().collect(),
                    };
                }
                None => {
                    self.scene = SceneView {
                        available: false,
                        base_x: client.map_build_base_x,
                        base_z: client.map_build_base_z,
                        level,
                        ..SceneView::default()
                    };
                }
            }
        }
        moved
    }

    /// Loc-family rebuild: sweep the sim world's four layers at
    /// `minusedlevel` (locs sit on scene tiles, so the world tile is
    /// `base + scene` with no pixel conversion). Gated on the scene gen —
    /// loc changes arrive on scene-family packets.
    fn rebuild_loc(&mut self, client: &mut Client) -> bool {
        if !track(client.gens.scene, &mut self.loc_gen) {
            return false;
        }
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        self.loc.clear();
        for sx in 0..104 {
            for sz in 0..104 {
                // Per-tile layer order matches the m8aq sweep: wall,
                // ground, ground decoration, wall decoration.
                if let Some(wall) = client.world.get_wall(level, sx, sz) {
                    self.loc.push(loc_view(
                        wall.typecode,
                        wall.typecode2 & 0xff,
                        LocLayer::Wall,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
                if let Some(sprite) = client.world.get_scene(level, sx, sz) {
                    self.loc.push(loc_view(
                        sprite.typecode,
                        sprite.typecode2 & 0xff,
                        LocLayer::Ground,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
                if let Some(gd) = client.world.get_gd(level, sx, sz) {
                    self.loc.push(loc_view(
                        gd.typecode,
                        gd.typecode2 & 0xff,
                        LocLayer::GroundDecoration,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
                if let Some(decor) = client.world.get_decor(level, sx, sz) {
                    self.loc.push(loc_view(
                        decor.typecode,
                        decor.typecode2 & 0xff,
                        LocLayer::WallDecoration,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
            }
        }
        true
    }

    /// Ground-item rebuild: iterate each `ground_obj` list at
    /// `minusedlevel` into a `GroundItemView` (obj definition, stack
    /// count, ground ops). The client's `LinkList` cursor is mutated in
    /// place (the same `head`/`next` pattern the client's own handlers
    /// use). Gated on the scene gen — object packets bump it.
    fn rebuild_ground_items(&mut self, client: &mut Client) -> bool {
        if !track(client.gens.scene, &mut self.ground_item_gen) {
            return false;
        }
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        self.ground_item.clear();
        for x in 0..104 {
            for z in 0..104 {
                let cell = &mut client.ground_obj[level as usize][x as usize][z as usize];
                let Some(list) = cell.as_mut() else {
                    continue;
                };
                let tile = WorldTile {
                    x: base.0 + x,
                    z: base.1 + z,
                    level,
                };
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                let mut node = list.head();
                while let Some(obj) = node {
                    self.ground_item.push(GroundItemView {
                        def: item_def_view(&client.cache, obj.id),
                        count: obj.count,
                        actions: ground_ops(&client.cache, obj.id),
                        tile,
                        distance,
                    });
                    node = list.next();
                }
            }
        }
        true
    }

    /// World-state rebuild: the client's world scalars. Cheap reads copy
    /// every rebuild (like the scene status), so counts stay fresh between
    /// world-gen bumps; the return value tracks the gen for the harness.
    fn rebuild_world(&mut self, client: &mut Client) -> bool {
        let moved = track(client.gens.world, &mut self.gens.world);
        self.world = WorldStateView {
            map_base_x: client.map_build_base_x,
            map_base_z: client.map_build_base_z,
            level: client.minusedlevel,
            members: client.members_account != 0,
            multi_combat: client.in_multizone != 0,
            player_count: client.player_count,
            npc_count: client.npc_count,
            cycle: client.loop_cycle,
        };
        moved
    }

    /// Camera rebuild: the follow-camera eye, the orbit target and the
    /// cinematic flag. The follow camera eases every frame with no packet,
    /// so the cheap fields copy every rebuild; the return value tracks the
    /// gen.
    fn rebuild_camera(&mut self, client: &mut Client) -> bool {
        let moved = track(client.gens.camera, &mut self.gens.camera);
        self.camera = CameraView {
            x: client.cam_x,
            y: client.cam_y,
            z: client.cam_z,
            pitch: client.cam_pitch,
            yaw: client.cam_yaw,
            orbit_pitch: client.orbit_camera_pitch,
            orbit_yaw: client.orbit_camera_yaw,
            cinematic: client.cinema_cam,
        };
        moved
    }

    /// Map-flag rebuild: the minimap destination flag, `Some` only while
    /// it is set. The flag flips on minimap clicks with no packet, so the
    /// view copies every rebuild; the return value tracks the gen.
    fn rebuild_map_flag(&mut self, client: &mut Client) -> bool {
        let moved = track(client.gens.map_flag, &mut self.gens.map_flag);
        self.map_flag = (client.minimap_flag_x != 0)
            .then_some(MapFlagView {
                lx: client.minimap_flag_x,
                lz: client.minimap_flag_z,
            });
        moved
    }

    fn rebuild_npcs(&mut self, client: &mut Client) -> bool {
        if client.gens.npc == self.gens.npc {
            return false;
        }
        self.gens.npc = client.gens.npc;
        self.npc.clear();
        self.npc.reserve(client.npc_count as usize);
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        for i in 0..client.npc_count as usize {
            let index = client.npc_ids[i] as usize;
            if let Some(npc) = client.npc.get(index).and_then(|n| n.as_ref()) {
                let tile = entity_world_tile(&npc.entity, base, level);
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                self.npc
                    .push(NpcView::from_slot(index, npc, base, level, distance, &client.cache));
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

/// The absolute world tile of an entity: its scene-local pixel coords
/// (`route * 128 + size * 64`) un-scaled by 128 and offset by the build
/// base, so every actor view is in the same world-tile space as the local
/// player's tile.
fn entity_world_tile(entity: &ClientEntity, base: (i32, i32), level: i32) -> WorldTile {
    WorldTile {
        x: base.0 + (entity.x - entity.size * 64) / 128,
        z: base.1 + (entity.z - entity.size * 64) / 128,
        level,
    }
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
    base: (i32, i32),
    level: i32,
    distance: i32,
    name: Option<String>,
    actions: Vec<Option<String>>,
) -> ActorView {
    ActorView {
        name,
        actions,
        tile: entity_world_tile(entity, base, level),
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

/// One `LocView` from a placed layer: decode the loc id and the
/// shape/angle info byte, then resolve the definition from the loc table.
/// An unloaded loc id reads the `LocType` defaults (the m8aq `LocType.list`
/// dummy type).
#[allow(clippy::too_many_arguments)]
fn loc_view(
    typecode: i32,
    info: i32,
    layer: LocLayer,
    base: (i32, i32),
    level: i32,
    sx: i32,
    sz: i32,
    local_tile: Option<(i32, i32)>,
    cache: &Cache,
) -> LocView {
    let id = (typecode >> 14) & 0x7fff;
    let shape = info & 0x1f;
    let angle = (info >> 6) & 0x3;
    let x = base.0 + sx;
    let z = base.1 + sz;
    let (name, description, actions, width, length, block_walk, block_range, active, animation, map_function, map_scene, force_approach) =
        match cache.locs.get(id as usize) {
            Some(loc) => (
                (!loc.name.is_empty()).then(|| loc.name.clone()),
                (!loc.desc.is_empty()).then(|| loc.desc.clone()),
                loc.op.clone(),
                loc.width,
                loc.length,
                loc.blockwalk,
                loc.blockrange,
                loc.active,
                loc.anim,
                loc.mapfunction,
                loc.mapscene,
                loc.forceapproach,
            ),
            None => (None, None, Vec::new(), 1, 1, true, true, false, -1, -1, -1, 0),
        };
    LocView {
        typecode,
        info,
        id,
        name,
        description,
        actions,
        tile: WorldTile { x, z, level },
        distance: local_tile
            .map(|(lx, lz)| chebyshev(x, z, lx, lz))
            .unwrap_or(0),
        layer,
        shape,
        angle,
        width,
        length,
        // A 90°/270° rotation swaps the footprint axes.
        footprint_width: if angle == 1 || angle == 3 { length } else { width },
        footprint_length: if angle == 1 || angle == 3 { width } else { length },
        block_walk,
        block_range,
        active,
        animation,
        map_function,
        map_scene,
        force_approach,
    }
}

/// The obj's definition view via Task 1's mapping. An unloaded obj id
/// reads the `ObjType` defaults with the requested id (the m8aq
/// `ObjType.list` dummy type).
fn item_def_view(cache: &Cache, id: i32) -> ItemDefView {
    let mut def = match cache.objs.get(id as usize) {
        Some(o) => ItemDefView::from_obj(o),
        None => ItemDefView::from_obj(&ObjType::default()),
    };
    def.id = id;
    def
}

/// The ground menu ops for an obj: the type's `op` table padded to five
/// slots with a `Take` default filling an empty third (m8aq `groundOps`).
fn ground_ops(cache: &Cache, id: i32) -> Vec<Option<String>> {
    let mut ops = cache
        .objs
        .get(id as usize)
        .map(|o| o.op.to_vec())
        .unwrap_or_else(|| vec![None; 5]);
    if ops.len() < 3 {
        ops.resize(3, None);
    }
    if ops[2].is_none() {
        ops[2] = Some("Take".into());
    }
    ops
}

fn track(world: u64, tracked: &mut u64) -> bool {
    if *tracked == world {
        return false;
    }
    *tracked = world;
    true
}
