use super::*;
use api::prot::Out;
use client::client::{Client, ClientConfig, ClientPlayer, MiniMenuAction};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::{Cache, LocType, NpcType, ObjType};
use client::dash3d::{ClientNpc, ClientObj, LocAngle, LocShape};
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
    // Evade's takingDamage gate requires an active session; dialog/pick
    // detect paths stay ownership-only and do not depend on this flag.
    c.ingame = true;
}

/// Write one hitmark slot on the local player (value/type/cycle).
fn plant_hit(c: &mut Client, slot: usize, value: i32, dtype: i32, cycle: i32) {
    let lp = c.local_player.as_mut().expect("local player");
    lp.entity.damage_values[slot] = value;
    lp.entity.damage_types[slot] = dtype;
    lp.entity.damage_cycles[slot] = cycle;
}

/// Active positive type-1 combat hit (value 5, type 1, cycle loop+70).
fn plant_positive_hit(c: &mut Client) {
    let until = c.loop_cycle + 70;
    plant_hit(c, 0, 5, 1, until);
}

/// An NPC of cache type `name` in client table slot `slot` (the
/// `NpcView.index` / cooldown key) with the given `face_entity` and
/// overhead text.
fn plant_npc(c: &mut Client, slot: usize, name: &str, face_entity: i32, overhead: Option<&str>) {
    plant_npc_with_op(c, slot, name, face_entity, overhead, "Talk-to");
}

fn plant_npc_typed(
    c: &mut Client,
    slot: usize,
    type_id: usize,
    name: &str,
    face_entity: i32,
    overhead: Option<&str>,
) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.npcs.len() <= type_id {
            cache.npcs.push(NpcType::default());
        }
        cache.npcs[type_id] = NpcType {
            id: type_id as i32,
            name: name.to_string(),
            op: vec![Some("Talk-to".to_string())],
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
    c.npc_ids[c.npc_count as usize] = slot as i32;
    c.npc_count += 1;
}

fn clear_npcs(c: &mut Client) {
    c.npc_count = 0;
    c.npc.fill(None);
}

fn plant_bank_open(c: &mut Client) {
    c.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    c.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    c.main_modal_id = 600;
    c.gens.iface += 1;
}

fn plant_shop_open(c: &mut Client) {
    c.main_modal_id = 3824;
    c.gens.iface += 1;
}

/// Like [`plant_npc`] but with a chosen first menu op (the growing
/// plant's `Pick`).
fn plant_npc_with_op(
    c: &mut Client,
    slot: usize,
    name: &str,
    face_entity: i32,
    overhead: Option<&str>,
    op: &str,
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
            op: vec![Some(op.to_string())],
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
    // `npc_ids` is a fixed `vec![0; MAX_NPC_COUNT]`; write the slot at
    // the count index (like `handle_packet`) instead of pushing, so the
    // snapshot's view list is exactly the planted slots in order.
    c.npc_ids[c.npc_count as usize] = slot as i32;
    c.npc_count += 1;
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

/// Advance every packet family and rebuild the **persistent**
/// snapshot: like the host's drain, one call per game tick, so
/// `snap.tick()` climbs 1, 2, … (a fresh `GameSnapshot` starts its
/// tick at 0 again, which is why the guardian tests reuse one).
fn tick_at(c: &mut Client, snap: &mut GameSnapshot) {
    c.gens.npc = c.gens.npc.wrapping_add(1);
    c.gens.player = c.gens.player.wrapping_add(1);
    c.gens.scene = c.gens.scene.wrapping_add(1);
    c.gens.iface = c.gens.iface.wrapping_add(1);
    c.gens.chat = c.gens.chat.wrapping_add(1);
    snap.rebuild(c);
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
fn talking_random_targeting_another_player_is_ignored_but_ours_is_handled() {
    let settings = ProfileSettings::default();

    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    // The target is player slot 1, not local slot 0. Even misleading
    // overhead ownership text must not make this other player's genie ours.
    plant_npc(&mut c, 0, "Genie", 32769, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let mut snap = GameSnapshot::new();
    let mut knocks = 0;

    tick_at(&mut c, &mut snap);
    let status = {
        let mut knock = |_: &DetectedRandom| {
            knocks += 1;
            RandomClaim::Handle
        };
        g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock))
    };
    assert_eq!(knocks, 0, "a foreign talking random must not claim");
    assert_eq!(status.kind, None);
    assert!(!status.ours);
    assert!(!status.handling);
    assert!(!status.hold);
    assert!(drv.menus.is_empty());
    assert!(drv.actions.is_empty());

    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", 32768, None);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Dialog));
    assert!(status.ours);
    assert!(status.handling);
    assert!(status.hold);
    assert_eq!(drv.menus, vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)]);
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn swarm_targeting_self_is_evade_ours_untargeted_swarm_is_not() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_positive_hit(&mut c);
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
    plant_positive_hit(&mut c);
    plant_npc(&mut c, 0, "Swarm", -1, None);
    let snap = snap_at(&mut c);
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);
}

#[test]
fn owned_hostile_without_positive_hit_is_not_evade() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Swarm", 32768, None);
    let snap = snap_at(&mut c);
    assert!(
        !snap.taking_damage(),
        "no hitmarks must fail the damage gate"
    );
    assert_eq!(
        detect(&snap, 0, &no_cooldown()),
        None,
        "owned hostile without a positive hit must not flee"
    );
}

#[test]
fn owned_hostile_zero_hit_or_poison_is_not_evade() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    // Zero red splat (value 0, type 1) does not qualify.
    let until = c.loop_cycle + 70;
    plant_hit(&mut c, 0, 0, 1, until);
    plant_npc(&mut c, 0, "Watchman", 32768, None);
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);

    // Poison type 2 with positive value still fails the type-1 gate.
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    let until = c.loop_cycle + 70;
    plant_hit(&mut c, 0, 4, 2, until);
    plant_npc(&mut c, 0, "Shade", 32768, None);
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);

    // Blocked/type-0 value-0 hit is not damage.
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    let until = c.loop_cycle + 70;
    plant_hit(&mut c, 0, 0, 0, until);
    plant_npc(&mut c, 0, "Zombie", 32768, None);
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);
}

#[test]
fn owned_hostile_expired_hit_and_exact_boundary_are_not_evade() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    // Strictly expired: cycle == loop_cycle is no longer active.
    let boundary = c.loop_cycle;
    plant_hit(&mut c, 0, 7, 1, boundary);
    plant_npc(&mut c, 0, "Rock golem", 32768, None);
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);

    // Past expiry.
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    let past = c.loop_cycle - 1;
    plant_hit(&mut c, 0, 7, 1, past);
    plant_npc(&mut c, 0, "Tree spirit", 32768, None);
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);
}

#[test]
fn owned_hostile_positive_hit_with_later_miss_is_evade() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    // Slot 0: live positive combat hit. Slot 1: later miss must not erase it.
    let until = c.loop_cycle + 70;
    plant_hit(&mut c, 0, 6, 1, until);
    plant_hit(&mut c, 1, 0, 1, until);
    plant_npc(&mut c, 0, "River troll", 32768, None);
    let snap = snap_at(&mut c);
    assert!(snap.taking_damage());
    let ev = detect(&snap, 0, &no_cooldown()).expect("evade with live positive hit");
    assert_eq!(ev.kind, RandomKind::Evade);
    assert_eq!(ev.name, "river troll");
}

#[test]
fn evade_expires_without_player_generation_advance() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    let until = c.loop_cycle + 70;
    plant_hit(&mut c, 0, 9, 1, until);
    plant_npc(&mut c, 0, "Swarm", 32768, None);
    let mut snap = GameSnapshot::new();
    c.gens.npc = 1;
    c.gens.player = 1;
    c.gens.scene = 1;
    snap.rebuild(&c);
    assert!(snap.taking_damage());
    assert_eq!(
        detect(&snap, 0, &no_cooldown()).map(|e| e.kind),
        Some(RandomKind::Evade)
    );

    // Advance loop_cycle past the splat without bumping player gen.
    c.loop_cycle += 70;
    let player_gen_before = c.gens.player;
    // rebuild_from_drain still refreshes native facts every call.
    assert!(!snap.rebuild_from_drain(&c, false));
    assert_eq!(c.gens.player, player_gen_before);
    assert!(
        !snap.taking_damage(),
        "hit expiry must clear without a player packet"
    );
    assert_eq!(
        detect(&snap, 0, &no_cooldown()),
        None,
        "evade must drop once the positive hit expires"
    );
}

#[test]
fn no_local_player_or_logged_out_fails_damage_gate() {
    let mut c = new_client();
    c.ingame = true;
    // No local player planted.
    plant_npc(&mut c, 0, "Swarm", 32768, None);
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);

    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_positive_hit(&mut c);
    plant_npc(&mut c, 0, "Swarm", 32768, None);
    c.ingame = false;
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    assert_eq!(detect(&snap, 0, &no_cooldown()), None);
}

#[test]
fn dialog_pick_still_detect_without_damage() {
    // Dialog ownership path must stay independent of the damage gate.
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    let ev = detect(&snap, 0, &no_cooldown()).expect("dialog");
    assert_eq!(ev.kind, RandomKind::Dialog);

    // Pick (strange plant) is shown without requiring damage — it is
    // gated on the owner, not on combat.
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, Some("Pick Test!"), "Pick");
    let snap = snap_at(&mut c);
    assert!(!snap.taking_damage());
    let ev = detect(&snap, 0, &no_cooldown()).expect("pick");
    assert_eq!(ev.kind, RandomKind::Pick);
    assert!(ev.ours);
}

