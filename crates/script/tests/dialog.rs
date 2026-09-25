// ShopBuyout `talkThrough` / `openDialogue` / `driveDialog`: the production
// import marshals into Rust-owned sequencing; queued npc/continue/answer
// verbs are the existing FlatBuffer actions, not a completed dialogue.

use script::isolate_fb::{
    encode_snapshot, ChatOptionInput, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn npc<'a>(name: &'a str, actions: &'a [String], index: i32) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id: 1,
        name: Some(name),
        x: 2532,
        z: 4712,
        level: 0,
        distance: 1,
        health: 100,
        max_health: 100,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 1,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    }
}

fn base<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2531,
            z: 4712,
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
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
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
    }
}

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    iso.post_snapshot(encode_snapshot(snap));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const TALK: &str = r#"
import { talkThrough } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__logs = [];
        try {
            globalThis.__ok = await talkThrough(
                globalThis.__npc || 'Gundai',
                globalThis.__prefer || ['access my bank'],
                (m) => globalThis.__logs.push(String(m)),
            );
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const OPEN: &str = r#"
import { openDialogue } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__logs = [];
        try {
            globalThis.__ok = await openDialogue('Gundai', (m) => globalThis.__logs.push(String(m)));
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const STUBS: &str = r#"
import { talkChoosingBy, talkStrict } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__choose = null;
        globalThis.__strict = null;
        try { globalThis.__choose = await talkChoosingBy('Gundai', [], ['access my bank'], () => {}); }
        catch (e) { globalThis.__choose = String(e.message || e); }
        try { globalThis.__strict = await talkStrict('Missing', ['access my bank'], () => {}); }
        catch (e) { globalThis.__strict = String(e.message || e); }
    }
}
"#;

#[test]
fn talk_through_opens_continues_prefers_then_completes_on_partial_bank() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Gundai".into(),
            action: "Talk-to".into(),
            index: Some(7),
        }],
        "the production talkThrough import must queue the existing npc verb"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    snap.tick = 2;
    snap.chat_open = true;
    snap.chat_modal_id = 968;
    snap.chat_continue = true;
    snap.chat_text = Some("Hello, what are you doing out here?");
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::ContinueDialog],
        "the first continue page must use the existing continue verb"
    );

    let choice = [ChatOptionInput {
        text: "I'd like to access my bank account, please.",
    }];
    snap.tick = 3;
    snap.chat_continue = false;
    snap.chat_options = &choice;
    snap.chat_modal_id = 4882;
    post(&iso, &snap);
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "continue ack (modal change) still waits the frozen extra tick"
    );

    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Answer { option: 1 }],
        "preferred 'access my bank' must answer the posted choice"
    );

    snap.tick = 5;
    snap.chat_open = false;
    snap.chat_modal_id = -1;
    snap.chat_options = &[];
    snap.bank_open = true;
    snap.bank_loaded = false;
    post(&iso, &snap);
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(
        iso.drain_interacts().is_empty(),
        "choice ack still waits the frozen two ticks"
    );

    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    assert!(iso.drain_interacts().is_empty());
    snap.tick = 7;
    post(&iso, &snap);
    tick(&iso, 7);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Bool(true),
        "chat close + bank component is a completed talkThrough"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn open_dialogue_waits_for_the_observed_chat_and_does_not_talk_again() {
    let iso = spawn(OPEN);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 3)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Gundai".into(),
            action: "Talk-to".into(),
            index: Some(3),
        }]
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    snap.tick = 2;
    snap.chat_modal_id = 968;
    snap.chat_continue = true;
    snap.chat_open = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(
        iso.drain_interacts().is_empty(),
        "openDialogue must not continue or re-Talk-to after the page lands"
    );
    iso.join();
}

