//! Portable isolate tests for native skill.xp / inventory.changed delivery.

use std::path::PathBuf;

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta, ChatLineInput, ItemRowInput, ReachViewInput,
    SnapshotInput, StatInput,
};
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: None,
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
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
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
        widgets: &[],
    }
}

fn bone(slot: i32) -> ItemRowInput<'static> {
    ItemRowInput {
        name: Some("Bones"),
        count: 1,
        id: 526,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
    }
}

fn prayer(xp: i32) -> StatInput<'static> {
    StatInput {
        index: 5,
        name: "prayer",
        xp,
        base: 2,
        effective: 2,
    }
}

fn listener_src() -> &'static str {
    r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('skill.xp', (e) => {
            globalThis.__order = globalThis.__order || [];
            globalThis.__order.push('xp');
            globalThis.__xp = (globalThis.__xp || 0) + e.delta;
            globalThis.__xpName = e.name;
            globalThis.__xpSkill = e.skill;
        });
        this.on('inventory.changed', (e) => {
            globalThis.__order = globalThis.__order || [];
            globalThis.__order.push('inv');
            if (e.id === -1 && e.previousId !== -1) {
                globalThis.__buried = (globalThis.__buried || 0) + 1;
            }
            globalThis.__lastSlot = e.slot;
            globalThis.__lastName = e.name;
        });
        this.on('skill.level', () => { globalThis.__level = (globalThis.__level || 0) + 1; });
    }
    loop() {}
}
"#
}

fn spawn_listeners() -> LoadIsolate {
    LoadIsolate::spawn(listener_src().to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn examplebot_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ExampleBot.ts");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let pin = script::js_cache::JsCache::origin_sha(&bytes);
    assert_eq!(
        pin, "8e8e27cd9c2e57cd6478132e4bfaf910678664898db44915acc08ffc501c0973",
        "tracked ExampleBot fixture drifted from rs2b0t 96410ec5 script-template"
    );
    let src = String::from_utf8(bytes).expect("ExampleBot utf-8");
    script::transpile_ts(&src).expect("transpile ExampleBot")
}

fn probe_alive(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr)
        .unwrap_or_else(|e| panic!("isolate unusable for {expr}: {e}"))
}

fn probe_i64(iso: &LoadIsolate, expr: &str) -> i64 {
    iso.probe(expr)
        .unwrap_or(serde_json::Value::Null)
        .as_i64()
        .unwrap_or(0)
}

#[test]
fn seed_then_bury_fires_callbacks_after_tick() {
    let iso = spawn_listeners();
    let bones: Vec<_> = (0..25).map(bone).collect();
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0, "seed must not fire");
    assert_eq!(probe_i64(&iso, "__xp||0"), 0);

    let stats1 = [prayer(224)];
    snap.inv = &[];
    snap.stats = &stats1;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    let _ = iso.probe("1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "snapshot must not execute callbacks"
    );
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 25);
    assert_eq!(probe_i64(&iso, "__xp||0"), 112);
    assert_eq!(iso.probe("__xpName").unwrap().as_str(), Some("prayer"));
    assert_eq!(probe_i64(&iso, "__xpSkill"), 5);
    let order = iso.probe("__order").unwrap();
    let rows = order.as_array().expect("order list");
    assert_eq!(rows[0].as_str(), Some("xp"));
    assert!(rows.iter().skip(1).all(|v| v.as_str() == Some("inv")));
    assert_eq!(probe_i64(&iso, "__level||0"), 0);
    iso.join();
}

#[test]
fn unchanged_examplebot_counts_delta_from_seeded_xp() {
    let js = examplebot_source();
    let shape = script::load::detect_shape(&js);
    let iso = LoadIsolate::spawn(js, shape, vec![]).unwrap();
    let bones: Vec<_> = (0..25).map(bone).collect();
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    snap.stats = &stats0;
    snap.ingame = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "(__rs_bot && __rs_bot.buried) || 0"), 0);

    let stats1 = [prayer(224)];
    snap.inv = &[];
    snap.stats = &stats1;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__rs_bot.buried"), 25);
    assert_eq!(probe_i64(&iso, "__rs_bot.xpGained"), 112);
    iso.join();
}