#[test]
fn drunken_dwarf_facing_self_is_dialog_not_evade() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    // FACEENTITY / playerfollow is the ours tell, not combat. A dwarf
    // that has come for us is still the dialog five, not a swarm.
    plant_npc(&mut c, 0, "Drunken dwarf", 32768, None);
    let snap = snap_at(&mut c);
    let ev = detect(&snap, 0, &no_cooldown()).expect("dwarf detected");
    assert_eq!(ev.kind, RandomKind::Dialog);
    assert_eq!(ev.name, "drunken dwarf");
    assert!(ev.ours);
    assert_eq!(ev.npc_index, Some(0));
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
fn bare_ground_fishing_gear_is_not_lost_gear_without_ownership() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_ground_obj(&mut c, 0, 0, 501, Some("Fishing rod"));
    let snap = snap_at(&mut c);
    assert_eq!(
        detect(&snap, 0, &no_cooldown()),
        None,
        "stateless detect must fail closed on unowned ground gear"
    );
}

#[test]
fn inv_box_beats_ground_fishing_net() {
    let mut c = new_client();
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_obj(&mut c, STRANGE_BOX_OBJ);
    plant_ground_obj(&mut c, 0, 0, 502, Some("Small fishing net"));
    let snap = snap_at(&mut c);
    let ev = detect(&snap, 0, &no_cooldown()).expect("box must beat lost-gear");
    assert_eq!(ev.kind, RandomKind::Box);
    assert_eq!(ev.name, "strange box");
}

// --- Guardian (Task 4): act + hold-while-handling, fake Driver ---

/// No-op packet sink the recording driver hands out.
struct NoopOut;
impl Out for NoopOut {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}

/// Recording driver: captures every `set_menu`/`do_action`/`try_move`
/// instead of sending, so the guardian's sends are asserted directly.
/// `route_origin` is `(0,0)` and `build_base` `(0,0)`, so absolute
/// world tiles equal the recorded `try_move` target.
struct FakeDriver {
    menus: Vec<(i32, i32, i32, i32, i32)>,
    actions: Vec<i32>,
    walks: Vec<(i32, i32)>,
    walk_nearest: Vec<bool>,
    walk_ok: bool,
    route_origin: Option<(i32, i32)>,
    out: NoopOut,
}

impl Default for FakeDriver {
    fn default() -> Self {
        Self {
            menus: Vec::new(),
            actions: Vec::new(),
            walks: Vec::new(),
            walk_nearest: Vec::new(),
            walk_ok: true,
            route_origin: Some((0, 0)),
            out: NoopOut,
        }
    }
}

impl Driver for FakeDriver {
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
        self.menus.push((slot, action, a, b, c));
    }
    fn do_action(&mut self, slot: i32) -> bool {
        self.actions.push(slot);
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        dx: i32,
        dz: i32,
        try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _t: i32,
    ) -> bool {
        self.walks.push((dx, dz));
        self.walk_nearest.push(try_nearest);
        self.walk_ok
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        self.route_origin
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

/// Attach a socket and set ingame scene-2 so the `Interactions`
/// preconditions (attached / ingame / scene ready) pass.
fn ingame_scene(c: &mut Client) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    std::mem::forget(listener);
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
}

/// The chat modal root + its BUTTON_CONTINUE child (an open dialog).
const CHAT_ROOT: i32 = 500;
const CHAT_CONTINUE: i32 = 501;

/// Open the chat modal with a continue button (the "dialog is open"
/// state the guardian continues through).
fn open_chat(c: &mut Client) {
    c.set_iface(
        CHAT_ROOT as usize,
        IfType {
            id: CHAT_ROOT,
            children: Some(vec![CHAT_CONTINUE]),
            ..Default::default()
        },
    );
    c.set_iface(
        CHAT_CONTINUE as usize,
        IfType {
            id: CHAT_CONTINUE,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        CHAT_CONTINUE as usize,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            ..Default::default()
        },
    );
    c.chat_modal_id = CHAT_ROOT;
    c.gens.iface += 1;
}

#[test]
fn guardian_talks_to_old_man_then_continues_open_chat() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Mysterious old man", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: our old man is a dialog event → Talk-to.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_eq!(status.kind, Some(RandomKind::Dialog));
    assert_eq!(status.name.as_deref(), Some("mysterious old man"));
    assert!(status.ours);
    assert!(status.handling);
    assert!(status.hold);
    assert!(status.toggle);
    assert!(!status.cooldown);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
        "Talk-to is the npc's first menu op"
    );
    assert_eq!(drv.actions, vec![0]);

    // Tick 2: the dialog is open → continue, not a second Talk-to.
    drv.menus.clear();
    drv.actions.clear();
    open_chat(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert!(status.handling);
    assert!(status.hold);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
        "an open chat continues"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn guardian_talks_to_a_sparse_npc_slot() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    // Only one live NPC, at client slot 7: the snapshot view list has
    // one entry (index 7), so a dense-vec lookup by `npc_index` would
    // miss it.
    plant_npc(&mut c, 7, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.handling);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 7, 0, 0)],
        "Talk-to must address the npc slot, not the dense vec position"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn talk_to_that_never_opens_chat_clears_on_continue_refuse() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: Talk-to arms the handle.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.handling, "Talk-to arms the in-flight handle");
    assert!(status.hold);

    // Tick 2: chat never opened → continue refuses; one-tick grace.
    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(
        status.handling,
        "first refuse after Talk-to keeps the handle for chat to open"
    );
    assert!(drv.actions.is_empty(), "refuse does not press");

    // Tick 3: still closed → clear (spec: continue refuses).
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(
        !status.handling,
        "second refuse with chat closed must lift the handle"
    );
    assert!(!status.hold, "no latch after continue refuse");
}

#[test]
fn in_flight_dialog_continues_after_genie_despawns() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: talk-to the genie.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.actions, vec![0]);

    // Tick 2: the chat opens → continue.
    drv.menus.clear();
    drv.actions.clear();
    open_chat(&mut c);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.actions, vec![0]);

    // Tick 3: the genie despawns while the chat is still open. detect
    // now finds nothing, but the in-flight dialog must keep continuing.
    drv.menus.clear();
    drv.actions.clear();
    c.npc[0] = None;
    c.npc_count = 0;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.handling, "the open dialog keeps the handle");
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
        "a despawned genie must not stop the open chat"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn toggle_off_detects_but_never_sends() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings {
        random_events: false,
        ..ProfileSettings::default()
    };
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(drv.menus.is_empty(), "toggle off: no Talk-to");
    assert!(drv.actions.is_empty());
    assert_eq!(status.kind, Some(RandomKind::Dialog), "detect still fills");
    assert_eq!(status.name.as_deref(), Some("genie"));
    assert!(status.ours);
    assert!(!status.toggle);
    assert!(!status.handling);
    assert!(!status.hold);
}

#[test]
fn wrong_talk_bins_npc_slot_and_other_slots_still_talk() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: talk-to the genie.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.actions, vec![0]);

    // Tick 2: the chat rejects the talk → the slot is binned, and the
    // guardian neither continues nor re-talks it.
    drv.menus.clear();
    drv.actions.clear();
    c.add_chat(0, "It's not here for you.", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert!(drv.actions.is_empty(), "wrong talk must not keep handling");
    assert!(status.cooldown, "the rejected slot is in the 45s bin");

    // Tick 3: the binned slot is skipped by detect — no send.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert!(drv.actions.is_empty());
    assert_eq!(status.kind, None);

    // A second genie on another slot is still talk-to-able.
    plant_npc(&mut c, 1, "Genie", -1, Some("Greetings Test!"));
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 3_000, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 1, 0, 0)],
        "an un-binned slot still gets Talk-to"
    );
    assert_eq!(drv.actions, vec![0]);
}

// --- Task 5: trapped-kind hold, lamp, and the on_random knock ---

#[test]
fn maze_square_holds_without_any_send() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 45 * 64, 71 * 64);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Maze));
    assert!(status.hold, "a trapped kind holds the slot");
    assert!(drv.menus.is_empty(), "trapped kinds get no act");
    assert!(drv.actions.is_empty());

    // Toggle off: still detected, but never held.
    let settings = ProfileSettings {
        random_events: false,
        ..ProfileSettings::default()
    };
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Maze));
    assert!(!status.hold, "toggle off never holds");
}

#[test]
fn lamp_auto_off_detects_but_never_rubs() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_obj(&mut c, LAMP_OBJ);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings {
        lamp_auto: false,
        ..ProfileSettings::default()
    };
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Lamp));
    assert_eq!(status.name.as_deref(), Some("lamp"));
    assert!(drv.menus.is_empty(), "lamp_auto off: no Rub");
    assert!(drv.actions.is_empty());
    assert!(!status.hold, "leftover lamp with auto off does not hold");
    assert!(
        !status.ours,
        "lamp auto off must not publish ours — EventSignal.pending is hold OR ours"
    );
    assert!(!status.handling, "inert lamp must not latch the handler");
}

/// Plant obj `obj_id` into the inventory TYPE_INV iface (the shape
/// `detect` reads the inv view from). Two slots with one empty, so a
/// lost-gear take is not read as a full pack by default.
fn plant_inv_obj(c: &mut Client, obj_id: i32) {
    plant_inv_named(c, obj_id, None);
}