#[test]
fn pause_latch_and_pending_do_not_duplicate_or_re_talk() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    snap.tick = 2;
    snap.chat_open = true;
    snap.chat_modal_id = 968;
    snap.chat_continue = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "hidden continue id must not Talk-to or continue"
    );

    snap.tick = 3;
    snap.ours = true;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn pause_hold_and_session_reset_drop_stale_dialog_actions() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    iso.pause();
    snap.tick = 2;
    snap.chat_modal_id = 968;
    snap.chat_continue = true;
    snap.chat_open = true;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());

    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn missing_npc_and_unmatched_fallback_are_honest() {
    let iso = spawn(TALK);
    let mut snap = base();
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    let logs = iso.probe("__logs").unwrap();
    assert!(
        logs.as_array()
            .unwrap()
            .iter()
            .any(|row| row.as_str().unwrap_or("").contains("no 'Gundai' nearby")),
        "missing NPC must log the canonical line, got {logs:?}"
    );
    iso.join();

    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    iso.drain_interacts();

    let opts = [
        ChatOptionInput { text: "Hello" },
        ChatOptionInput { text: "Goodbye" },
    ];
    snap.tick = 2;
    snap.chat_modal_id = 200;
    snap.chat_open = true;
    snap.chat_options = &opts;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Answer { option: 2 }],
        "unmatched drive takes the last posted option"
    );
    let logs = iso.probe("__logs").unwrap();
    assert!(
        logs.as_array()
            .unwrap()
            .iter()
            .any(|row| row.as_str().unwrap_or("").contains("taking the last")),
        "unmatched preferred must keep the canonical WARN, got {logs:?}"
    );
    iso.join();
}

#[test]
fn talk_choosing_by_stays_not_impl_and_talk_strict_stays_the_talk_through_alias() {
    let iso = spawn(STUBS);
    post(&iso, &base());
    tick(&iso, 1);
    let choose = iso.probe("__choose").unwrap();
    assert!(
        choose.as_str().unwrap_or("").contains("not impl"),
        "talkChoosingBy stays incomplete, got {choose:?}"
    );
    assert_eq!(
        iso.probe("__strict").unwrap(),
        Value::Bool(false),
        "talkStrict keeps the existing talkThrough alias, including false on a missing NPC"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn same_continue_page_does_not_duplicate_then_transitions() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    iso.drain_interacts();

    snap.tick = 2;
    snap.chat_open = true;
    snap.chat_modal_id = 968;
    snap.chat_continue = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::ContinueDialog]);

    for n in 3..=6u64 {
        snap.tick = n;
        post(&iso, &snap);
        tick(&iso, n);
        assert!(
            iso.drain_interacts().is_empty(),
            "same continue page at tick {n} must not re-press"
        );
        assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    }

    let choice = [ChatOptionInput {
        text: "I'd like to access my bank account, please.",
    }];
    snap.tick = 7;
    snap.chat_continue = false;
    snap.chat_modal_id = 4882;
    snap.chat_options = &choice;
    post(&iso, &snap);
    tick(&iso, 7);
    assert!(
        iso.drain_interacts().is_empty(),
        "acked continue still waits one tick"
    );
    snap.tick = 8;
    post(&iso, &snap);
    tick(&iso, 8);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Answer { option: 1 }]
    );
    iso.join();
}

#[test]
fn same_options_page_after_answer_does_not_duplicate() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let choice = [ChatOptionInput {
        text: "I'd like to access my bank account, please.",
    }];
    let mut snap = base();
    snap.npcs = &npcs;
    snap.chat_open = true;
    snap.chat_modal_id = 4882;
    snap.chat_options = &choice;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Answer { option: 1 }]
    );

    for n in 2..=5u64 {
        snap.tick = n;
        post(&iso, &snap);
        tick(&iso, n);
        assert!(
            iso.drain_interacts().is_empty(),
            "same options page at tick {n} must not answer again"
        );
        assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    }

    snap.tick = 6;
    snap.chat_open = false;
    snap.chat_modal_id = -1;
    snap.chat_options = &[];
    snap.bank_open = true;
    post(&iso, &snap);
    tick(&iso, 6);
    snap.tick = 7;
    post(&iso, &snap);
    tick(&iso, 7);
    snap.tick = 8;
    post(&iso, &snap);
    tick(&iso, 8);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn continue_ack_timeout_fails_without_repressing() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    iso.drain_interacts();

    snap.tick = 2;
    snap.chat_open = true;
    snap.chat_modal_id = 968;
    snap.chat_continue = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::ContinueDialog]);

    snap.tick = 3;
    post(&iso, &snap);
    tick(&iso, 3);
    assert!(iso.drain_interacts().is_empty());
    std::thread::sleep(std::time::Duration::from_millis(3_100));
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(
        iso.drain_interacts().is_empty(),
        "ack timeout must not spam continue"
    );
    iso.join();
}

