// Task 9: interact/settle kernel methods map to `doAction`/`tryMove`/`out`
// through a `Driver`, and the legal send table covers every `ClientProt`.
// The `Recorder` stub stands in for the real `Client` driver; two tests
// exercise the real `Client` driver end-to-end (same `/tmp` cache trick as
// `client/tests/gens.rs` — no network).

use api::interact::{
    answer_count, cheat, close_modal, create_interactions, interact, login, mainland_hop,
    offers_operation, op_loc, operation_of, press, seed_at, set_run, still_present, tele_args,
    walk, ActionSpec, Driver, Interactions, OpTarget, SendReason, SendResult, WireCommand,
    MAX_OPERATIONS, OFF_ISLAND_TELE, RUN_ORB_IFACE, RUN_ORB_OFF, SCENE_READY,
};
use api::obj_names::ItemDefView;
use api::prot::{LegalSend, LEGAL_SEND};
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, NpcView, WorldTile,
};
use client::client::{Client, ClientConfig, ClientNpc, ClientPlayer, MiniMenuAction};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::{LocType, NpcType, ObjType};
use client::dash3d::ClientObj;
use client::datastruct::LinkList;
use client::io::{ClientProt, Isaac, ServerProt};
use std::sync::Arc;

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    }
}

/// Every associated const on `ClientProt`, mirrored here so a missed or
/// extra `LEGAL_SEND` row fails the coverage test.
const ALL_CLIENT_PROTS: &[ClientProt] = &[
    ClientProt::NO_TIMEOUT,
    ClientProt::IDLE_TIMER,
    ClientProt::EVENT_MOUSE_CLICK,
    ClientProt::EVENT_MOUSE_MOVE,
    ClientProt::EVENT_APPLET_FOCUS,
    ClientProt::EVENT_CAMERA_POSITION,
    ClientProt::ANTICHEAT_OPLOGIC1,
    ClientProt::ANTICHEAT_OPLOGIC2,
    ClientProt::ANTICHEAT_OPLOGIC3,
    ClientProt::ANTICHEAT_OPLOGIC4,
    ClientProt::ANTICHEAT_OPLOGIC5,
    ClientProt::ANTICHEAT_OPLOGIC6,
    ClientProt::ANTICHEAT_OPLOGIC7,
    ClientProt::ANTICHEAT_OPLOGIC8,
    ClientProt::ANTICHEAT_OPLOGIC9,
    ClientProt::ANTICHEAT_CYCLELOGIC1,
    ClientProt::ANTICHEAT_CYCLELOGIC2,
    ClientProt::ANTICHEAT_CYCLELOGIC3,
    ClientProt::ANTICHEAT_CYCLELOGIC4,
    ClientProt::ANTICHEAT_CYCLELOGIC5,
    ClientProt::ANTICHEAT_CYCLELOGIC6,
    ClientProt::ANTICHEAT_CYCLELOGIC7,
    ClientProt::OPOBJ1,
    ClientProt::OPOBJ2,
    ClientProt::OPOBJ3,
    ClientProt::OPOBJ4,
    ClientProt::OPOBJ5,
    ClientProt::OPOBJT,
    ClientProt::OPOBJU,
    ClientProt::OPNPC1,
    ClientProt::OPNPC2,
    ClientProt::OPNPC3,
    ClientProt::OPNPC4,
    ClientProt::OPNPC5,
    ClientProt::OPNPCT,
    ClientProt::OPNPCU,
    ClientProt::OPLOC1,
    ClientProt::OPLOC2,
    ClientProt::OPLOC3,
    ClientProt::OPLOC4,
    ClientProt::OPLOC5,
    ClientProt::OPLOCT,
    ClientProt::OPLOCU,
    ClientProt::OPPLAYER1,
    ClientProt::OPPLAYER2,
    ClientProt::OPPLAYER3,
    ClientProt::OPPLAYER4,
    ClientProt::OPPLAYER5,
    ClientProt::OPPLAYERT,
    ClientProt::OPPLAYERU,
    ClientProt::OPHELD1,
    ClientProt::OPHELD2,
    ClientProt::OPHELD3,
    ClientProt::OPHELD4,
    ClientProt::OPHELD5,
    ClientProt::OPHELDT,
    ClientProt::OPHELDU,
    ClientProt::INV_BUTTON1,
    ClientProt::INV_BUTTON2,
    ClientProt::INV_BUTTON3,
    ClientProt::INV_BUTTON4,
    ClientProt::INV_BUTTON5,
    ClientProt::IF_BUTTON,
    ClientProt::RESUME_PAUSEBUTTON,
    ClientProt::CLOSE_MODAL,
    ClientProt::RESUME_P_COUNTDIALOG,
    ClientProt::TUT_CLICKSIDE,
    ClientProt::MAP_BUILD_COMPLETE,
    ClientProt::MOVE_OPCLICK,
    ClientProt::REPORT_ABUSE,
    ClientProt::MOVE_MINIMAPCLICK,
    ClientProt::INV_BUTTOND,
    ClientProt::IGNORELIST_DEL,
    ClientProt::IGNORELIST_ADD,
    ClientProt::IDK_SAVEDESIGN,
    ClientProt::CHAT_SETMODE,
    ClientProt::MESSAGE_PRIVATE,
    ClientProt::FRIENDLIST_DEL,
    ClientProt::FRIENDLIST_ADD,
    ClientProt::CLIENT_CHEAT,
    ClientProt::MESSAGE_PUBLIC,
    ClientProt::MOVE_GAMECLICK,
];

/// The outbound writes a driver receives, as recorded by the stub.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OutByte {
    Enc(i32),
    P1(i32),
    P2(i32),
    P4(i32),
    Jstr(String),
}

impl api::prot::Out for OutSink {
    fn p1_enc(&mut self, opcode: i32) {
        self.0.push(OutByte::Enc(opcode));
    }
    fn p1(&mut self, value: i32) {
        self.0.push(OutByte::P1(value));
    }
    fn p2(&mut self, value: i32) {
        self.0.push(OutByte::P2(value));
    }
    fn p4(&mut self, value: i32) {
        self.0.push(OutByte::P4(value));
    }
    fn pjstr(&mut self, s: &str) {
        self.0.push(OutByte::Jstr(s.to_string()));
    }
}

/// Outbound writes recorded by the stub driver.
#[derive(Default)]
struct OutSink(Vec<OutByte>);

/// Recording driver: captures every kernel call instead of sending.
#[derive(Default)]
struct Recorder {
    menus: Vec<(i32, i32, i32, i32, i32)>,
    actions: Vec<i32>,
    moves: Vec<WalkMove>,
    out: OutSink,
    logins: usize,
    route: Option<(i32, i32)>,
    base: (i32, i32),
    /// Packed scene typecode at the loc tile; `None` falls back to loc id.
    scene_typecode: Option<i32>,
    /// Tabs handed to `click_side_tab`.
    side_tabs: Vec<i32>,
}

/// One `try_move` call: `(src_x, src_z, dx, dz, forceapproach, ...)`.
type WalkMove = (i32, i32, i32, i32, bool, i32, i32, i32, i32, i32, i32);

impl Driver for Recorder {
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
        self.menus.push((slot, action, a, b, c));
    }

    fn do_action(&mut self, slot: i32) -> bool {
        self.actions.push(slot);
        true
    }

    fn try_move(
        &mut self,
        src_x: i32,
        src_z: i32,
        dx: i32,
        dz: i32,
        try_nearest: bool,
        loc_width: i32,
        loc_length: i32,
        loc_angle: i32,
        loc_shape: i32,
        forceapproach: i32,
        r#type: i32,
    ) -> bool {
        self.moves.push((
            src_x,
            src_z,
            dx,
            dz,
            try_nearest,
            loc_width,
            loc_length,
            loc_angle,
            loc_shape,
            forceapproach,
            r#type,
        ));
        true
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        self.route
    }

    fn build_base(&self) -> (i32, i32) {
        self.base
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        self.scene_typecode
    }

    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        self.logins += 1;
        true
    }

    fn click_side_tab(&mut self, tab: i32) -> bool {
        self.side_tabs.push(tab);
        true
    }
}