#[test]
fn sparse_omit_does_not_fire_inventory() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    let (kf, fp) = encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(kf);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.tick = 2;
    snap.hold = true;
    let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0);
    iso.join();
}

#[test]
fn identical_inv_does_not_refire() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 1);
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    let _ = iso.probe("1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        1,
        "identical repost must not refire"
    );
    iso.join();
}

#[test]
fn valid_empty_fires_one_slot() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 1);
    assert_eq!(iso.probe("__lastName").unwrap(), serde_json::Value::Null);
    iso.join();
}

#[test]
fn throw_isolates_sibling_listeners() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('skill.xp', () => { throw new Error('xp-boom'); });
        this.on('skill.xp', (e) => { globalThis.__xp2 = e.delta; });
        this.on('inventory.changed', () => { globalThis.__inv = true; });
    }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let bones = [bone(0)];
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let stats1 = [prayer(224)];
    snap.inv = &[];
    snap.stats = &stats1;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__xp2||0"), 112);
    assert_eq!(iso.probe("__inv").unwrap(), serde_json::Value::Bool(true));
    iso.on_game_tick(3);
    let _ = iso.probe("1");
    iso.join();
}

#[test]
fn fires_while_loop_is_parked() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    onStart() {
        this.on('inventory.changed', (e) => {
            if (e.id === -1 && e.previousId !== -1) globalThis.__buried = (globalThis.__buried||0)+1;
        });
    }
    async loop() {
        globalThis.__looping = true;
        await Execution.delayUntil(() => false, 60000);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    assert_eq!(iso.probe("__rs2b0t_host.loopInFlight").unwrap(), true);
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 1);
    assert_eq!(iso.probe("__rs2b0t_host.loopInFlight").unwrap(), true);
    iso.join();
}

#[test]
fn hold_delivers_events_but_drops_callback_actions() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('inventory.changed', (e) => {
            if (e.id === -1) globalThis.__buried = (globalThis.__buried||0)+1;
            const h = globalThis.__rs2b0t_host;
            h.interact = h.interact || [];
            h.interact.push({ op: 'held', name: 'Bones', action: 'Bury' });
        });
    }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.inv = &[];
    snap.hold = true;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 1);
    let interacts = iso.drain_interacts();
    assert!(
        interacts.is_empty(),
        "hold must drop callback actions: {interacts:?}"
    );
    iso.join();
}

#[test]
fn pause_freezes_callbacks_and_resume_delivers_net() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    iso.pause();
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    let _ = iso.probe("1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "paused snapshot is silent"
    );
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0, "paused tick is skipped");
    iso.resume();
    iso.on_game_tick(3);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 1);
    iso.join();
}

#[test]
fn peer_isolates_do_not_share_baselines() {
    let a = spawn_listeners();
    let b = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&a, &snap);
    post_snapshot_input(&b, &snap);
    a.on_game_tick(1);
    b.on_game_tick(1);
    let _ = a.probe("1");
    let _ = b.probe("1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&a, &snap);
    a.on_game_tick(2);
    let _ = a.probe("1");
    let _ = b.probe("1");
    assert_eq!(probe_i64(&a, "__buried||0"), 1);
    assert_eq!(probe_i64(&b, "__buried||0"), 0);
    a.join();
    b.join();
}

#[test]
fn chat_still_fires_with_inv_and_stats() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('chat.message', (e) => { globalThis.__chat = e.text; });
        this.on('inventory.changed', () => { globalThis.__inv = true; });
    }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.inv = &[];
    snap.chat_text = Some("You bury the bones.");
    let chat_lines = [ChatLineInput {
        seq: 1,
        text: "You bury the bones.",
        type_: 0,
        username: None,
    }];
    snap.chat_lines = &chat_lines;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(
        iso.probe("__chat").unwrap().as_str(),
        Some("You bury the bones.")
    );
    assert_eq!(iso.probe("__inv").unwrap(), true);
    iso.join();
}