/// The held genie lamp (obj 2528) with the pack's own held ops
/// (`xplamp.obj`: `iop1=Rub`, the `opheld1` handler).
fn plant_inv_lamp(c: &mut Client) {
    plant_inv_ops(
        c,
        LAMP_OBJ,
        Some("Lamp"),
        [Some("Rub"), None, None, None, None],
    );
}

/// Like [`plant_inv_obj`] with a cache name.
fn plant_inv_named(c: &mut Client, obj_id: i32, name: Option<&str>) {
    plant_inv_ops(c, obj_id, name, [None, None, None, None, None]);
}

/// Like [`plant_inv_obj`] with a cache name and cache held ops.
fn plant_inv_ops(c: &mut Client, obj_id: i32, name: Option<&str>, iop: [Option<&str>; 5]) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.objs.len() <= obj_id as usize {
            cache.objs.push(client::config::ObjType::default());
        }
        cache.objs[obj_id as usize] = client::config::ObjType {
            id: obj_id,
            name: name.map(str::to_string).unwrap_or_default(),
            iop: iop.map(|op| op.map(str::to_string)),
            ..Default::default()
        };
    }
    c.side_icon[3] = 300;
    c.set_iface(
        300,
        IfType {
            id: 300,
            layer_id: 300,
            children: Some(vec![301]),
            ..Default::default()
        },
    );
    c.set_iface(
        301,
        IfType {
            id: 301,
            layer_id: 300,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![obj_id + 1, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );
    c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
}

/// Empty the inventory TYPE_INV iface (the lamp-consumed shape).
fn clear_inv(c: &mut Client) {
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![0, 0]),
            link_obj_number: Some(vec![0, 0]),
            ..Default::default()
        },
    );
    c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
}

/// A wall loc of cache `name` at scene (`scene_x`, `scene_z`) (the
/// hazard-loc shape).
fn plant_loc(c: &mut Client, id: i32, name: &str, scene_x: i32, scene_z: i32) {
    plant_loc_with_op(c, id, name, scene_x, scene_z, None);
}

/// Like [`plant_loc`] with an optional first scene op (the growing
/// plant loc's `Pick`).
fn plant_loc_with_op(
    c: &mut Client,
    id: i32,
    name: &str,
    scene_x: i32,
    scene_z: i32,
    op: Option<&str>,
) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.locs.len() <= id as usize {
            cache.locs.push(LocType::default());
        }
        cache.locs[id as usize] = LocType {
            id,
            name: name.to_string(),
            op: op.map(|o| vec![Some(o.to_string())]).unwrap_or_default(),
            ..Default::default()
        };
    }
    let typecode = 0x4000_0000 + (id << 14) + scene_x + (scene_z << 7);
    c.world
        .set_wall(0, scene_x, scene_z, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
}

/// Open the genie-lamp skill IF (xplamp 2808) with skill buttons
/// 2812..2830 and confirm 2831.
fn open_lamp(c: &mut Client) {
    let children: Vec<i32> = (LAMP_IF_FIRST..=LAMP_IF_CONFIRM).collect();
    c.set_iface(
        LAMP_IF_ROOT as usize,
        IfType {
            id: LAMP_IF_ROOT,
            layer_id: LAMP_IF_ROOT,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(children.clone()),
            ..Default::default()
        },
    );
    for com in LAMP_IF_FIRST..=LAMP_IF_CONFIRM {
        c.set_iface(
            com as usize,
            IfType {
                id: com,
                layer_id: LAMP_IF_ROOT,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            com as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                ..Default::default()
            },
        );
    }
    c.main_modal_id = LAMP_IF_ROOT;
    c.gens.iface += 1;
}

/// Close the main modal (the `xplamp_confirm` `if_close`).
fn close_main_modal(c: &mut Client) {
    c.main_modal_id = -1;
    c.gens.iface += 1;
}

/// Close the chat modal (the drained award `mesbox`).
fn close_chat(c: &mut Client) {
    c.chat_modal_id = -1;
    c.gens.iface += 1;
}

/// Write one skill stat (client index) and publish it — the host reads
/// `StatView` through the stat generation.
fn plant_stat(c: &mut Client, index: usize, xp: i32, base: i32) {
    c.stat_xp[index] = xp;
    c.stat_base_level[index] = base;
    c.stat_effective_level[index] = base;
    c.gens.stat += 1;
}

#[test]
fn on_random_knocks_once_per_event_edge() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    let knocks = std::sync::Arc::new(std::sync::Mutex::new(0usize));

    // Tick 1: the rising edge knocks the script.
    tick_at(&mut c, &mut snap);
    {
        let mut knock = |_: &DetectedRandom| {
            *knocks.lock().unwrap() += 1;
            RandomClaim::Handle
        };
        let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
        assert_eq!(*knocks.lock().unwrap(), 1, "one knock on the edge");
        assert_eq!(status.claim, RandomClaim::Handle);
        assert!(!status.hold, "a Handle claim never holds");
        assert!(drv.menus.is_empty(), "a Handle claim blocks Talk-to");
        assert!(drv.actions.is_empty());
    }

    // Tick 2: the same event persists → no second knock, the claim
    // sticks.
    tick_at(&mut c, &mut snap);
    {
        let mut knock = |_: &DetectedRandom| {
            *knocks.lock().unwrap() += 1;
            RandomClaim::Handle
        };
        let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
        assert_eq!(
            *knocks.lock().unwrap(),
            1,
            "a persisting event must not re-knock"
        );
        assert_eq!(status.claim, RandomClaim::Handle);
    }

    // Tick 3: the event vanishes → the claim resets to Host.
    c.npc[0] = None;
    c.npc_count = 0;
    tick_at(&mut c, &mut snap);
    {
        let mut knock = |_: &DetectedRandom| {
            *knocks.lock().unwrap() += 1;
            RandomClaim::Handle
        };
        let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
        assert_eq!(status.kind, None);
        assert_eq!(
            status.claim,
            RandomClaim::Host,
            "a vanished event hands the claim back to the host"
        );
        assert_eq!(*knocks.lock().unwrap(), 1, "no event → no knock");
    }
}

#[test]
fn no_knock_source_keeps_host_claim_and_talks() {
    // No knock supplied (host-owned slot): the claim stays Host and a
    // dialog event still talks.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.menus, vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)]);
    assert_eq!(drv.actions, vec![0]);
    assert!(status.handling);
    assert!(status.hold);
}

// --- Task 9: range WalkTo + the remaining act kinds ---

#[test]
fn flee_candidates_rings_are_farthest_first() {
    let tiles = flee_candidates((10, 20));
    assert_eq!(tiles.len(), 40, "8 compass points x 5 rings");
    assert_eq!(tiles[0], (10, 8), "north of the dist-12 ring first");
    assert_eq!(tiles[7], (-2, 8), "north-west closes the dist-12 ring");
    assert_eq!(tiles[8], (10, 10), "the dist-10 ring comes next");
    assert_eq!(tiles[39], (6, 16), "north-west of the dist-4 ring last");
    assert!(
        tiles[0..8]
            .iter()
            .all(|(x, z)| cheb((10, 20), (*x, *z)) == 12),
        "every first-ring tile is chebyshev 12 from the threat"
    );
}

#[test]
fn out_of_range_dialog_walks_to_npc_before_talk_to() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    // Move the genie to tile (3, 0): out of Talk-to range.
    c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
    c.npc[0].as_mut().expect("planted").entity.z = 64;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Dialog));
    assert_eq!(
        drv.walks,
        vec![(3, 0)],
        "an out-of-range Talk-to arms a walk to the npc tile"
    );
    assert!(drv.menus.is_empty(), "no Talk-to while out of range");
    assert!(drv.actions.is_empty());
    assert!(status.hold, "the range walk holds the slot");
}

#[test]
fn evade_swarm_holds_flees_and_walks_back_after_despawn() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_positive_hit(&mut c);
    plant_npc(&mut c, 0, "Swarm", 32768, None);
    c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
    c.npc[0].as_mut().expect("planted").entity.z = 64;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: the swarm targets us → flee away, hold the slot.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Evade));
    assert!(status.ours);
    assert!(status.hold, "an in-flight flee holds the slot");
    assert_eq!(
        drv.walks,
        vec![(3, -12)],
        "the first flee candidate is the farthest ring, north of the threat"
    );

    // Tick 2: the swarm despawned → the hold lifts and the guardian
    // walks back toward the pre-flee tile.
    drv.walks.clear();
    c.npc[0] = None;
    c.npc_count = 0;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold, "the hold lifts once the threat is gone");
    assert_eq!(drv.walks, vec![(0, 0)], "walk back to the pre-flee tile");
}