/// `set_run` presses the run orb through the doAction path: menu slot 0 is
/// an IF_BUTTON on `RUN_ORB_IFACE`, and the driver accepts the dispatch.
#[test]
fn set_run_presses_run_orb_via_if_button_do_action() {
    let mut r = Recorder::default();
    assert!(set_run(&mut r, true));
    assert_eq!(r.actions, vec![0]);
    assert_eq!(
        r.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, RUN_ORB_IFACE)]
    );
}

/// `set_run(false)` presses the run-off orb (152), not the on orb.
#[test]
fn set_run_false_presses_run_off_orb() {
    let mut r = Recorder::default();
    assert!(set_run(&mut r, false));
    assert_eq!(
        r.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, RUN_ORB_OFF)]
    );
}

/// `press` dispatches an IF_BUTTON on the given child through slot 0.
#[test]
fn press_dispatches_if_button_on_child() {
    let mut r = Recorder::default();
    assert!(press(&mut r, 234));
    assert_eq!(r.actions, vec![0]);
    assert_eq!(r.menus, vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 234)]);
}

/// `walk` routes from the driver's local tile through `try_move` (type 0,
/// plain ground walk), not a raw packet inject.
#[test]
fn walk_calls_try_move_from_local_route() {
    let mut r = Recorder {
        route: Some((5, 7)),
        ..Recorder::default()
    };
    assert!(walk(&mut r, 30, 40));
    assert_eq!(r.moves, vec![(5, 7, 30, 40, false, 0, 0, 0, 0, 0, 0)]);
    assert!(r.out.0.is_empty());
}

/// `walk` without a local player route is not sent.
#[test]
fn walk_without_local_route_is_not_sent() {
    let mut r = Recorder::default();
    assert!(!walk(&mut r, 30, 40));
    assert!(r.moves.is_empty());
}

/// `walk` takes absolute world tiles: with a non-zero build base the
/// tryMove src/dest are translated into the client's scene space (the
/// route head is already scene-relative, so only the dest shifts).
#[test]
fn walk_translates_absolute_dest_into_scene_coords() {
    let mut r = Recorder {
        route: Some((52, 52)),
        base: (3200, 3200),
        ..Recorder::default()
    };
    assert!(walk(&mut r, 3230, 3222));
    assert_eq!(r.moves, vec![(52, 52, 30, 22, false, 0, 0, 0, 0, 0, 0)]);
}

/// `interact` dispatches the already-prepared menu slot.
#[test]
fn interact_dispatches_prepared_slot() {
    let mut r = Recorder::default();
    assert!(interact(&mut r, 2));
    assert_eq!(r.actions, vec![2]);
    assert!(r.menus.is_empty());
}

/// `op_loc` interacts with a loc via OP_LOC1: menu slot 0 carries
/// (OP_LOC1, loc_id, x, z) and the driver dispatches it.
#[test]
fn op_loc_sets_menu_oploc1_then_do_action() {
    let mut r = Recorder::default();
    assert!(op_loc(&mut r, 2816, 3438, 1530));
    assert_eq!(r.actions, vec![0]);
    assert_eq!(
        r.menus,
        vec![(0, MiniMenuAction::OP_LOC1, 1530, 2816, 3438)]
    );
}

/// `op_loc` takes absolute loc tiles: with a non-zero build base the menu
/// carries scene coordinates, matching `Client.interact_with_loc`.
#[test]
fn op_loc_translates_absolute_loc_into_scene_coords() {
    let mut r = Recorder {
        base: (3200, 3200),
        ..Recorder::default()
    };
    assert!(op_loc(&mut r, 3230, 3222, 1530));
    assert_eq!(r.actions, vec![0]);
    assert_eq!(r.menus, vec![(0, MiniMenuAction::OP_LOC1, 1530, 30, 22)]);
}

/// Live `interact_with_loc` matches `a` to `wall.typecode` via `type_code2`.
/// When the driver has a typecode at the scene tile, that packed value is
/// menu param `a` (loc id in bits 14..28), not the bare loc id.
#[test]
fn op_loc_uses_scene_typecode_as_menu_param_a() {
    let typecode = (1530i32 & 0x7fff) << 14;
    let mut r = Recorder {
        base: (2752, 3392),
        scene_typecode: Some(typecode),
        ..Recorder::default()
    };
    assert!(op_loc(&mut r, 2816, 3438, 1530));
    assert_eq!(
        r.menus,
        vec![(0, MiniMenuAction::OP_LOC1, typecode, 64, 46)]
    );
}

/// `cheat` is CLIENT_CHEAT: enc opcode, size byte (cmd+nul), pjstr(cmd).
#[test]
fn cheat_writes_client_cheat_without_colon_prefix() {
    let mut r = Recorder::default();
    assert!(cheat(&mut r, "ping"));
    assert_eq!(
        r.out.0,
        vec![
            OutByte::Enc(ClientProt::CLIENT_CHEAT.id),
            OutByte::P1(5),
            OutByte::Jstr("ping".into()),
        ]
    );
}

/// `mainland_hop` queues tele + setvar tutorial 1000 (no relog).
#[test]
fn mainland_hop_queues_tele_and_tutorial_setvar() {
    let mut r = Recorder::default();
    mainland_hop(&mut r);
    let tele = format!("tele {OFF_ISLAND_TELE}");
    assert_eq!(
        r.out.0,
        vec![
            OutByte::Enc(ClientProt::CLIENT_CHEAT.id),
            OutByte::P1((tele.len() + 1) as i32),
            OutByte::Jstr(tele),
            OutByte::Enc(ClientProt::CLIENT_CHEAT.id),
            OutByte::P1(("setvar tutorial 1000".len() + 1) as i32),
            OutByte::Jstr("setvar tutorial 1000".into()),
        ]
    );
}

#[test]
fn tele_args_splits_world_tile_into_mapsquare() {
    assert_eq!(tele_args(0, 3220, 3218), "tele 0,50,50,20,18");
}

#[test]
fn seed_at_sends_tutorial_setvar_then_tele() {
    let mut r = Recorder::default();
    seed_at(&mut r, 0, 3220, 3218);
    let tele = tele_args(0, 3220, 3218);
    assert_eq!(
        r.out.0,
        vec![
            OutByte::Enc(ClientProt::CLIENT_CHEAT.id),
            OutByte::P1(("setvar tutorial 1000".len() + 1) as i32),
            OutByte::Jstr("setvar tutorial 1000".into()),
            OutByte::Enc(ClientProt::CLIENT_CHEAT.id),
            OutByte::P1((tele.len() + 1) as i32),
            OutByte::Jstr(tele),
        ]
    );
}

/// Real `Client` driver: cheat matches Java `::ping` CLIENT_CHEAT layout.
#[test]
fn client_driver_cheat_matches_java_client_cheat() {
    let mut c = Client::new(cfg());
    assert!(cheat(&mut c, "ping"));
    assert_eq!(c.out.data()[0] as i32, ClientProt::CLIENT_CHEAT.id & 0xff);
    assert_eq!(c.out.data()[1], 5);
    assert_eq!(&c.out.data()[2..7], b"ping\n");
}

/// `close_modal` writes the CLOSE_MODAL opcode through the ISAAC sink.
#[test]
fn close_modal_writes_close_modal_opcode() {
    let mut r = Recorder::default();
    assert!(close_modal(&mut r));
    assert_eq!(r.out.0, vec![OutByte::Enc(ClientProt::CLOSE_MODAL.id)]);
}

/// `answer_count` writes RESUME_P_COUNTDIALOG + the amount.
#[test]
fn answer_count_writes_count_dialog() {
    let mut r = Recorder::default();
    assert!(answer_count(&mut r, 1500));
    assert_eq!(
        r.out.0,
        vec![
            OutByte::Enc(ClientProt::RESUME_P_COUNTDIALOG.id),
            OutByte::P4(1500),
        ]
    );
}

/// `login` routes through the driver's login handshake.
#[test]
fn login_dispatches_driver_login() {
    let mut r = Recorder::default();
    assert!(login(&mut r, "bot", "hunter2", false));
    assert_eq!(r.logins, 1);
}