#[test]
fn explicit_slots_are_not_packed() {
    let iso = spawn_listeners();
    let rows = [bone(3), bone(5)];
    let mut snap = base_snapshot();
    snap.inv = &rows;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 2);
    assert_eq!(probe_i64(&iso, "__lastSlot"), 5);
    iso.join();
}

#[test]
fn rejected_async_callback_does_not_poison_siblings() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('skill.xp', () => Promise.reject('nope'));
        this.on('skill.xp', (e) => { globalThis.__xp = e.xp; globalThis.__hits = (globalThis.__hits || 0) + 1; });
    }
    loop() { globalThis.__alive = (globalThis.__alive || 0) + 1; }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    let stats1 = [prayer(224)];
    snap.stats = &stats1;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let xp = probe_alive(&iso, "__xp");
    assert_eq!(
        xp.as_i64(),
        Some(224),
        "sibling must see the real xp, not a missing-probe fallback: {xp:?}"
    );
    assert_eq!(probe_alive(&iso, "__hits").as_i64(), Some(1));
    iso.on_game_tick(3);
    let alive = probe_alive(&iso, "__alive");
    assert!(
        alive.as_i64().unwrap_or(0) >= 1,
        "runner must keep ticking after rejection: {alive:?}"
    );
    let last_err = probe_alive(&iso, "globalThis.__rs2b0t_host.lastError");
    assert_eq!(
        last_err,
        serde_json::Value::Null,
        "handled rejection must not leave lastError armed: {last_err:?}"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter().any(|l| l.contains("nope")),
        "rejection must surface on the tick log, got {logs:?}"
    );
    let stats2 = [prayer(336)];
    snap.stats = &stats2;
    snap.tick = 4;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(4);
    let xp2 = probe_alive(&iso, "__xp");
    assert_eq!(
        xp2.as_i64(),
        Some(336),
        "later event still delivers: {xp2:?}"
    );
    assert_eq!(probe_alive(&iso, "__hits").as_i64(), Some(2));
    iso.join();
}

#[test]
fn runaway_callback_uses_existing_50ms_interrupt() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('skill.xp', () => { while (true) {} });
    }
    loop() { globalThis.__alive = (globalThis.__alive||0)+1; }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let stats1 = [prayer(224)];
    snap.stats = &stats1;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    std::thread::sleep(std::time::Duration::from_millis(80));
    iso.pause();
    iso.resume();
    iso.on_game_tick(3);
    let n = iso
        .probe("__alive")
        .expect("isolate must stay usable after interrupted callback");
    assert!(n.as_i64().unwrap_or(0) >= 1, "got {n}");
    iso.join();
}

#[test]
fn reset_session_drops_pending_and_reseeds() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.reset_session_work();
    iso.on_game_tick(2);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    let _ = iso.probe("1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "post-reset snapshot seeds"
    );
    let bones2 = [bone(0)];
    snap.inv = &bones2;
    snap.tick = 4;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(4);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0);
    snap.inv = &[];
    snap.tick = 5;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(5);
    let _ = iso.probe("1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 1);
    iso.join();
}

#[test]
fn multi_snapshot_without_tick_delivers_net() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    snap.inv = &bones;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    let _ = probe_alive(&iso, "1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "intermediate bury must not backlog across snapshots"
    );
    iso.join();
}

#[test]
fn logout_before_tick_does_not_replay_bury() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    snap.inv = &[];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    snap.ingame = false;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    snap.ingame = true;
    snap.inv = &bones;
    snap.tick = 4;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(4);
    let _ = probe_alive(&iso, "1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "pre-logout bury must not replay after reseed"
    );
    iso.join();
}

