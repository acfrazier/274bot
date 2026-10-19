//! Named bank openers act on native scene observations and accept host receipts.

use std::sync::Arc;

#[path = "common/bank_slot.rs"]
mod bank_slot;
use bank_slot::BankSlot;

use api::named_banks::NamedBankFacts;
use api::quest_progress::EvidenceStamp;
use api::snapshot::WorldTile;
use script::isolate_fb::{
    ChatOptionInput, IsolateBuf, NativeFactsInput, ReachViewInput, SceneEntityInput,
    SnapshotFingerprint, SnapshotInput, TileInput,
};
use script::native::{HostAction, HostEffect, InteractionReceipt, WalkEnd, WalkReceipt};
use script::shim::InteractReq;
use script::LoadShape;

fn spawn(src: &str) -> BankSlot {
    BankSlot::new(
        src,
        LoadShape::CompatClass,
        None,
        Arc::new(NamedBankFacts::default()),
    )
}

fn post_snapshot_input(slot: &mut BankSlot, input: &SnapshotInput<'_>) {
    slot.post_snapshot(input, NativeFactsInput::default());
}

fn post_snapshot_native(slot: &mut BankSlot, input: &SnapshotInput<'_>) {
    slot.post_snapshot(input, NativeFactsInput::default());
}

fn post_snapshot_delta_native(
    slot: &mut BankSlot,
    encoder: &mut IsolateBuf,
    last: &mut Option<SnapshotFingerprint>,
    input: &SnapshotInput<'_>,
) {
    let (bytes, next) = encoder.encode_snapshot_delta_with_native(
        last.as_ref(),
        input,
        NativeFactsInput::default(),
        false,
    );
    *last = Some(next);
    slot.post_snapshot_bytes(bytes, input, NativeFactsInput::default());
}

fn tick(slot: &mut BankSlot, n: u64, input: &SnapshotInput<'_>) {
    slot.tick_posted_with_frames(
        n,
        input,
        NativeFactsInput {
            bank_snapshot_generation: (input.bank_generation != 0).then(|| {
                i64::try_from(input.bank_generation).expect("fixture packet generation fits i64")
            }),
            ..Default::default()
        },
    );
}

fn one_action(slot: &mut BankSlot, message: &str) -> HostAction {
    let mut actions = slot.take_actions();
    assert_eq!(actions.len(), 1, "{message}");
    actions.pop().unwrap()
}

fn accept_walk(slot: &mut BankSlot, action: &HostAction, tick: u64) {
    let authority = action.authority();
    slot.slot.complete_native_walk(
        &authority,
        WalkReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            end: WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
}

fn accept_interaction(slot: &mut BankSlot, action: &HostAction, tick: u64) {
    let authority = action.authority();
    slot.slot.complete_native_interaction(
        &authority,
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
}

struct ReachFixture {
    base_x: i32,
    base_z: i32,
    width: i32,
    height: i32,
    walkable: Vec<u32>,
    reachable: Vec<u32>,
    reachable_adj: Vec<u32>,
    exact_rank: Vec<u16>,
    adjacent_rank: Vec<u16>,
    step: Vec<u8>,
    canlight: Vec<u32>,
}

impl ReachFixture {
    fn open(base_x: i32, base_z: i32, width: i32, height: i32) -> Self {
        let cells = usize::try_from(width * height).unwrap();
        let words = cells.div_ceil(32);
        Self {
            base_x,
            base_z,
            width,
            height,
            walkable: vec![u32::MAX; words],
            reachable: vec![u32::MAX; words],
            reachable_adj: vec![u32::MAX; words],
            exact_rank: vec![0; cells],
            adjacent_rank: vec![0; cells],
            step: vec![u8::MAX; cells],
            canlight: vec![0; words],
        }
    }

    fn view(&self) -> ReachViewInput<'_> {
        ReachViewInput {
            available: true,
            base_x: self.base_x,
            base_z: self.base_z,
            level: 0,
            width: self.width,
            height: self.height,
            walkable: &self.walkable,
            reachable: &self.reachable,
            reachable_adj: &self.reachable_adj,
            exact_rank: &self.exact_rank,
            adjacent_rank: &self.adjacent_rank,
            step: &self.step,
            canlight: &self.canlight,
            stamp: 1,
        }
    }
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3010,
            z: 3352,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: 0,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("bot"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        nearest_booth: None,
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn loc_row<'a>(
    id: i32,
    name: Option<&'a str>,
    x: i32,
    z: i32,
    distance: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: id,
        id,
        name,
        x,
        z,
        level: 0,
        distance,
        health: -1,
        max_health: -1,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
        shape: 10,
        angle: 0,
    }
}

fn tile(x: i32, z: i32) -> TileInput {
    TileInput { x, z, level: 0 }
}

const OPEN_NEAREST: &str = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await Bank.openNearest('Bank booth', 'Use-quickly');
    }
}
"#;