/// The real `Client` driver: `set_run` reaches the client's `doAction`
/// IF_BUTTON arm and writes the opcode + child through `out` (no raw
/// opcode inject that skips ISAAC).
#[test]
fn client_driver_set_run_writes_if_button_payload() {
    let mut c = Client::new(cfg());
    c.ingame = true;
    assert!(set_run(&mut c, true));
    assert_eq!(
        c.out.data()[..3],
        [
            ClientProt::IF_BUTTON.id as u8,
            (RUN_ORB_IFACE >> 8) as u8,
            RUN_ORB_IFACE as u8,
        ]
    );
    assert_eq!(c.out.pos, 3);
}

/// The real `Client` driver: `walk` writes MOVE_GAMECLICK (type 0) via
/// `tryMove`, ISAAC-encrypted like the client's own walk.
#[test]
fn client_driver_walk_writes_move_gameclick() {
    let mut c = Client::new(cfg());
    c.ingame = true;
    c.local_player = Some(ClientPlayer::at(5, 5));
    c.out.random = Some(Isaac::new(&[1, 2, 3, 4]));
    assert!(walk(&mut c, 10, 10));
    // (207 + -621246914) & 0xff = 13, then size + ctrl + absolute tile.
    assert_eq!(&c.out.data()[..7], &[13, 5, 0, 0, 10, 0, 10]);
    assert_eq!(c.out.pos, 7);
}

/// The real `Client` driver: the close dispatch runs `Client::close_modal`
/// (the doAction CLOSE_BUTTON arm), which clears the local modal ids and
/// writes CLOSE_MODAL through `out`.
#[test]
fn client_driver_close_button_clears_local_modal_state() {
    let mut c = Client::new(cfg());
    c.ingame = true;
    c.main_modal_id = 100;
    c.side_modal_id = 200;
    c.chat_modal_id = 300;
    c.set_menu(0, MiniMenuAction::CLOSE_BUTTON, 0, 0, 0);
    assert!(c.do_action(0));
    assert_eq!(c.main_modal_id, -1, "local main modal cleared");
    assert_eq!(c.side_modal_id, -1, "local side modal cleared");
    assert_eq!(c.chat_modal_id, -1, "local chat modal cleared");
    assert_eq!(c.out.data()[0] as i32, ClientProt::CLOSE_MODAL.id & 0xff);
    assert_eq!(c.out.pos, 1);
}

/// The real `Client` driver: `click_side_tab` flips `active_icon` and the
/// redraw flags for a bound tab and refuses an unbound one (the client's
/// `handle_tab_clicks` behavior, no packet).
#[test]
fn client_driver_click_side_tab_flips_active_icon() {
    let mut c = Client::new(cfg());
    c.side_icon[5] = 700;
    assert!(c.click_side_tab(5));
    assert_eq!(c.active_icon, 5);
    assert!(c.redraw_side);
    assert!(c.redraw_icons);
    assert_eq!(c.out.pos, 0, "a local flip sends nothing");

    assert!(!c.click_side_tab(9), "unbound tab is refused");
    assert_eq!(c.active_icon, 5, "the failed click does not flip");
    assert!(!c.click_side_tab(14), "out-of-range tab is refused");
}

/// Every `ClientProt` constant has a `LEGAL_SEND` row (same count), every
/// row is a real `ClientProt`, and opcode ids are unique.
#[test]
fn legal_send_covers_every_client_prot() {
    assert_eq!(LEGAL_SEND.len(), ALL_CLIENT_PROTS.len());

    for prot in ALL_CLIENT_PROTS {
        assert!(
            LEGAL_SEND
                .iter()
                .any(|row| row.id == prot.id && row.length == prot.length),
            "LEGAL_SEND is missing a row for {:?}",
            prot
        );
    }
    for row in LEGAL_SEND {
        assert!(
            ALL_CLIENT_PROTS
                .iter()
                .any(|prot| prot.id == row.id && prot.length == row.length),
            "LEGAL_SEND row is not a ClientProt: {:?}",
            row
        );
    }

    let mut ids: Vec<i32> = LEGAL_SEND.iter().map(|row| row.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), LEGAL_SEND.len(), "duplicate opcode ids");
    assert_eq!(
        legal_row(ClientProt::IF_BUTTON),
        LegalSend { id: 9, length: 2 }
    );
}

fn legal_row(prot: ClientProt) -> LegalSend {
    *LEGAL_SEND
        .iter()
        .find(|row| row.id == prot.id && row.length == prot.length)
        .unwrap()
}

/// Settle moved out of the interact suite with task 10: the v1 evidence
/// structs (`item_delta`/`xp_gained` array folds, `modal_delta`, the
/// `done()` budget struct) were replaced by the pollable
/// `api::settle::Settle`/`Outcome` + the eleven evidence predicates, which
/// `tests/settle.rs` covers.

#[test]
fn logout_presses_cc_logout_iface_and_missing_is_false() {
    use api::interact::{logout, CC_LOGOUT};
    use client::config::IfType;
    let empty: Vec<Option<Box<IfType>>> = Vec::new();
    let mut rec = Recorder::default();
    assert!(!logout(&mut rec, &empty));
    assert!(rec.actions.is_empty());
    assert!(rec.menus.is_empty());

    let mut ifaces: Vec<Option<Box<IfType>>> = vec![None; 10];
    let com = IfType {
        client_code: CC_LOGOUT,
        ..Default::default()
    };
    ifaces[7] = Some(Box::new(com));
    let mut rec = Recorder::default();
    assert!(logout(&mut rec, &ifaces));
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(rec.menus, vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 7)]);
}