#[test]
fn lamp_auto_redeems_and_drains_the_award_dialogue() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_lamp(&mut c);
    // The chosen skill's baseline: strength (client stat index 2) at
    // 1,000 xp / base 7, as the Confirm tick reads it.
    plant_stat(&mut c, 2, 1_000, 7);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default(); // lamp_auto on, skill strength
    let mut snap = GameSnapshot::new();

    // Tick 1: Rub — the pack's own held op 1 (`xplamp.obj` `iop1`,
    // the `opheld1` handler that opens the skill IF).
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Lamp));
    assert!(status.hold, "the redemption holds while it is in flight");
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_HELD1, LAMP_OBJ, 0, 301)],
        "Rub is the lamp's 1st held op"
    );

    // Tick 2: the IF is not open yet → no second Rub.
    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold);
    assert!(drv.menus.is_empty(), "pending IF: no second Rub");

    // Tick 3: the skill IF (2808) opens → IF_BUTTON strength (2813).
    drv.menus.clear();
    drv.actions.clear();
    open_lamp(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold);
    assert_eq!(drv.menus.len(), 1, "one skill button press");
    assert_eq!(drv.menus[0].1, MiniMenuAction::IF_BUTTON);
    assert_eq!(drv.menus[0].4, 2813, "strength is xplamp button 2813");

    // Tick 4: confirm 2831, still no Rub.
    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 2831)],
        "next tick presses confirm"
    );

    // Tick 5: `xplamp_confirm` ran — the IF closed, the lamp is
    // consumed and the skill advanced (`stat_advance`) — but the award
    // `mesbox` is open. The lamp leaving the pack is not completion:
    // drain the award page and keep holding.
    drv.menus.clear();
    drv.actions.clear();
    close_main_modal(&mut c);
    clear_inv(&mut c);
    open_chat(&mut c);
    plant_stat(&mut c, 2, 1_070, 7);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold, "the award dialogue still holds the slot");
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
        "the award page is clicked through, not left open"
    );
    assert_eq!(drv.actions, vec![0]);

    // Tick 6: the award dialogue closed and the reward observed → the
    // redemption is real and the hold lifts.
    drv.menus.clear();
    drv.actions.clear();
    close_chat(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold, "a redeemed lamp releases the slot");
    assert!(drv.menus.is_empty(), "no replay after completion");
    assert!(drv.actions.is_empty());
}

#[test]
fn lamp_redeem_without_the_reward_gives_up_bounded() {
    // The lamp is consumed and the IF closed, but the reward never
    // lands and no award dialogue is up: the hold must not sit on a
    // stale "the lamp left the pack" read forever.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_lamp(&mut c);
    plant_stat(&mut c, 2, 1_000, 7);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // Rub
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // waiting for the IF
    open_lamp(&mut c);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // skill button
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None); // confirm
    assert!(status.hold);

    // The lamp went away with the IF, but no stat packet and no award
    // dialogue: held for the bounded wait, then released.
    close_main_modal(&mut c);
    clear_inv(&mut c);
    let mut held_ticks = 0;
    let mut released = false;
    for _ in 0..=MAX_LAMP_WAIT {
        drv.menus.clear();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(drv.menus.is_empty(), "nothing is replayed after Confirm");
        if status.hold {
            held_ticks += 1;
        } else {
            released = true;
            break;
        }
    }
    assert!(released, "a reward that never lands must not hold forever");
    assert_eq!(held_ticks, MAX_LAMP_WAIT as i32, "the give-up is bounded");
}

#[test]
fn lamp_confirm_refused_stalls_without_replaying() {
    // `xplamp_confirm` refuses when no skill is selected
    // (`%xplamp` none: members-only or quest-locked picks, an unknown
    // vault skill): the IF stays open and the lamp stays held. The
    // host must not replay Confirm, re-Rub or hold the script.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_lamp(&mut c);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // Rub
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // waiting for the IF
    open_lamp(&mut c);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // skill button
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None); // confirm
    assert!(status.hold);

    // The IF stays open with the lamp still in the pack: bounded wait,
    // then the stall releases the slot without a second Confirm/Rub.
    let mut released = false;
    for _ in 0..=MAX_LAMP_WAIT {
        drv.menus.clear();
        drv.actions.clear();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(
            drv.menus.is_empty(),
            "a refused Confirm is never replayed (and no fresh Rub)"
        );
        if !status.hold {
            released = true;
            assert_eq!(
                status.kind,
                Some(RandomKind::Lamp),
                "the stalled lamp is still detected for the status row"
            );
            assert!(
                !status.ours,
                "a stalled lamp must not publish ours (EventSignal.pending)"
            );
            break;
        }
    }
    assert!(released, "a refused redemption must not hold the script");

    // The stall clears once the lamp is gone: a later lamp redeems.
    clear_inv(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold);
}

#[test]
fn lamp_unknown_skill_stalls_instead_of_holding() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_lamp(&mut c);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings {
        lamp_skill: "not a skill".into(),
        ..ProfileSettings::default()
    };
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // Rub
    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None); // waiting for the IF
    open_lamp(&mut c);
    let mut released = false;
    for _ in 0..=MAX_LAMP_WAIT {
        drv.menus.clear();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(drv.menus.is_empty(), "an unknown skill is never clicked");
        if !status.hold {
            released = true;
            break;
        }
    }
    assert!(released, "an unclickable skill must not hold the script");
}

#[test]
fn lamp_skill_map_matches_the_xplamp_buttons() {
    // The `xplamp.if` component order (identical in the 274 and 289
    // packs), as rs2b0t's `LAMP_IF.skills` maps it.
    let expected = [
        ("attack", 2812),
        ("strength", 2813),
        ("ranged", 2814),
        ("magic", 2815),
        ("defence", 2816),
        ("hitpoints", 2817),
        ("prayer", 2818),
        ("agility", 2819),
        ("herblore", 2820),
        ("thieving", 2821),
        ("crafting", 2822),
        ("runecraft", 2823),
        ("mining", 2824),
        ("smithing", 2825),
        ("fishing", 2826),
        ("cooking", 2827),
        ("firemaking", 2828),
        ("woodcutting", 2829),
        ("fletching", 2830),
    ];
    for (skill, button) in expected {
        assert_eq!(
            lamp_skill_button(skill),
            Some(button),
            "{skill} is xplamp button {button}"
        );
    }
    assert_eq!(
        lamp_skill_button(" Agility "),
        Some(2819),
        "the setting is trimmed and case-insensitive"
    );
    assert_eq!(lamp_skill_button("slayer"), None, "no slayer button");
    assert_eq!(lamp_skill_button(""), None);
}

#[test]
fn foreign_strange_plant_is_not_picked_held_or_chased() {
    // Another player's plant (its overhead names them): no Pick, no
    // walk, no hold. The slot must keep running the script.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, Some("Pick Bob!"), "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None, "a foreign plant is not our event");
    assert!(!status.ours);
    assert!(!status.hold, "a foreign plant must not pause the script");
    assert!(drv.menus.is_empty(), "no Pick for another player's plant");
    assert!(drv.actions.is_empty());
    assert!(drv.walks.is_empty(), "and no chase");
}

#[test]
fn unknown_strange_plant_is_not_picked_held_or_chased() {
    // A growing seed carries no ownership tell of its own (no facing,
    // no overhead — `macro_event_triffid_spawn` only animates it), so
    // an out-of-reach unidentified plant must not drive the host.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    // Out of reach, so a chase would be visible.
    c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
    c.npc[0].as_mut().expect("planted").entity.z = 64;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None, "unknown ownership is not ours");
    assert!(!status.hold);
    assert!(drv.menus.is_empty());
    assert!(drv.walks.is_empty(), "no chase without ownership evidence");
}

#[test]
fn adjacent_featureless_foreign_plant_is_probed_once_then_binned() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_eq!(status.kind, Some(RandomKind::Pick));
    assert!(!status.ours, "the probe is not ownership evidence");
    assert!(status.hold, "one in-flight probe may serialize the slot");
    assert_eq!(drv.menus, vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)]);
    assert_eq!(drv.actions, vec![0]);
    assert!(drv.walks.is_empty());

    drv.menus.clear();
    drv.actions.clear();
    c.add_chat(0, "It's not here for you.", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_eq!(status.kind, None);
    assert!(!status.ours);
    assert!(!status.hold, "foreign response releases immediately");
    assert!(drv.menus.is_empty());
    assert!(drv.actions.is_empty());

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 100_000, None);
    assert_eq!(status.kind, None, "the exact foreign actor stays binned");
    assert!(!status.hold);
    assert!(drv.menus.is_empty(), "foreign actor is never re-probed");
    assert!(drv.actions.is_empty());
}

#[test]
fn ignored_featureless_plant_does_not_shadow_a_second_adjacent_actor() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    plant_npc_with_op(&mut c, 1, "Strange plant", -1, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
        "the first actor receives the first probe"
    );

    drv.menus.clear();
    drv.actions.clear();
    c.add_chat(0, "It's not here for you.", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 1, 0, 0)],
        "the exact rejected actor is skipped, not the second actor"
    );
    assert!(!status.ours);
    assert!(status.hold);

    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 2_500, None);
    assert!(drv.menus.is_empty(), "the second probe is not replayed");
    assert!(drv.actions.is_empty());
}

#[test]
fn disappeared_awaited_actor_does_not_poison_another_adjacent_actor() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    plant_npc_with_op(&mut c, 1, "Strange plant", -1, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_eq!(drv.menus[0].2, 0);

    drv.menus.clear();
    drv.actions.clear();
    c.npc[0] = None;
    c.npc_ids[0] = 1;
    c.npc_count = 1;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 1, 0, 0)],
        "clean disappearance leaves the different actor probeable"
    );
    assert!(!status.ours);
    assert!(status.hold);
    assert!(drv.walks.is_empty());
}

#[test]
fn generic_talk_failure_does_not_reject_awaited_or_authenticated_plant() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    drv.menus.clear();
    drv.actions.clear();

    c.add_chat(0, "Someone else is trying to talk to you.", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert!(!status.ours);
    assert!(status.hold, "the awaited probe remains active");
    assert!(drv.actions.is_empty());

    c.add_chat(0, "The fruit isn't ready to be picked yet...", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 3_000, None);
    assert!(status.ours);
    assert!(status.hold);

    c.add_chat(0, "Someone else is trying to talk to you.", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 4_000, None);
    assert!(status.ours);
    assert!(
        status.hold,
        "authenticated ownership survives generic talk failure"
    );
    assert!(drv.actions.is_empty());

    c.add_chat(0, "It's not here for you.", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 5_000, None);
    assert_eq!(status.kind, None);
    assert!(!status.ours);
    assert!(
        !status.hold,
        "canonical plant rejection releases immediately"
    );
    assert!(drv.actions.is_empty());
}