const BANKING_OPEN: &str = r#"
import { Banking } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Banking.open();
    }
}
"#;

const OPEN_BOOTH: &str = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await Bank.openBooth(
            { x: 3013, z: 3355, level: 0 },
            'Bank booth',
            'Use-quickly',
        );
    }
}
"#;

fn east_booth<'a>(actions: &'a [String]) -> SceneEntityInput<'a> {
    loc_row(2213, Some("Bank booth"), 3011, 3354, 2, actions)
}

#[test]
fn named_open_nearest_from_chebyshev_2_queues_walk_near_then_same_named_open_booth() {
    let mut slot = spawn(OPEN_NEAREST);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), serde_json::Value::Null);
    let walk = one_action(
        &mut slot,
        "Chebyshev 2 must walk to the producer destination",
    );
    let HostEffect::Walk(request) = &walk.effect else {
        panic!("expected native walk");
    };
    assert_eq!(
        request.target,
        WorldTile {
            x: 3011,
            z: 3353,
            level: 0
        }
    );
    assert_eq!(request.radius, 0);
    accept_walk(&mut slot, &walk, 1);

    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 2, &snap);
    let open = one_action(&mut slot, "the selected booth must open after arrival");
    assert!(
        matches!(
            &open.effect,
            HostEffect::Interaction(InteractReq::OpenBooth {
                x: 3011,
                z: 3354,
                level: 0,
                id: 2213,
                name: Some(name),
                action: Some(action),
            }) if name == "Bank booth" && action == "Use-quickly"
        ),
        "click must reuse the same id/name/action/tile, not a later nearest"
    );
    slot.stop();
}

#[test]
fn native_bank_open_does_not_roundtrip_js_snapshot_collections() {
    let mut slot = spawn(OPEN_NEAREST);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    slot.probe(
        r#"(() => {
            Object.defineProperties(globalThis.__rs2b0t_host.snapshot, {
                locs: { get() { throw new Error('locs crossed JS/native seam'); } },
                banks: { get() { throw new Error('banks crossed JS/native seam'); } },
            });
            return true;
        })()"#,
    )
    .unwrap();

    tick(&mut slot, 1, &snap);
    let walk = one_action(
        &mut slot,
        "native bank open must read GameSnapshot, not JS collections",
    );
    assert!(matches!(
        &walk.effect,
        HostEffect::Walk(request)
            if request.target == WorldTile { x: 3011, z: 3353, level: 0 }
                && request.radius == 0
    ));
    slot.stop();
}

#[test]
fn named_open_nearest_from_chebyshev_1_queues_named_open_booth_only() {
    let mut slot = spawn(OPEN_NEAREST);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.here = Some(tile(3011, 3353));
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let open = one_action(&mut slot, "an operable booth must not start a walk");
    assert!(
        matches!(
            &open.effect,
            HostEffect::Interaction(InteractReq::OpenBooth { id: 2213, name: Some(name), action: Some(op), .. })
                if name == "Bank booth" && op == "Use-quickly"
        ),
        "projected can_operate must not walk"
    );
    slot.stop();
}

#[test]
fn named_open_nearest_missing_loc_after_approach_sends_no_click() {
    let mut slot = spawn(OPEN_NEAREST);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let walk = one_action(&mut slot, "first approach the selected booth");
    assert!(matches!(&walk.effect, HostEffect::Walk(_)));
    accept_walk(&mut slot, &walk, 1);

    let empty: [SceneEntityInput; 0] = [];
    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    snap.locs = &empty;
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 2, &snap);
    assert!(
        slot.take_actions().is_empty(),
        "missing loc must not invent a click"
    );
    assert_eq!(slot.probe("__ok").unwrap(), false);
    slot.stop();
}