/// Wire types: every `OpTarget` and `WireCommand` variant constructs with a
/// distinct discriminant, `SendResult` carries either a sent command or a
/// refusal reason, and every `SendReason` variant gets a match arm (the
/// exhaustive `match` is the compile-time count check).
#[test]
fn wire_command_kinds_and_reasons_compile_and_match() {
    use api::interact::{OpTarget, SendReason, SendResult, WireCommand};
    use api::snapshot::{
        ActorView, GroundItemView, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView,
        NpcView, PlayerView, WorldTile,
    };
    use std::collections::HashSet;
    use std::mem::discriminant;

    let tile = WorldTile {
        x: 1,
        z: 2,
        level: 0,
    };
    let npc = NpcView {
        index: 0,
        r#type: None,
        name: None,
        actions: Vec::new(),
        tile,
        distance: 0,
        animation: 0,
        pose_animation: 0,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: 0,
        health: 0,
        total_health: 0,
        face_entity: 0,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 0,
        x: 0,
        z: 0,
        yaw: 0,
    };
    let player = PlayerView {
        index: 0,
        actor: ActorView {
            name: None,
            actions: Vec::new(),
            tile,
            distance: 0,
            animation: 0,
            pose_animation: 0,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: 0,
            health: 0,
            total_health: 0,
            face_entity: 0,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
        },
        combat_level: 0,
        skill_level: 0,
    };
    let loc = LocView {
        typecode: 0,
        info: 0,
        id: 0,
        name: None,
        description: None,
        actions: Vec::new(),
        tile,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 0,
        angle: 0,
        width: 0,
        length: 0,
        footprint_width: 0,
        footprint_length: 0,
        block_walk: false,
        block_range: false,
        active: false,
        animation: 0,
        map_function: 0,
        map_scene: 0,
        force_approach: 0,
    };
    let gi = GroundItemView {
        def: item_def(),
        count: 0,
        actions: Vec::new(),
        tile,
        distance: 0,
    };
    let item = ItemView {
        def: item_def(),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: Vec::new(),
        component_id: 0,
    };

    let targets = [
        OpTarget::Npc(&npc),
        OpTarget::Player(&player),
        OpTarget::Loc(&loc),
        OpTarget::GroundItem(&gi),
        OpTarget::Item(&item),
    ];
    let distinct = targets.iter().map(discriminant).collect::<HashSet<_>>();
    assert_eq!(
        distinct.len(),
        targets.len(),
        "OpTarget discriminants collide"
    );

    let commands = [
        WireCommand::Op {
            target: OpTarget::Npc(&npc),
            operation: 1,
        },
        WireCommand::UseItem {
            select: &item,
            target: OpTarget::Loc(&loc),
        },
        WireCommand::UseWidget {
            component_id: 12,
            target: OpTarget::Item(&item),
        },
        WireCommand::Button {
            component_id: 13,
            button_type: 14,
        },
        WireCommand::Continue { component_id: 15 },
        WireCommand::Close,
        WireCommand::Count { value: 16 },
        WireCommand::Walk { tile },
        WireCommand::SideTab { tab: 17 },
        WireCommand::Login {
            username: "u".into(),
            password: "p".into(),
        },
        WireCommand::ClearLocalModal { component_id: 18 },
    ];
    let distinct = commands.iter().map(discriminant).collect::<HashSet<_>>();
    assert_eq!(
        distinct.len(),
        commands.len(),
        "WireCommand discriminants collide"
    );

    let reasons = [
        SendReason::NotAttached,
        SendReason::NotIngame,
        SendReason::SceneUnavailable,
        SendReason::OffScene,
        SendReason::LevelMismatch,
        SendReason::StaleTarget,
        SendReason::InvalidAction,
        SendReason::UnsupportedTarget,
        SendReason::ComponentNotVisible,
        SendReason::ClientSideOnly,
        SendReason::TargetMaskMismatch,
        SendReason::CountDialogOpen,
        SendReason::NoCountDialog,
        SendReason::InvalidCount,
        SendReason::NoModalOpen,
        SendReason::NoContinue,
        SendReason::NoChoice,
        SendReason::Unreachable,
        SendReason::InvalidTab,
        SendReason::AlreadyIngame,
        SendReason::DriverRejected,
    ];
    for r in reasons {
        let label = match r {
            SendReason::NotAttached => "NotAttached",
            SendReason::NotIngame => "NotIngame",
            SendReason::SceneUnavailable => "SceneUnavailable",
            SendReason::OffScene => "OffScene",
            SendReason::LevelMismatch => "LevelMismatch",
            SendReason::StaleTarget => "StaleTarget",
            SendReason::InvalidAction => "InvalidAction",
            SendReason::UnsupportedTarget => "UnsupportedTarget",
            SendReason::ComponentNotVisible => "ComponentNotVisible",
            SendReason::ClientSideOnly => "ClientSideOnly",
            SendReason::TargetMaskMismatch => "TargetMaskMismatch",
            SendReason::CountDialogOpen => "CountDialogOpen",
            SendReason::NoCountDialog => "NoCountDialog",
            SendReason::InvalidCount => "InvalidCount",
            SendReason::NoModalOpen => "NoModalOpen",
            SendReason::NoContinue => "NoContinue",
            SendReason::NoChoice => "NoChoice",
            SendReason::Unreachable => "Unreachable",
            SendReason::InvalidTab => "InvalidTab",
            SendReason::AlreadyIngame => "AlreadyIngame",
            SendReason::DriverRejected => "DriverRejected",
        };
        assert!(!label.is_empty());
    }
    let distinct = reasons.iter().map(discriminant).collect::<HashSet<_>>();
    assert_eq!(
        distinct.len(),
        reasons.len(),
        "SendReason discriminants collide"
    );

    match (SendResult::Sent {
        tick: 7,
        command: WireCommand::Close,
    }) {
        SendResult::Sent { tick, command } => {
            assert_eq!(tick, 7);
            assert!(matches!(command, WireCommand::Close));
        }
        SendResult::Refused { .. } => panic!("Sent refused"),
    }
    match (SendResult::Refused {
        tick: 8,
        reason: SendReason::Unreachable,
    }) {
        SendResult::Refused { tick, reason } => {
            assert_eq!(tick, 8);
            assert_eq!(reason, SendReason::Unreachable);
        }
        SendResult::Sent { .. } => panic!("Refused sent"),
    }
}

fn item_def() -> ItemDefView {
    ItemDefView {
        id: 0,
        name: None,
        stackable: false,
        members: false,
        base_value: 0,
        noted: false,
        certificate_link: -1,
        certificate_template: -1,
    }
}

// ---- Task 9: Interactions orchestration (ActionResolution/TargetIdentity) ----

/// A planted client attached to a local socket pair (the snapshot.rs
/// pattern): the bound listener plus connected `ClientStream` mark the
/// slot attached, with the scene ready (`ingame && scene_state == 2`).
struct Scene {
    _listener: std::net::TcpListener,
    client: Client,
}

fn configured_client() -> Client {
    let mut c = Client::new(cfg());
    c.ingame = true;
    c.scene_state = SCENE_READY;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c
}

fn scene() -> Scene {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    let mut c = configured_client();
    c.stream = Some(stream);
    Scene {
        _listener: listener,
        client: c,
    }
}

/// Bump every gen and rebuild every family into a fresh snapshot.
fn rebuild(c: &mut Client) -> GameSnapshot {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(c);
    snap
}

fn set_iface(c: &mut Client, id: usize, com: IfType) {
    c.set_iface(id, com);
}

fn set_iface_mut(c: &mut Client, id: usize, m: IfTypeMut) {
    c.set_iface_mut(id, m);
}

/// Plant an npc type whose menu ops the snapshot's npc view carries.
fn plant_npc_type(c: &mut Client, id: i32, name: &str, ops: &[&str]) {
    let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
    while cache.npcs.len() <= id as usize {
        cache.npcs.push(NpcType::default());
    }
    cache.npcs[id as usize] = NpcType {
        id,
        name: name.into(),
        op: ops.iter().map(|s| Some((*s).to_string())).collect(),
        ..Default::default()
    };
}

/// Plant one live npc at `slot` (pixel (50, 50) * 128 → world tile
/// (3250, 3250)).
fn plant_npc(c: &mut Client, slot: usize, id: i32) {
    let mut npc = ClientNpc::default();
    npc.entity.x = 50 * 128;
    npc.entity.z = 50 * 128;
    npc.r#type = Some(id as usize);
    c.npc[slot] = Some(Box::new(npc));
    c.npc_ids = vec![slot as i32];
    c.npc_count = 1;
}

/// The inventory tab (side tab 3) holding one item (stored id 4 → obj 3).
fn plant_inventory(c: &mut Client) {
    set_iface(
        c,
        500,
        IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        500,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );
    c.side_icon[3] = 500;
}