#[test]
fn adjacent_owned_growing_plant_authenticates_drains_and_retries_paced() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_eq!(drv.actions, vec![0], "the first Pick is the probe");

    drv.menus.clear();
    drv.actions.clear();
    c.add_chat(0, "The fruit isn't ready to be picked yet...", "");
    open_chat(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert!(status.ours, "the server response authenticates this actor");
    assert!(status.hold);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
        "the growing response is drained before another Pick"
    );
    assert_eq!(drv.actions, vec![0]);

    drv.menus.clear();
    drv.actions.clear();
    close_chat(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_500, None);
    assert!(status.ours);
    assert!(status.hold);
    assert!(drv.actions.is_empty(), "the retry interval is paced");

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 5_000, None);
    assert!(status.ours);
    assert!(status.hold);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
        "the authenticated actor is eventually picked again"
    );
    assert_eq!(drv.actions, vec![0]);

    drv.menus.clear();
    drv.actions.clear();
    c.npc[0] = None;
    c.npc_count = 0;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 10_000, None);
    assert_eq!(status.kind, None);
    assert!(!status.ours);
    assert!(!status.hold, "actor disappearance releases");
    assert!(drv.actions.is_empty());
    assert!(drv.walks.is_empty());
}

#[test]
fn featureless_ready_owned_plant_completes_on_first_pick_without_replay() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_eq!(drv.actions, vec![0]);

    drv.menus.clear();
    drv.actions.clear();
    c.npc[0] = None;
    c.npc_count = 0;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold);
    assert!(drv.actions.is_empty());

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 3_000, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold);
    assert!(drv.actions.is_empty(), "completion is not replayed");
}

#[test]
fn featureless_plant_probe_and_authenticated_wait_time_out_cleanly() {
    fn fixture() -> (Client, Guardian, FakeDriver, ProfileSettings, GameSnapshot) {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
        (
            c,
            Guardian::new(),
            FakeDriver::default(),
            ProfileSettings::default(),
            GameSnapshot::new(),
        )
    }

    let (mut c, mut g, mut drv, settings, mut snap) = fixture();
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    let status = g.tick(
        &mut drv,
        &snap,
        &settings,
        1_000 + PLANT_PROBE_TIMEOUT_MS,
        None,
    );
    assert_eq!(status.kind, None);
    assert!(!status.ours);
    assert!(!status.hold);
    assert!(drv.actions.is_empty(), "an unanswered probe is not retried");

    let (mut c, mut g, mut drv, settings, mut snap) = fixture();
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    drv.menus.clear();
    drv.actions.clear();
    c.add_chat(0, "The fruit isn't ready to be picked yet...", "");
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert!(status.ours);
    assert!(status.hold);

    tick_at(&mut c, &mut snap);
    let status = g.tick(
        &mut drv,
        &snap,
        &settings,
        2_000 + PLANT_AUTH_TIMEOUT_MS,
        None,
    );
    assert_eq!(status.kind, None);
    assert!(!status.ours, "timeout clears authenticated evidence");
    assert!(!status.hold);
    assert!(drv.actions.is_empty());
    assert!(drv.walks.is_empty());
}

#[test]
fn aggressive_strange_plant_is_not_picked_or_chased() {
    // The aggressive type offers only `Attack` (`antimacro.npc`
    // `macro_triffidseed_angry`: `op2=Attack`, no Pick), so even with
    // ownership evidence there is nothing to pick — and no chase.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, Some("Pick Test!"), "Attack");
    c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
    c.npc[0].as_mut().expect("planted").entity.z = 64;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None, "an attackable plant is not a pick");
    assert!(!status.hold);
    assert!(drv.menus.is_empty());
    assert!(drv.walks.is_empty(), "an aggressive plant is not chased");

    // The same attackable type without any ownership tell either.
    c.npc[0].as_mut().expect("planted").entity.chat_message = None;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold);
    assert!(drv.menus.is_empty());
    assert!(drv.walks.is_empty());
}

#[test]
fn featureless_strange_plant_does_not_starve_a_held_lamp() {
    // An unauthenticated adjacent plant must not shadow the lamp the
    // host must redeem.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
    plant_inv_lamp(&mut c);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Lamp), "the lamp is detected");
    assert!(status.ours);
    assert!(status.hold, "the lamp's own redemption holds");
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_HELD1, LAMP_OBJ, 0, 301)],
        "the lamp is rubbed, not the foreign plant picked"
    );
}

#[test]
fn authenticated_plant_drops_reused_id_op_and_angry_replacements() {
    for replacement_op in ["Take", "Attack"] {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc_with_op(&mut c, 0, "Strange plant", -1, None, "Pick");
        plant_npc_with_op(&mut c, 1, "Strange plant", -1, None, replacement_op);
        let replacement_type = c.npc[1].as_ref().expect("replacement").r#type;
        c.npc[1] = None;
        c.npc_count = 1;

        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 1_000, None);
        drv.menus.clear();
        drv.actions.clear();
        c.add_chat(0, "The fruit isn't ready to be picked yet...", "");
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
        assert!(status.ours, "fixture reaches authenticated state");
        assert!(status.hold);

        drv.menus.clear();
        drv.actions.clear();
        c.npc[0].as_mut().expect("plant").r#type = replacement_type;
        c.npc[0].as_mut().expect("plant").entity.x = 3 * 128 + 64;
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 20_000, None);
        assert!(
            !status.hold,
            "{replacement_op} replacement must lose authenticated hold"
        );
        assert!(
            drv.menus.is_empty(),
            "{replacement_op} replacement gets no action"
        );
        assert!(
            drv.actions.is_empty(),
            "{replacement_op} replacement gets no action"
        );
        assert!(
            drv.walks.is_empty(),
            "{replacement_op} replacement is not chased"
        );

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 21_000, None);
        assert!(!status.hold);
        assert!(drv.menus.is_empty(), "replacement remains suppressed");
        assert!(drv.actions.is_empty());
        assert!(drv.walks.is_empty());
    }
}

#[test]
fn plant_ownership_is_revalidated_when_the_slot_changes() {
    // Tick 1: an ours plant out of reach → walk to it. Tick 2: the
    // same slot now holds a foreign plant (a reused index) → the walk
    // and the pick stop.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, Some("Pick Test!"), "Pick");
    c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
    c.npc[0].as_mut().expect("planted").entity.z = 64;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Pick));
    assert!(status.ours);
    assert_eq!(drv.walks, vec![(3, 0)], "the ours plant is walked to");
    assert!(drv.menus.is_empty(), "no Pick while out of range");

    // The slot is reused by another player's plant.
    drv.walks.clear();
    c.npc[0].as_mut().expect("planted").entity.chat_message = Some("Pick Bob!".into());
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold, "the stale ownership must not hold the slot");
    assert!(drv.walks.is_empty(), "no chase for the reused slot");
    assert!(drv.menus.is_empty());

    // The foreign plant on the slot the host was walking to stays
    // ignored on the next tick too.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(drv.walks.is_empty());
}

#[test]
fn strange_plant_loc_is_not_picked() {
    // Both supported packs define the plant as an NPC (`antimacro.npc`)
    // and a loc carries no ownership tell, so a loc-shaped plant is
    // never our event.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_loc_with_op(&mut c, 510, "Strange plant", 0, 0, Some("Pick"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold);
    assert!(drv.menus.is_empty());
    assert!(drv.walks.is_empty());
}

#[test]
fn strange_box_pending_ticks_send_nothing_then_if_button() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_box(&mut c, 1);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: Open once.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
    );

    // Tick 2: IF not open yet → nothing (no second Open).
    drv.menus.clear();
    drv.actions.clear();
    c.main_modal_id = -1;
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(drv.menus.is_empty(), "pending cube IF: no second Open");
    assert!(drv.actions.is_empty());

    // Tick 3: cube IF opens → IF_BUTTON answer, no Open.
    drv.menus.clear();
    drv.actions.clear();
    open_cube(&mut c, "What colour is the Square?", [3069, 3065, 3075]);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6562)],
        "cube IF answers via button"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn guild_ground_gear_never_held_does_not_hold_or_take() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Fishing spot", -1, None);
    plant_inv_obj(&mut c, 999);
    plant_ground_obj(&mut c, 3, 0, 305, Some("Big fishing net"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 1_500, None);
    assert_ne!(status.kind, Some(RandomKind::LostGear));
    assert!(!status.hold, "guild display gear must not hold the slot");
    assert!(
        drv.menus.is_empty() && drv.actions.is_empty(),
        "never-held ground gear must not Take"
    );
}

fn lost_gear_harpoon_fixture() -> (Client, Guardian, FakeDriver, ProfileSettings, GameSnapshot) {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Fishing spot", -1, None);
    plant_inv_named(&mut c, 311, Some("Harpoon"));
    (
        c,
        Guardian::new(),
        FakeDriver::default(),
        ProfileSettings::default(),
        GameSnapshot::new(),
    )
}

fn drop_harpoon_to_ground(c: &mut Client) {
    clear_inv(c);
    plant_ground_obj(c, 3, 0, 311, Some("Harpoon"));
}