#[test]
fn empty_earlier_option_keeps_the_posted_answer_index() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let opts = [
        ChatOptionInput { text: "" },
        ChatOptionInput {
            text: "I'd like to access my bank account, please.",
        },
    ];
    let mut snap = base();
    snap.npcs = &npcs;
    snap.chat_open = true;
    snap.chat_modal_id = 4882;
    snap.chat_options = &opts;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Answer { option: 2 }],
        "empty first slot must not collapse the preferred answer to 1"
    );
    iso.join();
}

#[test]
fn pause_and_reset_during_continue_ack_drop_stale_actions() {
    let iso = spawn(TALK);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Gundai", &actions, 7)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    iso.drain_interacts();

    snap.tick = 2;
    snap.chat_open = true;
    snap.chat_modal_id = 968;
    snap.chat_continue = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::ContinueDialog]);

    iso.pause();
    snap.tick = 3;
    post(&iso, &snap);
    tick(&iso, 3);
    tick(&iso, 4);
    assert!(iso.drain_interacts().is_empty());
    iso.resume();
    snap.tick = 5;
    post(&iso, &snap);
    tick(&iso, 5);
    assert!(iso.drain_interacts().is_empty());
    iso.reset_session_work();
    snap.tick = 6;
    snap.chat_modal_id = 4882;
    snap.chat_continue = false;
    post(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn pick_preferred_is_the_frozen_pick_and_walk_with_hops_awaits_the_walk() {
    let src = r#"
import { pickPreferred, walkWithHops } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        const opts = ['Yes please.', 'No thanks.'];
        globalThis.__picks = [
            pickPreferred(opts, ['maybe', 'NO']),
            pickPreferred(opts, ['']),
            pickPreferred(opts, ['maybe']),
            pickPreferred(['', 'Bank'], ['', 'bank']),
        ];
        globalThis.__here = null;
        globalThis.__here = await walkWithHops({ x: 2531, z: 4712, level: 0 }, 0, [], () => {});
        globalThis.__far = null;
        globalThis.__far = await walkWithHops({ x: 2560, z: 4712, level: 0 }, 2, [], () => {});
    }
}
"#;
    let iso = spawn(src);
    post(&iso, &base());
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__picks").unwrap(),
        serde_json::json!(["No thanks.", "Yes please.", null, "Bank"])
    );
    assert_eq!(iso.probe("__here").unwrap(), true, "already there");
    assert!(
        matches!(
            iso.drain_interacts().as_slice(),
            [InteractReq::WalkNear {
                x: 2560,
                radius: 2,
                ..
            }]
        ),
        "the far walk is queued"
    );
    assert_eq!(
        iso.probe("__far").unwrap(),
        Value::Null,
        "a queued walk is not an arrival: walkWithHops awaits its result"
    );
    iso.join();
}

#[test]
fn walk_with_hops_log_promise_does_not_hold_the_walk() {
    let src = r#"
import { walkWithHops } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__lines = [];
        globalThis.__ok = null;
        globalThis.__ok = await walkWithHops(
            { x: 2532, z: 9600, level: 0 },
            2,
            [],
            (line) => {
                globalThis.__lines.push(line);
                return new Promise(() => {});
            },
        );
    }
}
"#;
    let iso = spawn(src);
    let mut snap = base();
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__lines").unwrap(),
        serde_json::json!(["no hop from (2531,4712) toward z 9600 — trying the baked graph"])
    );
    assert!(
        matches!(
            iso.drain_interacts().as_slice(),
            [InteractReq::WalkNear {
                x: 2532,
                z: 9600,
                radius: 2,
                ..
            }]
        ),
        "the fallback walk is queued in the log callback's tick"
    );
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 2532,
        z: 9600,
        level: 0,
    });
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        true,
        "the never-settling log promise must not hold the walk result"
    );
    iso.join();
}