/// A main modal (root 100) with a BUTTON_TARGET widget (101, npc mask),
/// a plain BUTTON_OK (102), a client-code BUTTON_OK (103) and a
/// player-only-mask target widget (104).
fn plant_modal(c: &mut Client) {
    set_iface(
        c,
        100,
        IfType {
            id: 100,
            layer_id: 100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![101, 102, 103, 104]),
            ..Default::default()
        },
    );
    set_iface(
        c,
        101,
        IfType {
            id: 101,
            layer_id: 100,
            r#type: ComponentType::TYPE_TEXT,
            target_mask: 0x2,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        101,
        IfTypeMut {
            button_type: ButtonType::BUTTON_TARGET,
            ..Default::default()
        },
    );
    set_iface(
        c,
        102,
        IfType {
            id: 102,
            layer_id: 100,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        102,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );
    set_iface(
        c,
        103,
        IfType {
            id: 103,
            layer_id: 100,
            r#type: ComponentType::TYPE_TEXT,
            client_code: 205,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        103,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );
    set_iface(
        c,
        104,
        IfType {
            id: 104,
            layer_id: 100,
            r#type: ComponentType::TYPE_TEXT,
            target_mask: 0x8,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        104,
        IfTypeMut {
            button_type: ButtonType::BUTTON_TARGET,
            ..Default::default()
        },
    );
    c.main_modal_id = 100;
}

/// The player-controls overlay (root 0): children 3/4 = retaliate
/// on/off, 5/6 = run off/on (the `controls_pair` indices).
fn plant_controls(c: &mut Client) {
    set_iface(
        c,
        0,
        IfType {
            id: 0,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1, 2, 3, 4, 5, 6]),
            ..Default::default()
        },
    );
    set_iface(
        c,
        1,
        IfType {
            id: 1,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        1,
        IfTypeMut {
            text: "Player controls".into(),
            ..Default::default()
        },
    );
    set_iface(
        c,
        2,
        IfType {
            id: 2,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        2,
        IfTypeMut {
            text: "Auto retaliate".into(),
            ..Default::default()
        },
    );
    for id in 3..=6 {
        set_iface(
            c,
            id,
            IfType {
                id: id as i32,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        set_iface_mut(
            c,
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_TOGGLE,
                ..Default::default()
            },
        );
    }
    c.side_icon[0] = 0;
}

fn fixture_npc(actions: &[&str]) -> NpcView {
    NpcView {
        index: 7,
        r#type: Some(9),
        name: Some("Goblin".into()),
        actions: actions.iter().map(|s| Some((*s).to_string())).collect(),
        tile: WorldTile {
            x: 3250,
            z: 3250,
            level: 0,
        },
        distance: 5,
        animation: 0,
        pose_animation: 0,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        health: 7,
        total_health: 7,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 2,
        size: 1,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

/// `operation_of` scans the target's labels in menu order (trimmed,
/// case-insensitive), skipping empty and "hidden" slots, capped at
/// `MAX_OPERATIONS`.
#[test]
fn operation_of_matches_label_case_insensitively_skipping_hidden() {
    let npc = fixture_npc(&["Attack", "  Hidden ", "", "Pickpocket", "Examine", "Sixth"]);
    let target = OpTarget::Npc(&npc);

    assert_eq!(operation_of(&target, "Attack"), Some(1));
    assert_eq!(
        operation_of(&target, "  attack  "),
        Some(1),
        "trimmed case-insensitive"
    );
    assert_eq!(
        operation_of(&target, "Pickpocket"),
        Some(4),
        "hidden/empty slots are skipped"
    );
    assert_eq!(operation_of(&target, "Examine"), Some(5));
    assert_eq!(
        operation_of(&target, "Sixth"),
        None,
        "beyond MAX_OPERATIONS"
    );
    assert_eq!(operation_of(&target, "Nope"), None);
    assert_eq!(MAX_OPERATIONS, 5);
}

/// `offers_operation` accepts only the 1..=MAX_OPERATIONS range with a
/// usable action in that slot.
#[test]
fn offers_operation_guards_range_and_usable_slots() {
    let npc = fixture_npc(&["Attack", "hidden", "Examine"]);
    let target = OpTarget::Npc(&npc);
    assert!(offers_operation(&target, 1));
    assert!(!offers_operation(&target, 2), "hidden slot");
    assert!(offers_operation(&target, 3));
    assert!(!offers_operation(&target, 4), "no fourth action");
    assert!(!offers_operation(&target, 0), "operations are 1-based");
    assert!(!offers_operation(&target, 6), "past MAX_OPERATIONS");
}

/// Recording driver + planted snapshot: walk to a same-level in-scene
/// tile sends a `Walk` through `try_move`; an off-scene tile is refused
/// with `OffScene` before the driver sees it.
#[test]
fn interactions_walk_refuses_off_scene_and_sends_when_ok() {
    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder {
        route: Some((50, 50)),
        base: (3200, 3200),
        ..Recorder::default()
    };
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.walk(WorldTile {
            x: 3210,
            z: 3212,
            level: 0,
        }) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Walk { tile } if tile == WorldTile { x: 3210, z: 3212, level: 0 }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("in-scene walk refused: {reason:?}"),
        }
        match ix.walk(WorldTile {
            x: 3500,
            z: 3212,
            level: 0,
        }) {
            SendResult::Refused { tick, reason } => {
                assert_eq!(tick, snap.tick() as u64);
                assert_eq!(reason, SendReason::OffScene);
            }
            SendResult::Sent { .. } => panic!("off-scene walk sent"),
        }
    }
    assert_eq!(rec.moves, vec![(50, 50, 10, 12, false, 0, 0, 0, 0, 0, 0)]);
}

/// `interact` resolves a label (and a raw operation) to an op number,
/// then dispatches the matching `MiniMenuAction` opcode through
/// `set_menu`/`do_action` with the npc slot as menu param `a`.
#[test]
fn interact_dispatches_npc_op_through_menu() {
    let mut s = scene();
    plant_npc_type(
        &mut s.client,
        9,
        "Goblin",
        &["Attack", "Pickpocket", "Examine"],
    );
    plant_npc(&mut s.client, 7, 9);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.interact(
            OpTarget::Npc(&snap.npcs()[0]),
            ActionSpec::Label("Pickpocket".into()),
        ) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(command, WireCommand::Op { operation: 2, .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        match ix.interact(OpTarget::Npc(&snap.npcs()[0]), ActionSpec::Operation(1)) {
            SendResult::Sent { command, .. } => {
                assert!(matches!(command, WireCommand::Op { operation: 1, .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0, 0]);
    assert_eq!(
        rec.menus,
        vec![
            (0, MiniMenuAction::OP_NPC2, 7, 0, 0),
            (0, MiniMenuAction::OP_NPC1, 7, 0, 0),
        ]
    );
}

/// A loc target dispatches through the scene coords with the packed
/// typecode as menu param `a` (the `interact_with_loc` contract).
#[test]
fn interact_dispatches_loc_op_through_scene_coords() {
    let mut s = scene();
    let typecode = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
    s.client
        .world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    let loc_id = (typecode >> 14) & 0x7fff;
    {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        while cache.locs.len() <= loc_id as usize {
            cache.locs.push(LocType::default());
        }
        cache.locs[loc_id as usize] = LocType {
            id: loc_id,
            name: "Door".into(),
            op: vec![Some("Open".into()), None, None, None, None],
            ..Default::default()
        };
    }
    let snap = rebuild(&mut s.client);
    let loc = &snap.locs()[0];
    assert_eq!(
        loc.tile,
        WorldTile {
            x: 3203,
            z: 3204,
            level: 0
        }
    );
    let mut rec = Recorder {
        base: (3200, 3200),
        ..Recorder::default()
    };
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.interact(OpTarget::Loc(loc), ActionSpec::Label("Open".into())) {
            SendResult::Sent { command, .. } => {
                assert!(matches!(command, WireCommand::Op { operation: 1, .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_LOC1, typecode, 3, 4)]
    );
}

/// Nearest scene loc with `Use-quickly` on the player's plane. A closer
/// loc with `Bank`, a Banker NPC Talk-to, and a Use-quickly on another
/// plane are never chosen. None on the plane refuses closed.
#[test]
fn open_nearest_booth_clicks_use_quickly_on_the_same_plane() {
    let mut s = scene();
    s.client.local_player = Some(ClientPlayer::at(5, 5));
    let near_id = 2213;
    let far_id = 2214;
    let bank_id = 2215;
    let near_tc = 0x4000_0000 + (near_id << 14) + 1 + (2 << 7);
    let far_tc = 0x4000_0000 + (far_id << 14) + 1 + (2 << 7);
    let bank_tc = 0x4000_0000 + (bank_id << 14) + 1 + (2 << 7);
    s.client
        .world
        .set_wall(0, 6, 5, 0, 0, 0, near_tc, 1 << 6, 0, 0, 0, 0);
    s.client
        .world
        .set_wall(0, 20, 5, 0, 0, 0, far_tc, 1 << 6, 0, 0, 0, 0);
    s.client
        .world
        .set_wall(0, 5, 6, 0, 0, 0, bank_tc, 1 << 6, 0, 0, 0, 0);
    {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        while cache.locs.len() <= bank_id as usize {
            cache.locs.push(LocType::default());
        }
        cache.locs[near_id as usize] = LocType {
            id: near_id,
            name: "Bank booth".into(),
            op: vec![None, Some("Use-quickly".into()), None, None, None],
            ..Default::default()
        };
        cache.locs[far_id as usize] = LocType {
            id: far_id,
            name: "Bank booth".into(),
            op: vec![None, Some("Use-quickly".into()), None, None, None],
            ..Default::default()
        };
        cache.locs[bank_id as usize] = LocType {
            id: bank_id,
            name: "Bank booth".into(),
            op: vec![Some("Bank".into()), None, None, None, None],
            ..Default::default()
        };
    }
    plant_npc_type(&mut s.client, 494, "Banker", &["Talk-to", "Bank"]);
    plant_npc(&mut s.client, 2, 494);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder {
        base: (3200, 3200),
        ..Recorder::default()
    };
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.open_nearest_booth() {
            SendResult::Sent { command, .. } => {
                assert!(matches!(command, WireCommand::Op { operation: 2, .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_LOC2, near_tc, 6, 5)],
        "nearest Use-quickly on the same plane, never Bank / Talk-to"
    );

    let mut s = scene();
    s.client.local_player = Some(ClientPlayer::at(5, 5));
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.open_nearest_booth(),
            SendResult::Refused {
                reason: SendReason::StaleTarget,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty(), "none on the plane fails closed");
}

/// Out of reach: walk toward the booth instead of Use-quickly (live
/// FAIL was `"I can't reach that!"` from op-ing with no walk).
#[test]
fn open_nearest_booth_walks_when_not_adjacent() {
    let mut s = scene();
    s.client.local_player = Some(ClientPlayer::at(5, 5));
    let booth_id = 2213;
    let far_tc = 0x4000_0000 + (booth_id << 14) + 1 + (2 << 7);
    s.client
        .world
        .set_wall(0, 20, 5, 0, 0, 0, far_tc, 1 << 6, 0, 0, 0, 0);
    {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        while cache.locs.len() <= booth_id as usize {
            cache.locs.push(LocType::default());
        }
        cache.locs[booth_id as usize] = LocType {
            id: booth_id,
            name: "Bank booth".into(),
            op: vec![None, Some("Use-quickly".into()), None, None, None],
            ..Default::default()
        };
    }
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder {
        base: (3200, 3200),
        route: Some((5, 5)),
        ..Recorder::default()
    };
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.open_nearest_booth() {
            SendResult::Sent { command, .. } => {
                assert!(
                    matches!(command, WireCommand::Walk { .. }),
                    "out-of-reach booth must walk, not op: {command:?}"
                );
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert!(
        rec.menus.is_empty(),
        "must not Use-quickly from 15 tiles away: menus={:?}",
        rec.menus
    );
    assert!(
        !rec.moves.is_empty(),
        "must try_move toward the booth when not adjacent"
    );
}

/// A ground-item target dispatches through the scene coords with the obj
/// id as menu param `a`.
#[test]
fn interact_dispatches_ground_item_op_through_scene_coords() {
    let mut s = scene();
    let bones_id = {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        let id = cache.objs.len() as i32;
        cache.objs.push(ObjType {
            id,
            name: "Bones".into(),
            op: [Some("Take".into()), None, None, None, None],
            ..Default::default()
        });
        id
    };
    let mut list = LinkList::new();
    list.push(ClientObj::new(bones_id, 2));
    s.client.ground_obj[0][10][12] = Some(Box::new(list));
    let snap = rebuild(&mut s.client);
    let gi = &snap.ground_items()[0];
    assert_eq!(
        gi.tile,
        WorldTile {
            x: 3210,
            z: 3212,
            level: 0
        }
    );
    let mut rec = Recorder {
        base: (3200, 3200),
        ..Recorder::default()
    };
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.interact(OpTarget::GroundItem(gi), ActionSpec::Label("Take".into())) {
            SendResult::Sent { command, .. } => {
                assert!(matches!(command, WireCommand::Op { operation: 1, .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_OBJ1, bones_id, 10, 12)]
    );
}

/// `use_item_on` first arms the held item (`USEHELD_START`) then aims the
/// target kind (`USEHELD_ONNPC`), both through `set_menu`/`do_action`.
#[test]
fn use_item_on_selects_held_item_then_aims_target() {
    let mut s = scene();
    plant_npc_type(&mut s.client, 9, "Goblin", &["Attack"]);
    plant_npc(&mut s.client, 7, 9);
    plant_inventory(&mut s.client);
    let snap = rebuild(&mut s.client);
    assert_eq!(snap.inventory()[0].def.id, 3);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.use_item_on(&snap.inventory()[0], OpTarget::Npc(&snap.npcs()[0])) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(command, WireCommand::UseItem { .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0, 0]);
    assert_eq!(
        rec.menus,
        vec![
            (0, MiniMenuAction::USEHELD_START, 3, 0, 500),
            (0, MiniMenuAction::USEHELD_ONNPC, 7, 0, 0),
        ]
    );
}

/// `Interactions::wear` resolves the held item by obj id and dispatches
/// its Wear op (or Wield for weapons) — the BankBudget session's arm.
#[test]
fn wear_dispatches_the_held_wear_op() {
    let mut s = scene();
    plant_inventory(&mut s.client);
    {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        cache.objs.resize(4, ObjType::default());
        cache.objs[3] = ObjType {
            id: 3,
            iop: [Some("Wear".into()), None, None, None, None],
            ..Default::default()
        };
    }
    let snap = rebuild(&mut s.client);
    assert_eq!(snap.inventory()[0].def.id, 3);
    assert_eq!(snap.inventory()[0].actions[0].as_deref(), Some("Wear"));
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.wear(3) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(command, WireCommand::Op { operation: 1, .. }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_HELD1, 3, 0, 500)],
        "OP_HELD1 at the held item id/slot/component"
    );
}

/// Weapons menu their op as Wield; `wear` accepts that label too.
#[test]
fn wear_accepts_a_wield_weapon_menu() {
    let mut s = scene();
    plant_inventory(&mut s.client);
    {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        cache.objs.resize(4, ObjType::default());
        cache.objs[3] = ObjType {
            id: 3,
            iop: [Some("Wield".into()), None, None, None, None],
            ..Default::default()
        };
    }
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(ix.wear(3), SendResult::Sent { .. }));
    }
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::OP_HELD1, 3, 0, 500)],
        "a Wield weapon wears through the same op"
    );
}

/// `wear` fails closed: a non-wearable item (no Wear/Wield slot) is
/// `InvalidAction`, and an item that is not held is `StaleTarget`.
#[test]
fn wear_refuses_non_wearable_and_missing_items() {
    let mut s = scene();
    plant_inventory(&mut s.client); // no iop: only a Drop slot
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.wear(3),
            SendResult::Refused {
                reason: SendReason::InvalidAction,
                ..
            }
        ));
        assert!(matches!(
            ix.wear(999),
            SendResult::Refused {
                reason: SendReason::StaleTarget,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty(), "nothing sent");
    assert!(rec.menus.is_empty());
}

/// `press` dispatches a BUTTON_OK through IF_BUTTON and refuses the
/// target-verb and client-code arms before the driver sees them.
#[test]
fn press_dispatches_button_and_refuses_target_client_side_only() {
    let mut s = scene();
    plant_modal(&mut s.client);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        let widget_ok = snap
            .widgets()
            .iter()
            .find(|w| w.component_id == 102)
            .unwrap();
        match ix.press(widget_ok) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Button {
                        component_id: 102,
                        button_type: 1
                    }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        let widget_target = snap
            .widgets()
            .iter()
            .find(|w| w.component_id == 101)
            .unwrap();
        assert!(matches!(
            ix.press(widget_target),
            SendResult::Refused {
                reason: SendReason::InvalidAction,
                ..
            }
        ));
        let widget_code = snap
            .widgets()
            .iter()
            .find(|w| w.component_id == 103)
            .unwrap();
        assert!(matches!(
            ix.press(widget_code),
            SendResult::Refused {
                reason: SendReason::ClientSideOnly,
                ..
            }
        ));
    }
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(rec.menus, vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 102)]);
}

/// `use_widget_on` opens the target-button arm (`TGT_BUTTON`) then aims
/// the target kind (`TGT_NPC`), and refuses on a target-mask mismatch.
#[test]
fn use_widget_on_aims_target_after_target_button() {
    let mut s = scene();
    plant_npc_type(&mut s.client, 9, "Goblin", &["Attack"]);
    plant_npc(&mut s.client, 7, 9);
    plant_modal(&mut s.client);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        let widget = snap
            .widgets()
            .iter()
            .find(|w| w.component_id == 101)
            .unwrap();
        match ix.use_widget_on(widget, OpTarget::Npc(&snap.npcs()[0])) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::UseWidget {
                        component_id: 101,
                        ..
                    }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        let wrong_mask = snap
            .widgets()
            .iter()
            .find(|w| w.component_id == 104)
            .unwrap();
        assert!(matches!(
            ix.use_widget_on(wrong_mask, OpTarget::Npc(&snap.npcs()[0])),
            SendResult::Refused {
                reason: SendReason::TargetMaskMismatch,
                ..
            }
        ));
    }
    assert_eq!(rec.actions, vec![0, 0]);
    assert_eq!(
        rec.menus,
        vec![
            (0, MiniMenuAction::TGT_BUTTON, 0, 0, 101),
            (0, MiniMenuAction::TGT_NPC, 7, 0, 0),
        ]
    );
}

/// `continue_dialog` sends the chat modal's PAUSE_BUTTON and refuses with
/// `NoContinue` when no continue component is open.
#[test]
fn continue_dialog_sends_pause_button_and_refuses_without_chat_modal() {
    let mut s = scene();
    set_iface(
        &mut s.client,
        200,
        IfType {
            id: 200,
            layer_id: 200,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![201]),
            ..Default::default()
        },
    );
    set_iface(
        &mut s.client,
        201,
        IfType {
            id: 201,
            layer_id: 200,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut s.client,
        201,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            ..Default::default()
        },
    );
    s.client.chat_modal_id = 200;
    let snap = rebuild(&mut s.client);
    assert_eq!(snap.chat_continue_component_id(), 201);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.continue_dialog() {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Continue { component_id: 201 }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(
        rec.menus,
        vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, 201)]
    );

    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.continue_dialog(),
            SendResult::Refused {
                reason: SendReason::NoContinue,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty());
}

/// `answer_choice` presses the chat modal's `option`-th BUTTON_OK choice
/// (the `p_choiceN` option an edge's `option` names) and refuses with
/// `NoChoice` when no such choice is up.
#[test]
fn answer_choice_presses_the_chat_option_button_and_refuses_without_one() {
    let mut s = scene();
    set_iface(
        &mut s.client,
        200,
        IfType {
            id: 200,
            layer_id: 200,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![201, 202]),
            ..Default::default()
        },
    );
    for (id, text) in [
        (201usize, "Yes please, I'd like to go to Brimhaven."),
        (202, "No thanks."),
    ] {
        set_iface(
            &mut s.client,
            id,
            IfType {
                id: id as i32,
                layer_id: 200,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut s.client,
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                text: text.into(),
                ..Default::default()
            },
        );
    }
    s.client.chat_modal_id = 200;
    let snap = rebuild(&mut s.client);
    assert_eq!(snap.chat_options().len(), 2);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.answer_choice(1) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Button {
                        component_id: 201,
                        button_type: ButtonType::BUTTON_OK
                    }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        // Out-of-range option: no such choice.
        assert!(matches!(
            ix.answer_choice(3),
            SendResult::Refused {
                reason: SendReason::NoChoice,
                ..
            }
        ));
    }
    assert_eq!(rec.menus, vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 201)]);

    // No chat modal: the second choice is refused, never a fake send.
    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.answer_choice(1),
            SendResult::Refused {
                reason: SendReason::NoChoice,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty());
}