#[test]
fn named_open_nearest_replaced_loc_after_approach_sends_no_click() {
    let mut slot = spawn(OPEN_NEAREST);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let walk = one_action(&mut slot, "first approach the selected booth");
    assert!(matches!(&walk.effect, HostEffect::Walk(_)));
    accept_walk(&mut slot, &walk, 1);

    let replaced = [
        loc_row(9999, Some("Bank booth"), 3011, 3354, 1, &use_quickly),
        loc_row(2213, Some("Bank booth"), 3012, 3353, 1, &use_quickly),
    ];
    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    snap.locs = &replaced;
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 2, &snap);
    assert!(
        slot.take_actions().is_empty(),
        "replaced id or a later matching booth must not receive the click"
    );
    assert_eq!(slot.probe("__ok").unwrap(), false);
    slot.stop();
}

#[test]
fn unnamed_banking_open_continues_through_omitted_locs_to_fresh_bank() {
    let mut slot = spawn(BANKING_OPEN);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [loc_row(
        2213,
        Some("Bank booth"),
        200,
        100,
        100,
        &use_quickly,
    )];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(95, 90, 110, 30);
    snap.here = Some(tile(100, 100));
    snap.locs = &locs;
    snap.reach = reach.view();
    let mut encoder = IsolateBuf::new();
    let mut last = None;
    post_snapshot_delta_native(&mut slot, &mut encoder, &mut last, &snap);
    tick(&mut slot, 1, &snap);
    let walk = one_action(
        &mut slot,
        "unnamed banking must approach the observed booth",
    );
    let HostEffect::Walk(request) = &walk.effect else {
        panic!("expected booth walk");
    };
    assert_eq!(
        request.target,
        WorldTile {
            x: 200,
            z: 100,
            level: 0
        }
    );
    assert_eq!(request.radius, 1);
    accept_walk(&mut slot, &walk, 1);

    snap.tick = 2;
    snap.here = Some(tile(199, 100));
    post_snapshot_delta_native(&mut slot, &mut encoder, &mut last, &snap);
    tick(&mut slot, 2, &snap);
    let open = one_action(
        &mut slot,
        "omitted locs delta must preserve the selected booth",
    );
    assert!(
        matches!(
            &open.effect,
            HostEffect::Interaction(InteractReq::OpenBooth {
                x: 200,
                z: 100,
                level: 0,
                id: 2213,
                name: None,
                action: None,
            })
        ),
        "unnamed continuation must reuse the selected id/tile without inventing name/action"
    );
    accept_interaction(&mut slot, &open, 2);

    snap.tick = 3;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    slot.post_snapshot(
        &snap,
        NativeFactsInput {
            bank_snapshot_generation: Some(1),
            ..Default::default()
        },
    );
    tick(&mut slot, 3, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), true);
    slot.stop();
}

