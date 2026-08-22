// Task 9: interact/settle kernel methods map to `doAction`/`tryMove`/`out`
// through a `Driver`, and the legal send table covers every `ClientProt`.
// The `Recorder` stub stands in for the real `Client` driver; two tests
// exercise the real `Client` driver end-to-end (same `/tmp` cache trick as
// `client/tests/gens.rs` — no network).

use api::interact::{
    answer_count, close_modal, interact, login, press, set_run, walk, Driver, RUN_ORB_IFACE,
};
use api::prot::{LegalSend, LEGAL_SEND};
use api::settle::{item_delta, modal_delta, xp_gained, Settle};
use client::client::{Client, ClientConfig, ClientPlayer, MiniMenuAction};
use client::io::{ClientProt, Isaac};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutByte {
    Enc(i32),
    P2(i32),
    P4(i32),
}

impl api::prot::Out for OutSink {
    fn p1_enc(&mut self, opcode: i32) {
        self.0.push(OutByte::Enc(opcode));
    }
    fn p2(&mut self, value: i32) {
        self.0.push(OutByte::P2(value));
    }
    fn p4(&mut self, value: i32) {
        self.0.push(OutByte::P4(value));
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
    moves: Vec<(i32, i32, i32, i32, bool, i32, i32, i32, i32, i32, i32)>,
    out: OutSink,
    logins: usize,
    route: Option<(i32, i32)>,
}

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

    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        self.logins += 1;
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

/// `interact` dispatches the already-prepared menu slot.
#[test]
fn interact_dispatches_prepared_slot() {
    let mut r = Recorder::default();
    assert!(interact(&mut r, 2));
    assert_eq!(r.actions, vec![2]);
    assert!(r.menus.is_empty());
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

/// Settle: before/after inv counts fold into the `item_delta` evidence.
#[test]
fn settle_item_delta_matches_inv_counts() {
    let before = [1, 0, 5];
    let after = [1, 3, 5];
    assert_eq!(item_delta(&before, &after), 3);
    assert_eq!(item_delta(&after, &before), -3);
    assert_eq!(item_delta(&before, &before), 0);
}

/// Settle: xp evidence is the non-negative gain across skills.
#[test]
fn settle_xp_gained_is_positive_gain() {
    let before = [100, 50];
    let after = [100, 62];
    assert_eq!(xp_gained(&before, &after), 12);
    assert_eq!(xp_gained(&after, &before), 0);
}

/// Settle: modal open/close transitions are detected from before/after ids.
#[test]
fn settle_detects_modal_open_and_close() {
    let (opened, closed) = modal_delta(None, Some(94));
    assert_eq!(opened, Some(94));
    assert_eq!(closed, None);

    let (opened, closed) = modal_delta(Some(94), None);
    assert_eq!(opened, None);
    assert_eq!(closed, Some(94));

    let (opened, closed) = modal_delta(Some(94), Some(94));
    assert_eq!((opened, closed), (None, None));
}

/// Settle: `done` needs armed evidence within the tick/ms budget.
#[test]
fn settle_done_requires_evidence_within_budget() {
    let s = Settle {
        arrived: true,
        ticks: 1,
        ms: 40,
        ..Settle::default()
    };
    assert!(s.done());

    let overdue = Settle {
        arrived: true,
        ticks: 11,
        ..Settle::default()
    };
    assert!(!overdue.done(), "past the tick budget");

    let no_evidence = Settle::default();
    assert!(!no_evidence.done(), "no evidence armed");
}