/// `close_modal` dispatches CLOSE_BUTTON through the driver (so the real
/// client's `close_modal` clears the local modal ids and writes
/// CLOSE_MODAL) and refuses with `NoModalOpen` when all four roots are
/// closed.
#[test]
fn close_modal_dispatches_close_button_and_refuses_with_none_open() {
    let mut s = scene();
    plant_modal(&mut s.client);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.close_modal() {
            SendResult::Sent { command, .. } => assert!(matches!(command, WireCommand::Close)),
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(rec.menus, vec![(0, MiniMenuAction::CLOSE_BUTTON, 0, 0, 0)]);
    assert!(rec.out.0.is_empty(), "the client-local close clears state");

    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.close_modal(),
            SendResult::Refused {
                reason: SendReason::NoModalOpen,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty());
    assert!(rec.out.0.is_empty());
}

/// `answer_count` writes RESUME_P_COUNTDIALOG for a valid count and
/// refuses when no count dialog is open or the count is negative.
#[test]
fn answer_count_writes_resume_p_countdialog_and_refuses_bad_states() {
    let mut s = scene();
    s.client.dialog_input_open = true;
    let snap = rebuild(&mut s.client);
    assert!(snap.count_dialog_open());
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.answer_count(1500) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(command, WireCommand::Count { value: 1500 }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        assert!(matches!(
            ix.answer_count(-1),
            SendResult::Refused {
                reason: SendReason::InvalidCount,
                ..
            }
        ));
    }
    assert_eq!(
        rec.out.0,
        vec![
            OutByte::Enc(ClientProt::RESUME_P_COUNTDIALOG.id),
            OutByte::P4(1500),
        ]
    );

    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.answer_count(5),
            SendResult::Refused {
                reason: SendReason::NoCountDialog,
                ..
            }
        ));
    }
    assert!(rec.out.0.is_empty());
}