// A second open supersedes the first and owns the only native approach.
#[test]
fn a_second_open_supersedes_the_first() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        const first = Bank.openNearest('Bank booth', 'Use-quickly');
        const second = Bank.openNearest('Bank booth', 'Use-quickly');
        first.then((r) => { globalThis.__first = r; });
        second.then((r) => { globalThis.__second = r; });
        await Promise.all([first, second]);
    }
}
"#;
    let mut slot = spawn(src);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    assert_eq!(slot.probe("globalThis.__first").unwrap(), false);
    assert_eq!(
        slot.probe("globalThis.__second ?? null").unwrap(),
        serde_json::Value::Null
    );
    let walk = one_action(&mut slot, "only the current open may own the walk");
    let HostEffect::Walk(request) = &walk.effect else {
        panic!("expected active native walk");
    };
    assert_eq!(
        request.target,
        WorldTile {
            x: 3011,
            z: 3353,
            level: 0
        }
    );
    assert_eq!(request.radius, 0);
    accept_walk(&mut slot, &walk, 1);

    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 2, &snap);
    let open = one_action(&mut slot, "the active open must click its selected booth");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth { id: 2213, .. })
    ));
    accept_interaction(&mut slot, &open, 2);

    snap.tick = 3;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    slot.post_snapshot(
        &snap,
        NativeFactsInput {
            bank_snapshot_generation: Some(1),
            ..Default::default()
        },
    );
    tick(&mut slot, 3, &snap);
    assert_eq!(slot.probe("globalThis.__second").unwrap(), true);
    assert_eq!(slot.probe("globalThis.__first").unwrap(), false);
    assert!(slot.take_actions().is_empty());
    slot.stop();
}
#[test]
fn named_open_nearest_completes_on_fresh_generation_not_queued_click() {
    let mut slot = spawn(OPEN_NEAREST);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.here = Some(tile(3011, 3353));
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), serde_json::Value::Null);
    let open = one_action(&mut slot, "named open must post the selected booth click");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth { id: 2213, .. })
    ));
    accept_interaction(&mut slot, &open, 1);

    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = false;
    snap.bank_generation = 1;
    slot.post_snapshot(
        &snap,
        NativeFactsInput {
            bank_snapshot_generation: Some(0),
            ..Default::default()
        },
    );
    tick(&mut slot, 2, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), serde_json::Value::Null);
    assert!(
        slot.take_actions().is_empty(),
        "open without loaded stock is not ready"
    );

    snap.tick = 3;
    snap.bank_loaded = true;
    slot.post_snapshot(
        &snap,
        NativeFactsInput {
            bank_snapshot_generation: Some(1),
            ..Default::default()
        },
    );
    tick(&mut slot, 3, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), true);
    assert!(
        slot.take_actions().is_empty(),
        "fresh generation completes without re-click"
    );
    slot.stop();
}
#[test]
fn supplied_stand_walks_then_delivers_fresh_bank_result() {
    let mut slot = spawn(OPEN_BOOTH);
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [loc_row(
        2213,
        Some("Bank booth"),
        3013,
        3354,
        4,
        &use_quickly,
    )];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3008, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_input(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let walk = one_action(&mut slot, "supplied stand must be approached natively");
    let HostEffect::Walk(request) = &walk.effect else {
        panic!("expected walk to supplied stand");
    };
    assert_eq!(
        request.target,
        WorldTile {
            x: 3013,
            z: 3355,
            level: 0
        }
    );
    assert_eq!(request.radius, 1);
    accept_walk(&mut slot, &walk, 1);

    snap.tick = 2;
    snap.here = Some(tile(3013, 3355));
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 2, &snap);
    let open = one_action(&mut slot, "arrived stand must open the same booth");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth {
            x: 3013,
            z: 3354,
            level: 0,
            id: 2213,
            name: Some(name),
            action: Some(action),
        }) if name == "Bank booth" && action == "Use-quickly"
    ));
    assert!(slot.probe("__ok").unwrap().is_null());
    accept_interaction(&mut slot, &open, 2);

    snap.tick = 3;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    slot.post_snapshot(
        &snap,
        NativeFactsInput {
            bank_snapshot_generation: Some(1),
            ..Default::default()
        },
    );
    tick(&mut slot, 3, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), true);
    slot.stop();
}

