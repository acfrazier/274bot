//! Native progress watchdog: isolate lifecycle facts, single-flight loop,
//! recoveryAnchor bridge, and slot identity/restart.

use std::time::{Duration, Instant};

use script::isolate_fb::{ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape, SlotScript, WatchdogAction, WatchdogState};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2655,
            z: 3298,
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

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn tick(iso: &LoadIsolate, n: u64, hold: bool) {
    let mut snap = base_snapshot();
    snap.tick = n;
    snap.hold = hold;
    post_snapshot_input(iso, &snap);
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

#[test]
fn note_progress_queues_lifecycle_op_and_is_noop_without_host() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    loop() { Execution.noteProgress(); }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, false);
    assert!(
        iso.drain_interacts().is_empty(),
        "note-progress is not a game op"
    );
    let life = iso.drain_lifecycle();
    assert!(
        life.iter().any(|r| matches!(r, InteractReq::NoteProgress)),
        "note-progress must ride the FlatBuffer batch: {life:?}"
    );
    iso.join();
}

#[test]
fn empty_loop_emits_loop_settled() {
    let iso = spawn(
        r#"
export default class T extends LoopingBot {
    loop() { globalThis.__n = (globalThis.__n || 0) + 1; }
}
"#,
    );
    tick(&iso, 1, false);
    tick(&iso, 2, false);
    assert_eq!(iso.probe("__n").unwrap(), 2);
    let life = iso.drain_lifecycle();
    assert!(
        life.iter()
            .filter(|r| matches!(r, InteractReq::LoopSettled))
            .count()
            >= 1,
        "loop() return is scheduler progress: {life:?}"
    );
    iso.join();
}

#[test]
fn never_resolving_loop_is_single_flight_and_still_paints_and_fires_listeners() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        globalThis.__paints = 0;
        globalThis.__loops = 0;
        BotHost.addTickListener(() => { globalThis.__fires += 1; });
    }
    onPaint() { globalThis.__paints = (globalThis.__paints || 0) + 1; }
    loop() {
        globalThis.__loops += 1;
        return new Promise(() => {});
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, false);
    tick(&iso, 2, false);
    tick(&iso, 3, false);
    let loops = iso.probe("__loops").unwrap().as_i64().unwrap();
    let fires = iso.probe("__fires").unwrap().as_i64().unwrap();
    let paints = iso.probe("__paints").unwrap().as_i64().unwrap();
    assert_eq!(loops, 1, "never-resolving loop must not re-enter");
    assert!(fires >= 2, "tick listeners continue: fires={fires}");
    assert!(paints >= 2, "onPaint continues: paints={paints}");
    assert!(
        !iso.drain_lifecycle()
            .iter()
            .any(|r| matches!(r, InteractReq::LoopSettled)),
        "unsettled loop must not emit loop-settled"
    );
    iso.join();
}

#[test]
fn wait_enqueue_and_settle_are_separate_facts() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__loops = (globalThis.__loops || 0) + 1;
        await Execution.delayUntil(() => Game.tick() >= 3, 6000);
        globalThis.__done = true;
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, false);
    assert_eq!(iso.probe("__loops").unwrap(), 1);
    let first = iso.drain_lifecycle();
    assert!(
        first.iter().any(|r| matches!(r, InteractReq::WaitEnqueued)),
        "park enqueue: {first:?}"
    );
    tick(&iso, 2, false);
    assert_eq!(iso.probe("__loops").unwrap(), 1, "still parked");
    tick(&iso, 3, false);
    assert_eq!(iso.probe("__done").unwrap(), true);
    let settled = iso.drain_lifecycle();
    assert!(
        settled
            .iter()
            .any(|r| matches!(r, InteractReq::WaitSettled)),
        "wait settle: {settled:?}"
    );
    iso.join();
}

#[test]
fn recovery_anchor_reply_is_generation_tagged_and_validated() {
    let src = r#"
export default class T extends LoopingBot {
    recoveryAnchor() { return { x: 3200, z: 3200, level: 0 }; }
    loop() {}
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, false);
    iso.request_recovery_anchor();
    let _ = iso.probe("true");
    let life = iso.drain_lifecycle();
    assert!(
        life.iter().any(|r| matches!(
            r,
            InteractReq::RecoveryAnchor {
                x: 3200,
                z: 3200,
                level: 0
            }
        )),
        "validated tile: {life:?}"
    );
    iso.join();
}

#[test]
fn invalid_recovery_anchor_is_none() {
    let src = r#"
export default class T extends LoopingBot {
    recoveryAnchor() { return [1, 2, 3]; }
    loop() {}
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, false);
    iso.request_recovery_anchor();
    let _ = iso.probe("true");
    let life = iso.drain_lifecycle();
    assert!(
        life.iter()
            .any(|r| matches!(r, InteractReq::RecoveryAnchorNone)),
        "array is invalid: {life:?}"
    );
    iso.join();
}