#[test]
fn held_then_lost_near_fishing_spot_takes_within_window() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    let held = g.tick(&mut drv, &snap, &settings, 1_000, None);
    assert_ne!(held.kind, Some(RandomKind::LostGear));
    assert!(drv.menus.is_empty());

    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    drv.menus.clear();
    drv.actions.clear();
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_eq!(status.kind, Some(RandomKind::LostGear));
    assert_eq!(status.name.as_deref(), Some("harpoon"));
    assert!(status.hold, "a proved loss holds while Take is in flight");
    assert_eq!(drv.menus.len(), 1);
    assert_eq!(
        drv.menus[0].1,
        MiniMenuAction::OP_OBJ3,
        "Take is the ground item's 3rd op"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn tools_dropped_away_from_fishing_spots_are_not_recovered() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 311, Some("Harpoon"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    drv.menus.clear();
    let status = g.tick(&mut drv, &snap, &settings, 600, None);
    assert_ne!(status.kind, Some(RandomKind::LostGear));
    assert!(!status.hold);
    assert!(drv.menus.is_empty());
}

#[test]
fn fishing_latch_covers_current_or_prior_tick_and_repeated_same_tick_scans() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);

    clear_npcs(&mut c);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    let lost = g.tick(&mut drv, &snap, &settings, 600, None);
    assert_eq!(lost.kind, Some(RandomKind::LostGear));

    drv.menus.clear();
    let same_tick = g.tick(&mut drv, &snap, &settings, 650, None);
    assert_eq!(
        same_tick.kind,
        Some(RandomKind::LostGear),
        "repeated same-tick scans stay latched"
    );

    tick_at(&mut c, &mut snap);
    let expired_latch = g.tick(&mut drv, &snap, &settings, 1_200, None);
    assert_ne!(
        expired_latch.kind,
        Some(RandomKind::LostGear),
        "latch is only current or prior game tick"
    );
}

#[test]
fn whirlpool_type_id_latches_the_same_as_a_named_fishing_spot() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_typed(&mut c, 0, 403, "Whirlpool", -1, None);
    plant_inv_named(&mut c, 311, Some("Harpoon"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 600, None);
    assert_eq!(status.kind, Some(RandomKind::LostGear));
    assert_eq!(drv.menus.len(), 1);
}

#[test]
fn expired_losses_are_ignored_after_the_90s_window() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    assert_eq!(
        g.tick(&mut drv, &snap, &settings, 1_000, None).kind,
        Some(RandomKind::LostGear)
    );
    drv.menus.clear();
    let expired = g.tick(&mut drv, &snap, &settings, 91_001, None);
    assert_ne!(expired.kind, Some(RandomKind::LostGear));
    assert!(drv.menus.is_empty() || expired.kind != Some(RandomKind::LostGear));
}

#[test]
fn bank_suppression_covers_the_open_and_the_following_update() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);

    plant_bank_open(&mut c);
    tick_at(&mut c, &mut snap);
    assert!(
        snap.bank_component_id() >= 0,
        "bank fact must be on the snapshot"
    );
    g.tick(&mut drv, &snap, &settings, 1_000, None);

    close_main_modal(&mut c);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    drv.menus.clear();
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_ne!(status.kind, Some(RandomKind::LostGear));
    assert!(drv.menus.is_empty());
}

#[test]
fn shop_suppression_covers_the_open_and_the_following_update() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);

    plant_shop_open(&mut c);
    tick_at(&mut c, &mut snap);
    assert!(snap.shop().open, "shop fact must be on the snapshot");
    g.tick(&mut drv, &snap, &settings, 1_000, None);

    close_main_modal(&mut c);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    drv.menus.clear();
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_ne!(status.kind, Some(RandomKind::LostGear));
    assert!(drv.menus.is_empty());
}

#[test]
fn reacquisition_clears_a_recorded_loss() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    assert_eq!(
        g.tick(&mut drv, &snap, &settings, 2_000, None).kind,
        Some(RandomKind::LostGear)
    );

    plant_inv_named(&mut c, 311, Some("Harpoon"));
    tick_at(&mut c, &mut snap);
    drv.menus.clear();
    let status = g.tick(&mut drv, &snap, &settings, 2_500, None);
    assert_ne!(status.kind, Some(RandomKind::LostGear));
}

#[test]
fn lifecycle_reset_and_slot_isolation_drop_foreign_loss() {
    let (mut c, mut g, mut drv, settings, mut snap) = lost_gear_harpoon_fixture();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    drop_harpoon_to_ground(&mut c);
    tick_at(&mut c, &mut snap);
    assert_eq!(
        g.tick(&mut drv, &snap, &settings, 2_000, None).kind,
        Some(RandomKind::LostGear)
    );

    let mut other = Guardian::new();
    drv.menus.clear();
    let isolated = other.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_ne!(
        isolated.kind,
        Some(RandomKind::LostGear),
        "a fresh slot has no ownership history"
    );
    assert!(drv.menus.is_empty());

    g = Guardian::new();
    let reset = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_ne!(reset.kind, Some(RandomKind::LostGear));
}

#[test]
fn inv_box_still_beats_a_proved_lost_net() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Fishing spot", -1, None);
    plant_inv_named(&mut c, 303, Some("Small fishing net"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 1_000, None);
    clear_inv(&mut c);
    plant_ground_obj(&mut c, 0, 0, 303, Some("Small fishing net"));
    plant_inv_box(&mut c, 1);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
    assert_eq!(status.kind, Some(RandomKind::Box));
    assert_eq!(status.name.as_deref(), Some("strange box"));
}

#[test]
fn lost_tool_handle_in_inv_uses_on_ground_head() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_ground_obj(&mut c, 0, 0, 503, Some("Bronze axe head"));
    plant_inv_named(&mut c, 504, Some("Bronze axe handle"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::LostTool));
    assert!(status.hold, "the reattach holds while in flight");
    assert_eq!(
        drv.menus.len(),
        2,
        "use-on arms select then the ground target"
    );
    assert_eq!(drv.menus[0].1, MiniMenuAction::USEHELD_START);
    assert_eq!(
        drv.menus[1].1,
        MiniMenuAction::USEHELD_ONOBJ,
        "the handle is used on the ground head"
    );
}

#[test]
fn lost_tool_without_ground_head_sends_no_reattach() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_named(&mut c, 504, Some("Bronze axe handle"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::LostTool));
    assert!(drv.menus.is_empty(), "no head: no fake reattach");
    assert!(drv.actions.is_empty());
    assert!(!status.hold, "nothing in flight: no hold");
}

#[test]
fn hazard_smoking_rock_underfoot_walks_off() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_loc(&mut c, 510, "Smoking rock", 0, 0);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Hazard));
    assert_eq!(status.name.as_deref(), Some("smoking rock"));
    assert!(status.hold, "stepping off holds the slot");
    assert_eq!(
        drv.walks,
        vec![(4, 0)],
        "standing on the hazard uses the frozen four-tile step"
    );
}

#[test]
fn hazard_next_to_player_flees_four_tiles_to_the_far_side() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 1, 0);
    plant_loc(&mut c, 510, "Smoking rock", 0, 0);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Hazard));
    assert!(status.hold);
    assert_eq!(
        drv.walks,
        vec![(5, 0)],
        "the four-tile candidate must be on the side away from the hazard"
    );
}

#[test]
fn our_strange_plant_gets_picked() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc_with_op(&mut c, 0, "Strange plant", -1, Some("Pick Test!"), "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Pick));
    assert!(status.ours);
    assert!(status.hold, "the pick holds while the plant is still there");
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
        "Pick is the plant's 1st op"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn our_strange_plant_facing_self_is_picked() {
    // The other ownership channel (`FACEENTITY`, the aggressive type
    // turning on its owner): a plant that faces us is ours.
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    // `face_entity` >= 32768 decodes as Player kind; + self_slot (0).
    plant_npc_with_op(&mut c, 0, "Strange plant", 32768, None, "Pick");
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Pick));
    assert!(status.ours);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
        "the ours plant is picked"
    );
}

// --- Task 10: mime + strange-box solvers ---

#[test]
fn mime_answer_maps_seq_to_button_index() {
    assert_eq!(mime_answer(860), Some(0), "emote_cry");
    assert_eq!(mime_answer(857), Some(1), "emote_think");
    assert_eq!(mime_answer(861), Some(2), "emote_laugh");
    assert_eq!(mime_answer(866), Some(3), "emote_dance");
    assert_eq!(mime_answer(1130), Some(4), "emote_climbing_rope");
    assert_eq!(mime_answer(1129), Some(5), "emote_mime_lean");
    assert_eq!(mime_answer(1128), Some(6), "emote_glass_wall");
    assert_eq!(mime_answer(1131), Some(7), "emote_glass_box");
}

#[test]
fn mime_answer_unknown_seq_is_none() {
    assert_eq!(mime_answer(858), None, "bow");
    assert_eq!(mime_answer(862), None, "cheer/idle");
    assert_eq!(mime_answer(0), None);
}

#[test]
fn solve_cube_answers_colour_question_by_shape_position() {
    assert_eq!(
        solve_cube(
            "What colour is the Square?",
            [Some(3069), Some(3065), Some(3075)]
        ),
        Some(0),
        "square-red sits in model slot 0"
    );
    assert_eq!(
        solve_cube(
            "What colour is the Star?",
            [Some(3063), Some(3085), Some(3071)]
        ),
        Some(1),
        "the star is model slot 1"
    );
    assert_eq!(
        solve_cube(
            "What colour is the Half Moon?",
            [Some(3089), Some(3063), Some(3079)]
        ),
        Some(0),
        "two-word shape still matches"
    );
    assert_eq!(
        solve_cube(
            "What colour is the Halfmoon?",
            [Some(3063), Some(3079), Some(3091)]
        ),
        Some(2),
        "the public rs2b2t server spells the shape as one word"
    );
}

