//! Random-event detection over one `GameSnapshot` (guardian spec
//! `2026-09-01-random-event-guardian-design.md`, the Detect / Ours / Kinds
//! locks). Detect is snapshot-only and stateless apart from the
//! caller-owned [`CooldownMap`]; act / hold / dialog and the wrong-talk
//! 45 s cooldown writes are Task 4.

use std::collections::HashMap;

use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot, NpcView};

/// The kind of random event one detection found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomKind {
    Dialog,
    Pick,
    Evade,
    Maze,
    Mime,
    Box,
    Lamp,
    Hazard,
    LostTool,
    LostGear,
}

/// One detected random event on the current snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRandom {
    pub kind: RandomKind,
    pub name: String,
    pub ours: bool,
    pub npc_index: Option<usize>,
}

/// NPC slot index → cooldown expiry `now_ms`: the slot is skipped while
/// `now_ms < value`. Task 4 stores `now_ms + 45_000` on a wrong-talk.
pub type CooldownMap = HashMap<usize, u64>;

/// Maze square: `x>>6 == 45 && z>>6 == 71` at level 0.
const MAZE_X: i32 = 45;
const MAZE_Z: i32 = 71;
/// Mime square: `x>>6 == 31 && z>>6 == 74` at level 0.
const MIME_X: i32 = 31;
const MIME_Z: i32 = 74;

/// Dialog act-set names, lowercase (`NpcView.name`).
const DIALOG_NAMES: &[&str] = &[
    "genie",
    "drunken dwarf",
    "mysterious old man",
    "sandwich lady",
    "frog",
];

/// Hostile evade names: the 274 `antimacro.npc` macro guardians (their
/// npc ids 391..443 match rs2b0t, verified against the Lost City pack).
/// Detect is name-based per the guardian spec.
const EVADE_NAMES: &[&str] = &[
    "river troll",
    "swarm",
    "rock golem",
    "zombie",
    "shade",
    "watchman",
    "tree spirit",
];

/// The growing plant (pick kind): shown even when not ours yet.
const PICK_NAME: &str = "strange plant";

/// `Strange box` obj id (verified against the Lost City pack).
const STRANGE_BOX_OBJ: i32 = 3062;
/// `Lamp` (the genie lamp) obj id (verified against the Lost City pack).
const LAMP_OBJ: i32 = 2528;

/// Ground-search radius for lost tool/gear (rs2b0t `.within(10)`).
const LOST_GEAR_RADIUS: i32 = 10;

/// Fishing gear the macro randoms can knock off (rs2b0t `FISHING_GEAR`).
const FISHING_GEAR: &[&str] = &[
    "small fishing net",
    "big fishing net",
    "fishing rod",
    "oily fishing rod",
    "fly fishing rod",
    "harpoon",
    "lobster pot",
    "fishing bait",
    "feather",
];

/// Detect the first random event the snapshot shows, in rs2b0t
/// `detectRaw` order: maze/mime by map square, then scene NPCs/locs,
/// then lost-gear on the ground, then inv-held box/lamp, then lost-tool.
/// Returns `None` when nothing applies. NPC kinds are owner-gated
/// (except `pick`, which the TUI may show as not ours). The rs2b0t
/// gear-loss 90 s window (the caller-state half of `lost-gear`) is not
/// needed here: gear on the ground and out of the inventory detects.
pub fn detect(snap: &GameSnapshot, now_ms: u64, cooldown: &CooldownMap) -> Option<DetectedRandom> {
    if let Some((x, z, level)) = snap.tile() {
        if level == 0 {
            if x >> 6 == MIME_X && z >> 6 == MIME_Z {
                return Some(no_npc_event(RandomKind::Mime, "mime"));
            }
            if x >> 6 == MAZE_X && z >> 6 == MAZE_Z {
                return Some(no_npc_event(RandomKind::Maze, "maze"));
            }
        }
    }
    if let Some(ev) = detect_scene(snap, now_ms, cooldown) {
        return Some(ev);
    }
    if let Some(gear) = lost_gear(snap) {
        return Some(no_npc_event(RandomKind::LostGear, &gear));
    }
    if snap.inv().iter().any(|(id, _)| *id == STRANGE_BOX_OBJ) {
        return Some(no_npc_event(RandomKind::Box, "strange box"));
    }
    if snap.inv().iter().any(|(id, _)| *id == LAMP_OBJ) {
        return Some(no_npc_event(RandomKind::Lamp, "lamp"));
    }
    if has_lost_tool(snap) {
        return Some(no_npc_event(RandomKind::LostTool, "lost tool"));
    }
    None
}

/// A map-square or inventory-held event: ours by position/possession.
fn no_npc_event(kind: RandomKind, name: &str) -> DetectedRandom {
    DetectedRandom {
        kind,
        name: name.to_string(),
        ours: true,
        npc_index: None,
    }
}