#[test]
fn sparse_offline_omit_ingame_does_not_fire() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    snap.stats = &stats0;
    let (kf, fp) = encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(kf);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    snap.ingame = false;
    snap.tick = 2;
    let (logout, fp2) = encode_snapshot_delta(Some(&fp), &snap, false);
    iso.post_snapshot(logout);
    snap.tick = 3;
    let stats1 = [prayer(224)];
    snap.stats = &stats1;
    snap.inv = &[];
    let (sparse, _) = encode_snapshot_delta(Some(&fp2), &snap, false);
    iso.post_snapshot(sparse);
    iso.on_game_tick(3);
    let _ = probe_alive(&iso, "1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0);
    assert_eq!(probe_i64(&iso, "__xp||0"), 0);
    snap.ingame = true;
    snap.tick = 4;
    let (online, _) = encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(online);
    iso.on_game_tick(4);
    let _ = probe_alive(&iso, "1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0);
    assert_eq!(probe_i64(&iso, "__xp||0"), 0);
    iso.join();
}

#[test]
fn pause_reconnect_does_not_fabricate_history() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    iso.pause();
    snap.ingame = false;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    snap.ingame = true;
    snap.inv = &[];
    let stats1 = [prayer(224)];
    snap.stats = &stats1;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.resume();
    iso.on_game_tick(4);
    let _ = probe_alive(&iso, "1");
    assert_eq!(probe_i64(&iso, "__buried||0"), 0);
    assert_eq!(probe_i64(&iso, "__xp||0"), 0);
    iso.join();
}

#[test]
fn pause_size0_then_ready_does_not_fabricate() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    iso.pause();
    snap.inv_size = 0;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    snap.inv_size = 28;
    snap.inv = &bones;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.resume();
    iso.on_game_tick(4);
    let _ = probe_alive(&iso, "1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "size0→ready during pause must seed"
    );
    iso.join();
}

#[test]
fn pause_invalid_then_valid_does_not_fabricate() {
    let iso = spawn_listeners();
    let bones = [bone(0)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    iso.pause();
    let bad = [ItemRowInput {
        name: Some("Bones"),
        count: 1,
        id: 526,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: -1,
    }];
    snap.inv = &bad;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    snap.inv = &bones;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.resume();
    iso.on_game_tick(4);
    let _ = probe_alive(&iso, "1");
    assert_eq!(
        probe_i64(&iso, "__buried||0"),
        0,
        "invalid→valid during pause must seed"
    );
    iso.join();
}

#[test]
fn hold_async_public_action_is_dropped() {
    let src = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    onStart() {
        this.on('skill.xp', async () => {
            globalThis.__xp = (globalThis.__xp || 0) + 1;
            await Promise.resolve();
            const b = Inventory.first('Bones');
            if (b) b.interact('Bury');
            globalThis.__after = true;
        });
    }
    loop() { globalThis.__loop = (globalThis.__loop || 0) + 1; }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let bones = [bone(0)];
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.inv = &bones;
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = probe_alive(&iso, "1");
    let stats1 = [prayer(224)];
    snap.stats = &stats1;
    snap.hold = true;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    assert_eq!(probe_alive(&iso, "__xp").as_i64(), Some(1));
    assert_eq!(probe_alive(&iso, "__after"), serde_json::Value::Bool(true));
    let held = iso.drain_interacts();
    assert!(
        held.is_empty(),
        "hold must drop async public action: {held:?}"
    );
    snap.hold = false;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    let _ = probe_alive(&iso, "1");
    let after = iso.drain_interacts();
    assert!(
        after.is_empty(),
        "lifted hold must not flush dropped callback action: {after:?}"
    );
    assert!(
        probe_alive(&iso, "__loop").as_i64().unwrap_or(0) >= 1,
        "non-hold tick must still run loop"
    );
    iso.join();
}

#[test]
fn hold_runaway_callback_next_tick_recovers() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('skill.xp', () => { while (true) {} });
    }
    loop() { globalThis.__alive = (globalThis.__alive || 0) + 1; }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let stats0 = [prayer(112)];
    let mut snap = base_snapshot();
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let before = probe_alive(&iso, "__alive")
        .as_i64()
        .expect("tick 1 loop must run");
    let stats1 = [prayer(224)];
    snap.stats = &stats1;
    snap.hold = true;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    std::thread::sleep(std::time::Duration::from_millis(80));
    snap.hold = false;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    let n = probe_alive(&iso, "__alive")
        .as_i64()
        .expect("isolate must stay usable after hold-path interrupt");
    assert!(
        n > before,
        "next eligible tick must recover after hold-path interrupt: before={before} after={n}"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter().any(|l| l.contains("interrupted")),
        "runaway under hold must use the existing interrupt, got {logs:?}"
    );
    iso.join();
}