#[test]
fn walk_with_hops_climbs_the_nearest_same_side_ladder_then_arrives() {
    let src = r#"
import { walkWithHops } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await walkWithHops({ x: 2532, z: 9600, level: 0 }, 2, [
            { stand: { x: 2540, z: 9600, level: 0 }, locName: 'Ladder', op: 'Climb-up', arrive: { x: 2540, z: 4712, level: 0 } },
            { stand: { x: 2532, z: 4712, level: 0 }, locName: 'Ladder', op: 'Climb-down', arrive: { x: 2532, z: 9601, level: 0 } },
        ], () => {});
    }
}
"#;
    let iso = spawn(src);
    let ops = ["Climb-down".to_string()];
    let mut ladder = npc("Ladder", &ops, 0);
    ladder.id = 1759;
    ladder.x = 2532;
    ladder.z = 4713;
    let locs = [ladder];
    let mut snap = base();
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Loc {
            x: 2532,
            z: 4713,
            level: 0,
            action: "Climb-down".into(),
            id: Some(1759),
        }],
        "the surface hop's ladder, not the underground one"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 2532,
        z: 9601,
        level: 0,
    });
    snap.locs = &[];
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), true, "landed within radius");
    assert!(iso.drain_interacts().is_empty(), "no final walk needed");
    iso.join();
}

#[test]
fn walk_with_hops_walks_to_the_stand_then_opens_and_waits_the_frozen_ticks_to_climb() {
    let src = r#"
import { walkWithHops } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await walkWithHops({ x: 2532, z: 9600, level: 0 }, 2, [
            { stand: { x: 2532, z: 4712, level: 0 }, locName: 'Trapdoor', op: 'Climb-down',
              open: 'Open', arrive: { x: 2532, z: 9601, level: 0 } },
        ], () => {});
    }
}
"#;
    let iso = spawn(src);
    let shut = ["Open".to_string()];
    let opened = ["Climb-down".to_string()];
    let trapdoor = |ops| {
        let mut loc = npc("Trapdoor", ops, 0);
        loc.id = 1570;
        loc.z = 4713;
        loc
    };
    let closed_locs = [trapdoor(&shut)];
    let open_locs = [trapdoor(&opened)];
    let mut snap = base();
    snap.here = Some(TileInput {
        x: 2500,
        z: 4712,
        level: 0,
    });
    snap.locs = &closed_locs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert!(
        matches!(
            iso.drain_interacts().as_slice(),
            [InteractReq::WalkNear {
                x: 2532,
                z: 4712,
                radius: 2,
                ..
            }]
        ),
        "far from the hop: walk to its stand first"
    );
    let loc_op = |action: &str| InteractReq::Loc {
        x: 2532,
        z: 4713,
        level: 0,
        action: action.into(),
        id: Some(1570),
    };
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 2531,
        z: 4712,
        level: 0,
    });
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![loc_op("Open")],
        "no ladder yet: open"
    );
    // Frozen: delayTicks(2), re-find, delayTicks(2), re-find.
    for n in 3..=5 {
        snap.tick = n;
        snap.locs = if n == 5 { &open_locs } else { &closed_locs };
        post(&iso, &snap);
        tick(&iso, n);
        assert!(iso.drain_interacts().is_empty(), "tick {n}: still waiting");
    }
    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    assert_eq!(iso.drain_interacts(), vec![loc_op("Climb-down")]);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    snap.tick = 7;
    snap.here = Some(TileInput {
        x: 2532,
        z: 9601,
        level: 0,
    });
    snap.locs = &[];
    post(&iso, &snap);
    tick(&iso, 7);
    tick(&iso, 8);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    iso.join();
}