/// Scene NPCs and locs, in rs2b0t `detectSceneEvents` order: dialog /
/// pick / evade over the NPC pass, then hazard locs. Cooled NPC slots
/// (the 45 s wrong-talk bin) are skipped.
fn detect_scene(
    snap: &GameSnapshot,
    now_ms: u64,
    cooldown: &CooldownMap,
) -> Option<DetectedRandom> {
    let self_slot = snap.self_slot();
    let display_name = snap
        .local_player()
        .and_then(|lp| lp.player.actor.name.clone());
    for npc in snap.npcs() {
        if cooldown
            .get(&npc.index)
            .is_some_and(|until| now_ms < *until)
        {
            continue;
        }
        let Some(raw) = npc.name.as_deref() else {
            continue;
        };
        let name = raw.to_lowercase();
        let ours = is_ours(npc, self_slot, display_name.as_deref());
        let facing_self = npc.target
            == Some(ActorTargetView {
                kind: ActorKind::Player,
                index: self_slot.max(0) as usize,
            });
        // The drunken dwarf is dialog until it comes for us (`target ==
        // self`); then it is evade like the hostile guardians.
        if ours
            && (EVADE_NAMES.contains(&name.as_str()) || (name == "drunken dwarf" && facing_self))
        {
            return Some(DetectedRandom {
                kind: RandomKind::Evade,
                name,
                ours: true,
                npc_index: Some(npc.index),
            });
        }
        if ours && DIALOG_NAMES.contains(&name.as_str()) {
            return Some(DetectedRandom {
                kind: RandomKind::Dialog,
                name,
                ours: true,
                npc_index: Some(npc.index),
            });
        }
        // Growing plants give no client tell; the TUI can still name it.
        if name == PICK_NAME {
            return Some(DetectedRandom {
                kind: RandomKind::Pick,
                name,
                ours,
                npc_index: Some(npc.index),
            });
        }
    }
    for loc in snap.locs() {
        let Some(raw) = loc.name.as_deref() else {
            continue;
        };
        let name = raw.to_lowercase();
        // Hazard names first, per the spec's pack note: rs2b0t's
        // gas-chest / smoking-rock ids collide with unrelated 274 locs
        // ("Chest", "Rocks"), so this rev matches names only. Whirlpool
        // locs exist in the 274 pack; the other two are no-ops here.
        if name == "whirlpool" {
            return Some(DetectedRandom {
                kind: RandomKind::Hazard,
                name: "whirlpool".to_string(),
                ours: true,
                npc_index: None,
            });
        }
        if name == "gas chest" {
            return Some(DetectedRandom {
                kind: RandomKind::Hazard,
                name: "poisonous gas".to_string(),
                ours: true,
                npc_index: None,
            });
        }
        if name == "smoking rock" {
            return Some(DetectedRandom {
                kind: RandomKind::Hazard,
                name: "smoking rock".to_string(),
                ours: true,
                npc_index: None,
            });
        }
    }
    None
}

/// The spec's hard owner: the NPC faces the local player, or its
/// overhead text contains the local display name. No distance grab.
fn is_ours(npc: &NpcView, self_slot: i32, display_name: Option<&str>) -> bool {
    if npc.target
        == Some(ActorTargetView {
            kind: ActorKind::Player,
            index: self_slot.max(0) as usize,
        })
    {
        return true;
    }
    match (display_name, npc.overhead_text.as_deref()) {
        (Some(name), Some(text)) => text.contains(name),
        _ => false,
    }
}

/// The random's axe/pickaxe handle sits in the inventory, worn, or on
/// the ground near us (rs2b0t `handleLocation` vs inv/ground).
fn has_lost_tool(snap: &GameSnapshot) -> bool {
    let handle = |name: Option<&str>| {
        name.is_some_and(|n| {
            let l = n.to_lowercase();
            l.contains("axe handle") || l.contains("pickaxe handle")
        })
    };
    snap.inventory()
        .iter()
        .any(|i| handle(i.def.name.as_deref()))
        || snap
            .equipment()
            .iter()
            .any(|i| handle(i.def.name.as_deref()))
        || snap
            .ground_items()
            .iter()
            .any(|g| g.distance <= LOST_GEAR_RADIUS && handle(g.def.name.as_deref()))
}

/// A fishing-gear item on the ground near us that we are not holding
/// (rs2b0t `GearLossTracker` ground search, minus the 90 s window).
fn lost_gear(snap: &GameSnapshot) -> Option<String> {
    for gear in FISHING_GEAR {
        let in_inv = snap
            .inventory()
            .iter()
            .any(|i| item_named(i.def.name.as_deref(), gear));
        if !in_inv
            && snap
                .ground_items()
                .iter()
                .any(|g| g.distance <= LOST_GEAR_RADIUS && item_named(g.def.name.as_deref(), gear))
        {
            return Some((*gear).to_string());
        }
    }
    None
}