#[test]
fn session_reset_drops_stale_recovery_reply() {
    let src = r#"
export default class T extends LoopingBot {
    recoveryAnchor() { return { x: 1, z: 2, level: 0 }; }
    loop() {}
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, false);
    iso.request_recovery_anchor();
    iso.reset_session_work();
    let _ = iso.probe("true");
    assert!(
        iso.drain_lifecycle().is_empty(),
        "stale generation-tagged reply must drop"
    );
    iso.join();
}

#[test]
fn hold_skips_recovery_anchor_request() {
    let src = r#"
export default class T extends LoopingBot {
    recoveryAnchor() { globalThis.__sampled = true; return { x: 1, z: 2, level: 0 }; }
    loop() {}
}
"#;
    let iso = spawn(src);
    tick(&iso, 1, true);
    iso.request_recovery_anchor();
    let _ = iso.probe("true");
    assert_eq!(iso.probe("globalThis.__sampled || false").unwrap(), false);
    assert!(iso.drain_lifecycle().is_empty());
    iso.join();
}

#[test]
fn slot_stop_cancels_restart_and_clears_identity() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export default class T extends LoopingBot { loop() {} }".into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    assert!(slot.load_identity().is_some());
    let t = Instant::now();
    slot.feed_watchdog(t, Some((1, 1, 0)), &[], false, true, &[]);
    let later = t + script::watchdog::HARD_STALL;
    let action = slot.feed_watchdog(later, Some((1, 1, 0)), &[], false, true, &[]);
    assert!(matches!(action, WatchdogAction::Restart { .. }));
    slot.stop();
    assert!(slot.load_identity().is_none());
    assert_eq!(slot.watchdog().state(), WatchdogState::Idle);
    assert_eq!(
        slot.feed_watchdog(
            later + script::watchdog::WEDGE,
            Some((1, 1, 0)),
            &[],
            false,
            true,
            &[]
        ),
        WatchdogAction::None
    );
}

#[test]
fn slot_restart_preserves_source_settings_and_cooldown() {
    let mut slot = SlotScript::new();
    let src =
        "export default class T extends LoopingBot { loop() { this.n = (this.n || 0) + 1; } }";
    let mut bag = serde_json::Map::new();
    bag.insert("spell".into(), serde_json::json!("High"));
    slot.start_load_with_settings(
        src.into(),
        LoadShape::CompatClass,
        Some(&bag),
        vec![("sib.js".into(), "export const x = 1;".into())],
    )
    .unwrap();
    let before = slot.load_identity().unwrap().clone();
    assert_eq!(
        before
            .settings_bag
            .as_ref()
            .unwrap()
            .get("spell")
            .and_then(|v| v.as_str()),
        Some("High")
    );
    let t = Instant::now();
    slot.feed_watchdog(t, Some((0, 0, 0)), &[], false, true, &[]);
    let action = slot.feed_watchdog(
        t + script::watchdog::WEDGE,
        Some((0, 0, 0)),
        &[],
        false,
        true,
        &[],
    );
    assert_eq!(action, WatchdogAction::RequestAnchor);
    slot.feed_watchdog(
        t + script::watchdog::WEDGE,
        Some((0, 0, 0)),
        &[],
        false,
        true,
        &[InteractReq::RecoveryAnchorNone],
    );
    slot.restart_load_from_identity(t + script::watchdog::WEDGE)
        .unwrap();
    let after = slot.load_identity().unwrap();
    assert_eq!(&*after.source, &*before.source);
    assert_eq!(after.shape, before.shape);
    assert_eq!(&*after.siblings, &*before.siblings);
    assert_eq!(after.settings_bag, before.settings_bag);
    assert!(slot.watchdog().last_recovery().is_some());
    let later = slot.feed_watchdog(
        t + script::watchdog::WEDGE + script::watchdog::WEDGE,
        Some((0, 0, 0)),
        &[],
        false,
        true,
        &[],
    );
    assert!(
        !matches!(
            later,
            WatchdogAction::RequestAnchor | WatchdogAction::Restart { .. }
        ),
        "cooldown survives recreate: {later:?}"
    );
    slot.stop();
}

#[test]
fn slot_start_while_running_still_errors() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export default class T extends LoopingBot { loop() {} }".into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let err = slot
        .start_load(
            "export default class U extends LoopingBot { loop() {} }".into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap_err();
    assert!(err.contains("already active"));
    slot.stop();
}

#[test]
fn explicit_note_progress_is_real_host_effect() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export default class T extends LoopingBot { loop() {} }".into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let t = Instant::now();
    slot.feed_watchdog(t, Some((1, 1, 0)), &[], false, true, &[]);
    slot.feed_watchdog(
        t + Duration::from_secs(60),
        Some((1, 1, 0)),
        &[],
        false,
        true,
        &[InteractReq::NoteProgress],
    );
    assert!(!matches!(
        slot.feed_watchdog(
            t + script::watchdog::WEDGE,
            Some((1, 1, 0)),
            &[],
            false,
            true,
            &[]
        ),
        WatchdogAction::RequestAnchor | WatchdogAction::Restart { .. }
    ));
    slot.stop();
}