/// `click_side_tab` routes the tab through `Driver::click_side_tab` (the
/// client-local side-icon flip) and refuses an unavailable tab before the
/// driver sees it.
#[test]
fn click_side_tab_dispatches_driver_hook_and_refuses_unavailable_tab() {
    let mut s = scene();
    plant_inventory(&mut s.client);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.click_side_tab(3) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(command, WireCommand::SideTab { tab: 3 }));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        assert!(matches!(
            ix.click_side_tab(99),
            SendResult::Refused {
                reason: SendReason::InvalidTab,
                ..
            }
        ));
    }
    assert_eq!(rec.side_tabs, vec![3]);
    assert!(rec.out.0.is_empty(), "no wire send for a local flip");
}

/// `login` refuses while ingame and routes the handshake through
/// `Driver::login` once logged out.
#[test]
fn login_refuses_when_ingame_and_sends_when_logged_out() {
    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.login("bot", "hunter2"),
            SendResult::Refused {
                reason: SendReason::AlreadyIngame,
                ..
            }
        ));
    }
    assert_eq!(rec.logins, 0);

    let mut s = scene();
    s.client.ingame = false;
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.login("bot", "hunter2") {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Login { ref username, .. } if username == "bot"
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.logins, 1);
}

/// `clear_local_modal` refuses when the named component is not an open
/// root and dispatches CLOSE_BUTTON (the client-local close) when it is.
#[test]
fn clear_local_modal_refuses_when_modal_not_open_and_closes_when_open() {
    let mut s = scene();
    plant_modal(&mut s.client);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.clear_local_modal(999),
            SendResult::Refused {
                reason: SendReason::NoModalOpen,
                ..
            }
        ));
        match ix.clear_local_modal(100) {
            SendResult::Sent { command, .. } => {
                assert!(matches!(
                    command,
                    WireCommand::ClearLocalModal { component_id: 100 }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
    }
    assert_eq!(rec.actions, vec![0]);
    assert_eq!(rec.menus, vec![(0, MiniMenuAction::CLOSE_BUTTON, 0, 0, 0)]);
    assert!(rec.out.0.is_empty(), "the client-local close clears state");
}

/// `set_run`/`set_retaliate` press the on/off toggle components of the
/// controls overlay with the live button type.
#[test]
fn set_run_and_retaliate_press_the_toggle_components() {
    let mut s = scene();
    plant_controls(&mut s.client);
    let snap = rebuild(&mut s.client);
    let run = snap.run_controls().expect("run toggle pair");
    assert_eq!((run.on_component_id, run.off_component_id), (6, 5));
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.set_run(true) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Button {
                        component_id: 6,
                        button_type: 4
                    }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        assert!(matches!(ix.set_run(false), SendResult::Sent { .. }));
        assert!(matches!(ix.set_retaliate(true), SendResult::Sent { .. }));
        assert!(matches!(ix.set_retaliate(false), SendResult::Sent { .. }));
    }
    assert_eq!(
        rec.menus,
        vec![
            (0, MiniMenuAction::TOGGLE_BUTTON, 0, 0, 6),
            (0, MiniMenuAction::TOGGLE_BUTTON, 0, 0, 5),
            (0, MiniMenuAction::TOGGLE_BUTTON, 0, 0, 3),
            (0, MiniMenuAction::TOGGLE_BUTTON, 0, 0, 4),
        ]
    );
}

/// The m8aq precondition order: not attached, not ingame, count dialog
/// open (unless answering the count), scene not ready.
#[test]
fn interact_refuses_when_not_attached_not_ingame_or_count_dialog_open() {
    let npc = fixture_npc(&["Attack"]);
    let attack = ActionSpec::Label("Attack".into());

    let mut c = configured_client();
    let snap = rebuild(&mut c);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.interact(OpTarget::Npc(&npc), attack.clone()),
            SendResult::Refused {
                reason: SendReason::NotAttached,
                ..
            }
        ));
    }

    let mut s = scene();
    s.client.ingame = false;
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.interact(OpTarget::Npc(&npc), attack.clone()),
            SendResult::Refused {
                reason: SendReason::NotIngame,
                ..
            }
        ));
    }

    let mut s = scene();
    s.client.dialog_input_open = true;
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.interact(OpTarget::Npc(&npc), attack.clone()),
            SendResult::Refused {
                reason: SendReason::CountDialogOpen,
                ..
            }
        ));
        assert!(matches!(
            ix.walk(WorldTile {
                x: 3210,
                z: 3212,
                level: 0
            }),
            SendResult::Refused {
                reason: SendReason::CountDialogOpen,
                ..
            }
        ));
    }

    let mut s = scene();
    s.client.scene_state = 1;
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.interact(OpTarget::Npc(&npc), attack),
            SendResult::Refused {
                reason: SendReason::SceneUnavailable,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty());
}