/// A chest access row interacts with its native loc identity and waits for a loaded bank.
#[test]
fn open_nearest_access_chest_row_interacts_with_the_named_loc() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Bank.openNearestAccess({ name: 'Shantay chest', op: 'Open' });
    }
}
"#;
    let mut slot = spawn(src);
    let actions = ["Open".to_string()];
    let locs = [loc_row(
        2693,
        Some("Shantay chest"),
        3309,
        3120,
        1,
        &actions,
    )];
    let mut snap = base_snapshot();
    snap.here = Some(tile(3308, 3120));
    snap.locs = &locs;
    post_snapshot_input(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let open = one_action(
        &mut slot,
        "native access must interact with the selected chest",
    );
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::Loc {
            x: 3309,
            z: 3120,
            level: 0,
            action,
            id: Some(2693),
        }) if action == "Open"
    ));
    assert_eq!(slot.probe("typeof __ok").unwrap(), "undefined");
    accept_interaction(&mut slot, &open, 1);

    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    slot.post_snapshot(
        &snap,
        NativeFactsInput {
            bank_snapshot_generation: Some(1),
            ..Default::default()
        },
    );
    tick(&mut slot, 2, &snap);
    assert_eq!(slot.probe("__ok").unwrap(), true);
    assert!(slot.take_actions().is_empty());
    slot.stop();
}
/// An NPC access row talks, selects the matching option, and waits for loaded stock.
#[test]
fn open_npc_access_row_talks_then_answers_the_access_option() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Bank.openNpcAccess(
            { name: 'Banker', op: 'Bank', choose: "I'd like to access my bank account" },
        );
    }
}
"#;
    let mut slot = spawn(src);
    let actions = ["Talk-to".to_string(), "Bank".to_string()];
    let mut banker = loc_row(2127, Some("Banker"), 2852, 2954, 2, &actions);
    banker.index = 17;
    let npcs = [banker];
    let mut snap = base_snapshot();
    snap.npcs = &npcs;
    post_snapshot_input(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let talk = one_action(
        &mut slot,
        "native NPC access must talk to the matching banker",
    );
    assert!(matches!(
        &talk.effect,
        HostEffect::Interaction(InteractReq::Npc { name, action, index })
            if name == "Banker" && action == "Bank" && *index == Some(17)
    ));
    accept_interaction(&mut slot, &talk, 1);

    let options = [
        ChatOptionInput {
            text: "Nothing thanks",
            com_id: 2493,
        },
        ChatOptionInput {
            text: "I'd like to access my bank account, please.",
            com_id: 2494,
        },
    ];
    snap.tick = 2;
    snap.chat_modal_id = 2492;
    snap.chat_open = true;
    snap.chat_options = &options;
    post_snapshot_input(&mut slot, &snap);
    tick(&mut slot, 2, &snap);
    let answer = one_action(&mut slot, "NPC access must answer the matching option");
    assert!(
        matches!(
            &answer.effect,
            HostEffect::Interaction(InteractReq::Answer { option: 2 })
        ),
        "the option containing `choose`, 1-based"
    );
    accept_interaction(&mut slot, &answer, 2);

    snap.chat_modal_id = -1;
    snap.chat_open = false;
    snap.chat_options = &[];
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    for tick_number in 3..=6 {
        snap.tick = tick_number;
        slot.post_snapshot(
            &snap,
            NativeFactsInput {
                bank_snapshot_generation: Some(1),
                ..Default::default()
            },
        );
        tick(&mut slot, tick_number, &snap);
    }
    assert_eq!(slot.probe("__ok").unwrap(), true);
    assert!(slot.take_actions().is_empty());
    slot.stop();
}
#[test]
fn a_named_approach_opens_the_same_identity_across_an_omitted_locs_delta() {
    let mut slot = spawn(OPEN_NEAREST);
    let quick = ["Use-quickly".to_string()];
    let examine_quick = ["Examine".to_string(), "Use-quickly".to_string()];
    let initial_locs = [
        loc_row(111, Some("Bank booth"), 3010, 3353, 9, &quick),
        loc_row(2213, Some("Bank booth"), 3011, 3354, 2, &examine_quick),
    ];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.bank_generation = 7;
    snap.locs = &initial_locs;
    snap.reach = reach.view();
    let mut encoder = IsolateBuf::new();
    let mut last = None;
    post_snapshot_delta_native(&mut slot, &mut encoder, &mut last, &snap);
    tick(&mut slot, 1, &snap);
    let walk = one_action(&mut slot, "nearest named booth must be approached");
    assert!(matches!(
        &walk.effect,
        HostEffect::Walk(request)
            if request.target == (WorldTile { x: 3011, z: 3353, level: 0 })
                && request.radius == 0
    ));
    accept_walk(&mut slot, &walk, 1);

    // The delta omits the locs page; the native snapshot still owns the
    // original scene row and the click must reuse that selected identity.
    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    post_snapshot_delta_native(&mut slot, &mut encoder, &mut last, &snap);
    tick(&mut slot, 2, &snap);
    let open = one_action(
        &mut slot,
        "the omitted locs delta must preserve the selected booth",
    );
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth {
            x: 3011,
            z: 3354,
            id: 2213,
            name: Some(name),
            action: Some(action),
            ..
        }) if name == "Bank booth" && action == "Use-quickly"
    ));
    slot.stop();
}
#[test]
fn session_reset_clears_the_native_bank_observation() {
    let mut slot = spawn(OPEN_NEAREST);
    let quick = ["Use-quickly".to_string()];
    let locs = [east_booth(&quick)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(3005, 3348, 16, 16);
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_input(&mut slot, &snap);
    slot.reset_session_work();

    tick(&mut slot, 1, &snap);
    assert_eq!(
        slot.probe("__ok").unwrap(),
        serde_json::Value::Bool(false),
        "a forgotten scene has no booth to open"
    );
    assert!(slot.take_actions().is_empty());
    slot.stop();
}
#[test]
fn bank_access_already_adjacent_does_not_rearm_navigation() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() { await Bank.openNearestAccess({name:'Bank booth',op:'Use-quickly'}); }
}
"#;
    let mut slot = spawn(src);
    let quick = ["Use-quickly".to_string()];
    let locs = [loc_row(2213, Some("Bank booth"), 11, 10, 1, &quick)];
    let mut snap = base_snapshot();
    let reach = ReachFixture::open(10, 10, 2, 1);
    snap.here = Some(tile(10, 10));
    snap.locs = &locs;
    snap.reach = reach.view();
    post_snapshot_native(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    let action = one_action(
        &mut slot,
        "already-adjacent bank access must not walk again",
    );
    assert!(matches!(
        &action.effect,
        HostEffect::Interaction(InteractReq::OpenBooth { .. })
    ));
    slot.stop();
}

