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