fn item_named(name: Option<&str>, want: &str) -> bool {
    name.is_some_and(|n| n.eq_ignore_ascii_case(want))
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::client::{Client, ClientConfig, ClientPlayer};
    use client::config::{Cache, NpcType, ObjType};
    use client::dash3d::{ClientNpc, ClientObj};
    use client::datastruct::LinkList;
    use std::sync::Arc;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        }
    }

    /// An attached-free client at base (0, 0) with the local player slot 0.
    fn new_client() -> Client {
        let mut c = crate::prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.map_build_base_x = 0;
        c.map_build_base_z = 0;
        c.self_slot = 0;
        c
    }

    /// Plant the local player with display name `name` on world tile
    /// `(x, z)` (base 0, level 0).
    fn plant_player(c: &mut Client, name: &str, x: i32, z: i32) {
        let mut lp = ClientPlayer::at(x, z);
        lp.entity.x = x * 128 + 64;
        lp.entity.z = z * 128 + 64;
        lp.name = Some(name.to_string());
        c.local_player = Some(lp);
    }

    /// An NPC of cache type `name` in client table slot `slot` (the
    /// `NpcView.index` / cooldown key) with the given `face_entity` and
    /// overhead text.
    fn plant_npc(
        c: &mut Client,
        slot: usize,
        name: &str,
        face_entity: i32,
        overhead: Option<&str>,
    ) {
        let type_id = 500 + slot;
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.npcs.len() <= type_id {
                cache.npcs.push(NpcType::default());
            }
            cache.npcs[type_id] = NpcType {
                id: type_id as i32,
                name: name.to_string(),
                ..Default::default()
            };
        }
        let mut npc = ClientNpc::at(0, 0);
        npc.r#type = Some(type_id);
        npc.entity.face_entity = face_entity;
        npc.entity.chat_message = overhead.map(str::to_string);
        while c.npc.len() <= slot {
            c.npc.push(None);
        }
        c.npc[slot] = Some(Box::new(npc));
        c.npc_ids.push(slot as i32);
        c.npc_count = c.npc_ids.len() as i32;
    }

    /// A ground item of cache obj `name` on scene tile (x, z) (level 0).
    fn plant_ground_obj(c: &mut Client, x: i32, z: i32, obj_id: i32, name: Option<&str>) {
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.objs.len() <= obj_id as usize {
                cache.objs.push(ObjType::default());
            }
            if let Some(name) = name {
                cache.objs[obj_id as usize] = ObjType {
                    id: obj_id,
                    name: name.to_string(),
                    ..Default::default()
                };
            }
        }
        let mut list = LinkList::new();
        list.push_front(ClientObj::new(obj_id, 1));
        c.ground_obj[0][x as usize][z as usize] = Some(Box::new(list));
    }

    /// Rebuild a fresh snapshot with the npc, player and scene families
    /// moved (scene moves the loc/ground-item views too).
    fn snap_at(c: &mut Client) -> GameSnapshot {
        c.gens.npc = 1;
        c.gens.player = 1;
        c.gens.scene = 1;
        let mut snap = GameSnapshot::new();
        snap.rebuild(c);
        snap
    }

    fn no_cooldown() -> CooldownMap {
        HashMap::new()
    }

    #[test]
    fn overhead_greetings_with_local_name_is_dialog_ours() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("dialog detected");
        assert_eq!(ev.kind, RandomKind::Dialog);
        assert_eq!(ev.name, "genie");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, Some(0));
    }

    #[test]
    fn neighbour_genie_overhead_other_name_is_not_ours() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Bob!"));
        let snap = snap_at(&mut c);
        assert_eq!(detect(&snap, 0, &no_cooldown()), None);
    }

    #[test]
    fn swarm_targeting_self_is_evade_ours_untargeted_swarm_is_not() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        // `face_entity` >= 32768 decodes as Player kind; + self_slot (0).
        plant_npc(&mut c, 0, "Swarm", 32768, None);
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("evade detected");
        assert_eq!(ev.kind, RandomKind::Evade);
        assert_eq!(ev.name, "swarm");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, Some(0));

        // Same type without a target: not ours, so nothing to detect.
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Swarm", -1, None);
        let snap = snap_at(&mut c);
        assert_eq!(detect(&snap, 0, &no_cooldown()), None);
    }

    #[test]
    fn maze_square_is_maze_ours_with_no_npc() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 45 * 64, 71 * 64);
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("maze detected");
        assert_eq!(ev.kind, RandomKind::Maze);
        assert_eq!(ev.name, "maze");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, None);
    }

    #[test]
    fn ground_axe_handle_is_lost_tool() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_ground_obj(&mut c, 0, 0, 500, Some("Axe handle"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("lost-tool detected");
        assert_eq!(ev.kind, RandomKind::LostTool);
        assert_eq!(ev.name, "lost tool");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, None);
    }

    #[test]
    fn ground_fishing_gear_not_in_inv_is_lost_gear() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_ground_obj(&mut c, 0, 0, 501, Some("Fishing rod"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("lost-gear detected");
        assert_eq!(ev.kind, RandomKind::LostGear);
        assert_eq!(ev.name, "fishing rod");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, None);
    }
}