/// The default booth's `isArrived` pre-check respects a blocked edge.
#[test]
fn bank_access_default_booth_across_a_wall_edge_walks_first() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() { await Bank.openNearestAccess({name:'Bank booth',op:'Use-quickly'}); }
}
"#;
    let view = |reachable_adj: &'static [u32], adjacent_rank: &'static [u16]| ReachViewInput {
        available: true,
        base_x: 10,
        base_z: 10,
        level: 0,
        width: 2,
        height: 1,
        walkable: &[0b01],
        reachable: &[0b01],
        reachable_adj,
        exact_rank: &[0, u16::MAX],
        adjacent_rank,
        step: &[0, 0],
        canlight: &[],
        stamp: 0,
    };
    let run = |reach: ReachViewInput<'static>| {
        let mut slot = spawn(src);
        let actions = ["Use-quickly".to_string()];
        let locs = [loc_row(2213, Some("Bank booth"), 11, 10, 1, &actions)];
        let mut snap = base_snapshot();
        snap.here = Some(tile(10, 10));
        snap.locs = &locs;
        snap.reach = reach;
        post_snapshot_native(&mut slot, &snap);
        tick(&mut slot, 1, &snap);
        let action = one_action(
            &mut slot,
            "wall-edge fixture must produce one native action",
        );
        slot.stop();
        action
    };
    let blocked = run(view(&[0b01], &[0, u16::MAX]));
    assert!(
        matches!(
            &blocked.effect,
            HostEffect::Walk(request)
                if request.target == (WorldTile { x: 11, z: 10, level: 0 })
                    && request.radius == 1
        ),
        "Chebyshev 1 across a wall is not arrived"
    );
    let open = run(view(&[0b11], &[0, 1]));
    assert!(
        matches!(
            &open.effect,
            HostEffect::Interaction(InteractReq::OpenBooth { x: 11, z: 10, .. })
        ),
        "a live adjacent reach opens without another walk"
    );
}
/// Frozen calls the openers' `log?.()` without awaiting it.
#[test]
fn opener_log_promises_are_not_awaited() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        const log = (m) => { (globalThis.__lines ||= []).push(m); return new Promise(() => {}); };
        globalThis.__chest = await Bank.openNearestAccess({ name: 'Shantay chest', op: 'Open' }, log);
        globalThis.__npc = await Bank.openNpcAccess(
            { name: 'Banker', op: 'Bank', choose: 'access' }, log);
    }
}
"#;
    let mut slot = spawn(src);
    let mut snap = base_snapshot();
    post_snapshot_input(&mut slot, &snap);
    tick(&mut slot, 1, &snap);
    assert_eq!(
        slot.probe("__chest").unwrap(),
        false,
        "no chest: frozen false"
    );
    for tick_number in 2..=6u64 {
        snap.tick = tick_number;
        post_snapshot_input(&mut slot, &snap);
        tick(&mut slot, tick_number, &snap);
    }
    assert_eq!(
        slot.probe("__npc").unwrap(),
        false,
        "three banker attempts, then false"
    );
    let lines = slot.probe("__lines").unwrap();
    assert_eq!(lines[0], "no usable 'Shantay chest' in the scene");
    assert!(
        lines
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line == "could not get Banker to open the bank"),
        "{lines}"
    );
    assert!(slot.take_actions().is_empty());
    slot.stop();
}