#[test]
fn solve_cube_answers_shape_question_by_colour_position() {
    assert_eq!(
        solve_cube("Which shape is Blue?", [Some(3063), Some(3085), Some(3089)]),
        Some(2),
        "the blue part is model slot 2"
    );
}

#[test]
fn solve_cube_unknown_question_or_missing_model_is_none() {
    assert_eq!(solve_cube("??", [Some(3063), Some(3071), Some(3079)]), None);
    assert_eq!(
        solve_cube("What colour is the Star?", [None, Some(3063), Some(3071)]),
        None,
        "a missing model obj id is unsolvable"
    );
    assert_eq!(
        solve_cube(
            "What colour is the Potato?",
            [Some(3063), Some(3071), Some(3079)]
        ),
        None,
        "a shape not on the cube is unsolvable"
    );
}

/// Plant `qty` Strange boxes (obj 3062, `Open` as held op 1) into the
/// inv (one stacked row).
fn plant_inv_box(c: &mut Client, qty: i32) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.objs.len() <= STRANGE_BOX_OBJ as usize {
            cache.objs.push(client::config::ObjType::default());
        }
        cache.objs[STRANGE_BOX_OBJ as usize] = client::config::ObjType {
            id: STRANGE_BOX_OBJ,
            name: "Strange box".to_string(),
            iop: [Some("Open".into()), None, None, None, None],
            ..Default::default()
        };
    }
    c.side_icon[3] = 300;
    c.set_iface(
        300,
        IfType {
            id: 300,
            layer_id: 300,
            children: Some(vec![301]),
            ..Default::default()
        },
    );
    c.set_iface(
        301,
        IfType {
            id: 301,
            layer_id: 300,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![STRANGE_BOX_OBJ + 1, 0]),
            link_obj_number: Some(vec![qty, 0]),
            ..Default::default()
        },
    );
    c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
}

/// Open the mysterious-cube main modal (macro_cube 6554) with three
/// obj-model children, the question text and the three answer buttons.
fn open_cube(c: &mut Client, question: &str, models: [i32; 3]) {
    c.set_iface(
        6554,
        IfType {
            id: 6554,
            layer_id: 6554,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![6555, 6557, 6559, 6561, 6562, 6563, 6564]),
            ..Default::default()
        },
    );
    for (com, obj) in [(6555, models[0]), (6557, models[1]), (6559, models[2])] {
        c.set_iface(
            com as usize,
            IfType {
                id: com,
                layer_id: 6554,
                r#type: ComponentType::TYPE_MODEL,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            com as usize,
            IfTypeMut {
                model1_type: 4,
                model1_id: obj,
                ..Default::default()
            },
        );
    }
    c.set_iface(
        6561,
        IfType {
            id: 6561,
            layer_id: 6554,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        6561,
        IfTypeMut {
            text: question.to_string(),
            ..Default::default()
        },
    );
    for com in [6562, 6563, 6564] {
        c.set_iface(
            com as usize,
            IfType {
                id: com,
                layer_id: 6554,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            com as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                ..Default::default()
            },
        );
    }
    c.main_modal_id = 6554;
    c.gens.iface += 1;
}

#[test]
fn mime_square_answers_emote_and_holds_until_off_square() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 31 * 64, 74 * 64);
    plant_npc(&mut c, 0, "Mime", -1, None);
    c.npc[0].as_mut().expect("planted").entity.primary_anim = 860;
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: on the square, no emote chat yet → the guardian watches
    // the mime, sends nothing.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Mime));
    assert!(status.hold, "on the mime square the slot holds");
    assert!(drv.menus.is_empty(), "no press before the emote chat opens");
    assert!(drv.actions.is_empty());

    // Tick 2: the emote chat (6543) opens → IF_BUTTON on the button
    // for the last seen emote (cry → index 0 → 6546).
    drv.menus.clear();
    drv.actions.clear();
    c.chat_modal_id = 6543;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6546)],
        "emote index 0 maps to button 6546"
    );
    assert_eq!(drv.actions, vec![0]);

    // Tick 3: the chat stays open → one press per chat-open, no spam.
    drv.menus.clear();
    drv.actions.clear();
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold);
    assert!(
        drv.menus.is_empty(),
        "no repeat press while the chat stays up"
    );
    assert!(drv.actions.is_empty());

    // Tick 4: off the mime square → the hold lifts.
    drv.menus.clear();
    drv.actions.clear();
    plant_player(&mut c, "Test", 0, 0);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold, "off the mime square the hold lifts");
    assert!(drv.menus.is_empty());
    assert!(drv.actions.is_empty());
}

#[test]
fn strange_box_opens_solves_and_holds_until_consumed() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_box(&mut c, 1);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: no cube iface yet → Open the held box (held op 1).
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Box));
    assert_eq!(status.name.as_deref(), Some("strange box"));
    assert!(status.hold, "a held strange box holds the slot");
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
        "Open is the box's 1st held op"
    );
    assert_eq!(drv.actions, vec![0]);

    // Tick 2: the cube iface opens → the Square question answers the
    // square-red model (slot 0) via IF_BUTTON 6562.
    drv.menus.clear();
    drv.actions.clear();
    open_cube(&mut c, "What colour is the Square?", [3069, 3065, 3075]);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(status.hold);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6562)],
        "square-red is model slot 0 → answer button 1"
    );
    assert_eq!(drv.actions, vec![0]);

    // Tick 3: the box is consumed → the hold lifts.
    drv.menus.clear();
    drv.actions.clear();
    clear_inv(&mut c);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold, "no box: no hold");
    assert!(drv.menus.is_empty());
}

#[test]
fn strange_box_reopens_while_more_boxes_held() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_box(&mut c, 2);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: Open the first box.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
        "a held box opens"
    );

    // Tick 2: the cube iface answers the first box.
    drv.menus.clear();
    drv.actions.clear();
    open_cube(&mut c, "What colour is the Square?", [3069, 3065, 3075]);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.menus, vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6562)]);

    // Tick 3: one box consumed (2→1) and the modal closed → the next
    // held box opens (rs2b0t repeats while the inv holds one).
    drv.menus.clear();
    drv.actions.clear();
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![STRANGE_BOX_OBJ + 1, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );
    c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
    c.main_modal_id = -1;
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
        "a second held box reopens after the first was consumed"
    );
    assert_eq!(drv.actions, vec![0]);
}

#[test]
fn strange_box_unknown_question_sends_no_click() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_inv_box(&mut c, 1);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: open the box.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Box));
    assert!(status.hold);
    assert_eq!(drv.actions, vec![0]);

    // Tick 2: an unsolvable cube question → fail closed: no click,
    // the trapped hold stays while the box is held.
    drv.menus.clear();
    drv.actions.clear();
    open_cube(&mut c, "??", [3063, 3071, 3079]);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Box));
    assert!(status.hold, "an unsolvable cube keeps the trapped hold");
    assert!(drv.menus.is_empty(), "unknown question: no click");
    assert!(drv.actions.is_empty());
}

// --- Task 11: maze behavioural port ---

/// The NW spawn's route (door 0 (2890,4592), door 1 (2888,4587)).
fn nw_route() -> Vec<(i32, i32)> {
    maze::select_route(maze::graph(), maze::MAZE_SPAWNS[0]).expect("the NW spawn solves")
}

/// Plant the canonical maze's walls and closed door edges into the real
/// client's collision map. Maze world origin is scene (0,0).
fn maze_collision_client_at(player: (i32, i32)) -> Client {
    let mut c = new_client();
    c.map_build_base_x = 45 * 64;
    c.map_build_base_z = 71 * 64;
    c.local_player = Some(ClientPlayer::at(
        player.0 - c.map_build_base_x,
        player.1 - c.map_build_base_z,
    ));
    c.ingame = true;

    let graph = maze::graph();
    for &(ax, az, bx, bz) in graph.wall_edge.iter().chain(graph.door.keys()) {
        let x = ax - c.map_build_base_x;
        let z = az - c.map_build_base_z;
        let angle = if bx == ax + 1 {
            LocAngle::EAST
        } else if bz == az + 1 {
            LocAngle::NORTH
        } else {
            panic!("maze edge is not in canonical order: ({ax},{az})-({bx},{bz})");
        };
        c.collision[0].add_wall(x, z, LocShape::WALL_STRAIGHT, angle, false);
    }
    c
}

fn nw_maze_collision_client() -> Client {
    maze_collision_client_at(maze::MAZE_SPAWNS[0])
}

#[test]
fn maze_nw_spawn_uses_real_client_nearest_route_to_open_first_door() {
    let spawn = maze::MAZE_SPAWNS[0];
    let first_door = nw_route()[0];

    let mut exact = nw_maze_collision_client();
    assert!(
        !walk(&mut exact, first_door.0, first_door.1),
        "the closed edge makes the NW first-door tile exact-unreachable"
    );
    assert_eq!(
        exact.out.pos, 0,
        "an unreachable exact walk emits no packet"
    );

    let mut c = nw_maze_collision_client();
    let mut solve = maze::MazeSolve::new(nw_route());
    assert!(step_maze_phase(
        &mut solve,
        &mut c,
        &GameSnapshot::new(),
        spawn
    ));
    assert!(
        c.out.pos > 0,
        "the Maze approach must emit a nearest-route move"
    );
    assert_eq!(
        c.try_move_nearest, 1,
        "the real client must accept its nearest fallback"
    );
    assert_eq!(
        (c.route_x[0], c.route_z[0]),
        (11, 49),
        "nearest fallback lands on the reachable near side of the closed first door"
    );
    let approached = (
        c.map_build_base_x + c.route_x[0],
        c.map_build_base_z + c.route_z[0],
    );

    assert!(step_maze_phase(
        &mut solve,
        &mut c,
        &GameSnapshot::new(),
        approached
    ));
    assert_eq!(
        solve.phase,
        maze::MazePhase::OpenDoor { from: approached },
        "the near-side arrival opens the first door"
    );
}