/// `interact` refuses an unresolvable action, a stale target and an item
/// target with no action family; tile-addressed targets also get the
/// level/off-scene checks before identity.
#[test]
fn interact_refuses_invalid_action_stale_target_and_unsupported() {
    let mut s = scene();
    plant_npc_type(&mut s.client, 9, "Goblin", &["Attack"]);
    plant_npc(&mut s.client, 7, 9);
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        let npc = &snap.npcs()[0];

        assert!(matches!(
            ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Bark".into())),
            SendResult::Refused {
                reason: SendReason::InvalidAction,
                ..
            }
        ));
        assert!(matches!(
            ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(9)),
            SendResult::Refused {
                reason: SendReason::InvalidAction,
                ..
            }
        ));

        let mut gone = npc.clone();
        gone.index = 99;
        assert!(matches!(
            ix.interact(OpTarget::Npc(&gone), ActionSpec::Label("Attack".into())),
            SendResult::Refused {
                reason: SendReason::StaleTarget,
                ..
            }
        ));

        let item = ItemView {
            def: item_def(),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::None,
            slot: 0,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        };
        assert!(matches!(
            ix.interact(OpTarget::Item(&item), ActionSpec::Label("Wield".into())),
            SendResult::Refused {
                reason: SendReason::UnsupportedTarget,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty());
    assert!(rec.menus.is_empty());
}

/// `still_present` re-checks the target's identity against the snapshot
/// per kind: npc slot+id, remote player slot+name (never the self slot),
/// loc typecode/layer/tile, ground-item id/tile, item id/slot/component.
#[test]
fn still_present_matches_identity_per_kind() {
    let mut s = scene();
    // npc
    plant_npc_type(&mut s.client, 9, "Goblin", &["Attack"]);
    plant_npc(&mut s.client, 7, 9);
    // remote player
    let mut other = ClientPlayer::at(15, 16);
    other.entity.x = 100;
    other.entity.z = 150;
    other.name = Some("Other".into());
    other.combat_level = 3;
    other.skill_level = 5;
    s.client.players[3] = Some(Box::new(other));
    s.client.player_ids = vec![3];
    s.client.player_count = 1;
    // a loc (wall) and a ground-item stack
    let typecode = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
    s.client
        .world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    let bones_id = {
        let cache = Arc::get_mut(&mut s.client.cache).expect("sole cache owner");
        let id = cache.objs.len() as i32;
        cache.objs.push(ObjType {
            id,
            name: "Bones".into(),
            ..Default::default()
        });
        id
    };
    let mut list = LinkList::new();
    list.push(ClientObj::new(bones_id, 2));
    s.client.ground_obj[0][10][12] = Some(Box::new(list));
    plant_inventory(&mut s.client);
    let snap = rebuild(&mut s.client);

    let npc = &snap.npcs()[0];
    assert!(still_present(&OpTarget::Npc(npc), &snap));
    let mut slot_moved = npc.clone();
    slot_moved.index = 99;
    assert!(!still_present(&OpTarget::Npc(&slot_moved), &snap));
    let mut id_changed = npc.clone();
    id_changed.r#type = Some(999);
    assert!(!still_present(&OpTarget::Npc(&id_changed), &snap));

    let player = &snap.players()[0];
    assert!(still_present(&OpTarget::Player(player), &snap));
    let mut renamed = player.clone();
    renamed.actor.name = Some("Renamed".into());
    assert!(
        !still_present(&OpTarget::Player(&renamed), &snap),
        "player name mismatch"
    );

    let loc = &snap.locs()[0];
    assert!(still_present(&OpTarget::Loc(loc), &snap));
    let mut loc_moved = loc.clone();
    loc_moved.tile.x += 1;
    assert!(
        !still_present(&OpTarget::Loc(&loc_moved), &snap),
        "loc tile moved"
    );
    let mut loc_layer = loc.clone();
    loc_layer.layer = LocLayer::Ground;
    assert!(
        !still_present(&OpTarget::Loc(&loc_layer), &snap),
        "loc layer changed"
    );

    let gi = &snap.ground_items()[0];
    assert!(still_present(&OpTarget::GroundItem(gi), &snap));
    let mut gi_moved = gi.clone();
    gi_moved.tile.z += 1;
    assert!(
        !still_present(&OpTarget::GroundItem(&gi_moved), &snap),
        "ground-item tile moved"
    );

    let item = &snap.inventory()[0];
    assert!(still_present(&OpTarget::Item(item), &snap));
    let mut item_slot = item.clone();
    item_slot.slot = 5;
    assert!(
        !still_present(&OpTarget::Item(&item_slot), &snap),
        "item slot moved"
    );

    // the self slot is never a present player target
    let mut s2 = scene();
    s2.client.self_slot = 3;
    let mut other = ClientPlayer::at(15, 16);
    other.name = Some("Other".into());
    s2.client.players[3] = Some(Box::new(other));
    s2.client.player_ids = vec![3];
    s2.client.player_count = 1;
    let snap2 = rebuild(&mut s2.client);
    assert!(
        !still_present(&OpTarget::Player(&snap2.players()[0]), &snap2),
        "the self slot is never a target"
    );
}

/// `set_note_mode` presses the bank Note (on) / Item (off) toggle buttons
/// discovered on the open main modal; missing controls refuse closed.
#[test]
fn set_note_mode_presses_bank_note_toggle_and_refuses_without_controls() {
    let mut s = scene();
    plant_npc_type(&mut s.client, 9, "Goblin", &["Attack"]);
    // Bank main modal 600: withdraw 601 + Note 602 + Item 603.
    set_iface(
        &mut s.client,
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601, 602, 603]),
            ..Default::default()
        },
    );
    set_iface(
        &mut s.client,
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [
                Some("Withdraw 1".into()),
                Some("Withdraw 5".into()),
                Some("Withdraw 10".into()),
                Some("Withdraw All".into()),
                None,
            ],
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut s.client,
        601,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0]),
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );
    for (id, text) in [(602, "Note"), (603, "Item")] {
        set_iface(
            &mut s.client,
            id,
            IfType {
                id: id as i32,
                layer_id: 600,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut s.client,
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_TOGGLE,
                text: text.into(),
                ..Default::default()
            },
        );
    }
    s.client.main_modal_id = 600;
    let snap = rebuild(&mut s.client);
    assert!(snap.bank_note_controls().is_some());
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        match ix.set_note_mode(true) {
            SendResult::Sent { tick, command } => {
                assert_eq!(tick, snap.tick() as u64);
                assert!(matches!(
                    command,
                    WireCommand::Button {
                        component_id: 602,
                        button_type: ButtonType::BUTTON_TOGGLE
                    }
                ));
            }
            SendResult::Refused { reason, .. } => panic!("refused: {reason:?}"),
        }
        assert!(matches!(ix.set_note_mode(false), SendResult::Sent { .. }));
    }
    assert_eq!(
        rec.menus,
        vec![
            (0, MiniMenuAction::TOGGLE_BUTTON, 0, 0, 602),
            (0, MiniMenuAction::TOGGLE_BUTTON, 0, 0, 603),
        ]
    );

    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.set_note_mode(true),
            SendResult::Refused {
                reason: SendReason::ComponentNotVisible,
                ..
            }
        ));
    }
    assert!(rec.actions.is_empty());
}

/// `create_interactions` wires the same snapshot + driver pair.
#[test]
fn create_interactions_wires_snapshot_and_driver() {
    let mut s = scene();
    let snap = rebuild(&mut s.client);
    let mut rec = Recorder::default();
    let mut ix = create_interactions(&snap, &mut rec);
    assert!(matches!(
        ix.close_modal(),
        SendResult::Refused {
            reason: SendReason::NoModalOpen,
            ..
        }
    ));
}