#[test]
fn maze_spawn_walks_opens_and_advances_door_to_door() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 2891, 4597); // NW spawn
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Tick 1: the route solves from the observed tile (no send yet).
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Maze));
    assert!(status.hold, "on the maze square the slot holds");
    assert!(drv.menus.is_empty() && drv.walks.is_empty());

    // Tick 2: walk to door 0.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.walks, vec![(2890, 4592)], "walk toward door 0");

    // Adjacent to the door: oploc Open with the door's loc id.
    drv.walks.clear();
    plant_player(&mut c, "Test", 2890, 4593);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_LOC1, 3628, 2890, 4592)],
        "oploc Open is the door's OP_LOC1"
    );
    assert!(drv.walks.is_empty());

    // The open pushes the player through (>= 2 tiles): advance.
    drv.menus.clear();
    plant_player(&mut c, "Test", 2887, 4592);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(drv.menus.is_empty(), "the through tick just advances");
    assert!(drv.walks.is_empty());

    // Next tick: walk to door 1.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.walks, vec![(2888, 4587)], "walk toward door 1");
}

#[test]
fn maze_wrong_door_mesbox_is_continued_then_the_route_advances() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 2891, 4597);
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Solve, walk to door 0, open it.
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    plant_player(&mut c, "Test", 2890, 4593);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::OP_LOC1, 3628, 2890, 4592)]
    );

    // The door refuses: the wrong-door mesbox opens → continue drains
    // it (rs2b0t clearMesbox).
    drv.menus.clear();
    drv.walks.clear();
    open_chat(&mut c);
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(
        drv.menus,
        vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
        "the wrong-door mesbox is continued through"
    );

    // Chat closed: the refused door advances the route → walk to door 1.
    drv.menus.clear();
    c.chat_modal_id = -1;
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert!(drv.menus.is_empty(), "the refusal tick just advances");
    tick_at(&mut c, &mut snap);
    g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(drv.walks, vec![(2888, 4587)], "advance after the refusal");
}

#[test]
fn maze_walled_off_door_resyncs_then_gives_up_after_three() {
    let mut solve = maze::MazeSolve::new(nw_route());
    solve.next = 1; // door 0 already behind us
    let mut drv = FakeDriver::default();
    let snap = GameSnapshot::new(); // no chat

    // Door 1 never gets closer: WALK_LIMIT sends, then the walk is
    // stuck → one resync back through door 0.
    for _ in 0..maze::WALK_LIMIT {
        assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
    }
    drv.walks.clear();
    assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
    assert_eq!(solve.phase, maze::MazePhase::Resync);
    assert_eq!(solve.resyncs, 1);
    drv.walks.clear();
    assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
    assert_eq!(drv.walks, vec![(2890, 4592)], "resync walks to door 0");

    // Three resyncs are the ceiling: a fourth walled-off door gives
    // up the pass instead of stepping back again.
    solve.resyncs = maze::MAX_RESYNCS;
    solve.phase = maze::MazePhase::WalkDoor;
    solve.walk_from = None;
    solve.walk_sends = 0;
    drv.walks.clear();
    for _ in 0..maze::WALK_LIMIT {
        assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
    }
    assert!(
        !step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)),
        "resyncs capped: the pass gives up"
    );
}

#[test]
fn maze_chamber_route_exhausts_to_touch_without_reopening() {
    let mut solve = maze::MazeSolve::new(vec![maze::MAZE_SHRINE_DOOR]);
    solve.next = solve.doors.len();
    let mut drv = FakeDriver::default();
    let snap = GameSnapshot::new();
    let interior = (maze::MAZE_SHRINE_DOOR.0 + 1, maze::MAZE_SHRINE_DOOR.1);

    assert!(step_maze_phase(&mut solve, &mut drv, &snap, interior));
    assert_eq!(
        solve.phase,
        maze::MazePhase::Touch { pass: 0 },
        "a route that already opened the chamber proceeds directly to Touch"
    );
    assert!(
        drv.menus.is_empty(),
        "route exhaustion itself sends no action"
    );

    assert!(step_maze_phase(&mut solve, &mut drv, &snap, interior));
    assert_eq!(
        drv.menus,
        vec![(
            0,
            MiniMenuAction::OP_LOC1,
            maze::MAZE_SHRINE_LOC,
            maze::MAZE_SHRINE.0,
            maze::MAZE_SHRINE.1,
        )],
        "the next action is Touch, not a second chamber-door Open"
    );
}

#[test]
fn maze_route_without_chamber_last_still_opens_chamber() {
    let snap = GameSnapshot::new();
    let outside = maze::MAZE_SHRINE_DOOR;

    for doors in [vec![], vec![(2900, 4600)]] {
        let mut solve = maze::MazeSolve::new(doors);
        solve.next = solve.doors.len();
        let mut drv = FakeDriver::default();

        assert!(step_maze_phase(&mut solve, &mut drv, &snap, outside));
        assert_eq!(solve.phase, maze::MazePhase::ShrineDoor);
        assert!(drv.menus.is_empty());

        assert!(step_maze_phase(&mut solve, &mut drv, &snap, outside));
        assert_eq!(
            drv.menus,
            vec![(
                0,
                MiniMenuAction::OP_LOC1,
                maze::MAZE_DOOR_IDS[0],
                maze::MAZE_SHRINE_DOOR.0,
                maze::MAZE_SHRINE_DOOR.1,
            )],
            "an empty or non-chamber route preserves the chamber-door fallback"
        );
    }
}

#[test]
fn maze_closed_chamber_collision_blocks_outside_but_reaches_from_interior() {
    let outside = maze::MAZE_SHRINE_DOOR;
    let interior = (outside.0 + 1, outside.1);

    let mut blocked = maze_collision_client_at(outside);
    assert!(
        !walk(&mut blocked, maze::MAZE_SHRINE.0, maze::MAZE_SHRINE.1),
        "the closed chamber edge blocks shrine SW from outside"
    );
    assert_eq!(blocked.out.pos, 0, "a blocked walk emits no packet");

    let mut reachable = maze_collision_client_at(interior);
    assert!(
        walk(&mut reachable, maze::MAZE_SHRINE.0, maze::MAZE_SHRINE.1),
        "the shrine SW is reachable from the chamber interior"
    );
    assert!(
        reachable.out.pos > 0,
        "the reachable interior walk emits a packet"
    );
}

#[test]
fn maze_touches_the_shrine_and_the_hold_lifts_off_square() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 2911, 4576); // post-chamber-door tile
    let mut g = Guardian::new();
    let mut solve = maze::MazeSolve::new(vec![]);
    solve.phase = maze::MazePhase::Touch { pass: 0 };
    g.maze = Some(solve);
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();

    // Near the shrine on pass 0: Touch from where we stand.
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, Some(RandomKind::Maze));
    assert!(status.hold, "the trapped hold stays while touching");
    assert_eq!(
        drv.menus,
        vec![(
            0,
            MiniMenuAction::OP_LOC1,
            maze::MAZE_SHRINE_LOC,
            2911,
            4575
        )],
        "Touch is the shrine's OP_LOC1"
    );
    assert!(drv.walks.is_empty(), "near the shrine: no stand walk");

    // The shrine teleports the player off the square → the hold lifts.
    drv.menus.clear();
    plant_player(&mut c, "Test", 0, 0);
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.kind, None);
    assert!(!status.hold, "off the maze square the hold lifts");
}

#[test]
fn maze_touch_walks_preserve_adjacent_and_onto_completion_semantics() {
    let snap = GameSnapshot::new();

    let mut adjacent = maze::MazeSolve::new(vec![]);
    adjacent.phase = maze::MazePhase::Touch { pass: 1 };
    let mut adjacent_driver = FakeDriver::default();
    let adjacent_stand = maze::TOUCH_STANDS[1];
    assert!(step_maze_phase(
        &mut adjacent,
        &mut adjacent_driver,
        &snap,
        (adjacent_stand.0 - 1, adjacent_stand.1)
    ));
    assert!(
        adjacent_driver.walks.is_empty(),
        "odd touch passes complete adjacent to the stand"
    );
    assert_eq!(adjacent.phase, maze::MazePhase::TouchWait);

    let mut onto = maze::MazeSolve::new(vec![]);
    onto.phase = maze::MazePhase::Touch { pass: 2 };
    let mut onto_driver = FakeDriver::default();
    let onto_stand = maze::TOUCH_STANDS[2];
    assert!(step_maze_phase(
        &mut onto,
        &mut onto_driver,
        &snap,
        (onto_stand.0 - 1, onto_stand.1)
    ));
    assert_eq!(
        onto_driver.walks,
        vec![onto_stand],
        "even touch passes still require standing on the target"
    );
    assert_eq!(
        onto_driver.walk_nearest,
        vec![true],
        "canonical Maze walks use nearest routing even for onto passes"
    );
    assert_eq!(onto.phase, maze::MazePhase::Touch { pass: 2 });
}
