//! Native BotHost.addTickListener on observed ticks. No packet attach.
//!
//! Fire after posted snapshot+tick and before onStart/loop, or before parked
//! settle. Pause/hold/generation skip do not fire.

use script::isolate_fb::{ReachViewInput, SnapshotInput, TileInput};
use script::{LoadIsolate, LoadShape};

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

fn post_tick(iso: &LoadIsolate, tick: u64, hold: bool) {
    let mut snap = base_snapshot();
    snap.tick = tick;
    snap.hold = hold;
    post_snapshot_input(iso, &snap);
    iso.on_game_tick(tick);
}

fn fires(iso: &LoadIsolate) -> i64 {
    iso.probe("globalThis.__fires || 0")
        .unwrap()
        .as_i64()
        .unwrap()
}

fn tick_error_logs(iso: &LoadIsolate) -> Vec<String> {
    iso.drain_logs()
        .into_iter()
        .filter(|l| l.starts_with("tick ") || l.contains("script requested stop"))
        .collect()
}

const COUNTING: &str = r#"
import { BotHost } from '../../runtime/BotHost.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        globalThis.__seen = [];
        this.stopFightObserver = BotHost.addTickListener(() => {
            globalThis.__fires += 1;
            globalThis.__seen.push({
                tickCount: BotHost.tickCount,
                gameTick: Game.tick(),
            });
        });
    }
    loop() {}
}
"#;

#[test]
fn add_tick_listener_on_start_does_not_fail() {
    let iso = spawn(COUNTING);
    post_tick(&iso, 1, false);
    let _ = iso.probe("true");
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .all(|l| !l.contains("not impl") && !l.starts_with("tick ")),
        "addTickListener must not fail the start tick: {logs:?}"
    );
    assert_eq!(
        fires(&iso),
        0,
        "start tick fires before onStart; Set is empty"
    );
    iso.join();
}

#[test]
fn start_tick_does_not_fire_later_ticks_see_posted_snapshot() {
    let iso = spawn(COUNTING);
    post_tick(&iso, 7, false);
    assert_eq!(fires(&iso), 0, "zero fires during the start tick");
    post_tick(&iso, 8, false);
    post_tick(&iso, 9, false);
    assert_eq!(fires(&iso), 2, "one fire per later posted tick");
    let seen = iso.probe("__seen").unwrap();
    assert_eq!(seen[0]["tickCount"], 8);
    assert_eq!(seen[0]["gameTick"], 8);
    assert_eq!(seen[1]["tickCount"], 9);
    assert_eq!(seen[1]["gameTick"], 9);
    iso.join();
}

#[test]
fn parked_delay_until_still_fires_listeners() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        BotHost.addTickListener(() => { globalThis.__fires += 1; });
    }
    async loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
        if (globalThis.__rs_loops === 1) {
            await Execution.delayUntil(() => Game.tick() >= 4, 6000);
        }
    }
}
"#;
    let iso = spawn(src);
    post_tick(&iso, 1, false);
    assert_eq!(iso.probe("__rs_loops").unwrap(), 1, "first loop parks");
    assert_eq!(fires(&iso), 0, "start tick does not fire");
    post_tick(&iso, 2, false);
    post_tick(&iso, 3, false);
    assert_eq!(iso.probe("__rs_loops").unwrap(), 1, "still parked");
    assert_eq!(fires(&iso), 2, "parked pump fires listeners");
    iso.join();
}

#[test]
fn pause_freezes_listeners_and_resume_continues() {
    let iso = spawn(COUNTING);
    post_tick(&iso, 1, false);
    assert_eq!(fires(&iso), 0);
    iso.pause();
    post_tick(&iso, 2, false);
    post_tick(&iso, 3, false);
    assert_eq!(fires(&iso), 0, "paused ticks do not fire listeners");
    iso.resume();
    post_tick(&iso, 4, false);
    assert_eq!(fires(&iso), 1, "resume continues listener delivery");
    iso.join();
}

#[test]
fn hold_freezes_listeners_and_still_paints() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        BotHost.addTickListener(() => { globalThis.__fires += 1; });
    }
    loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
    }
    onPaint() {
        globalThis.__rs_paints = (globalThis.__rs_paints || 0) + 1;
        const p = Paint.begin();
        p.title('held');
        p.row('status');
        p.end();
    }
}
"#;
    let iso = spawn(src);
    post_tick(&iso, 1, false);
    assert_eq!(fires(&iso), 0);
    let paints_after_first: i64 = iso.probe("__rs_paints").unwrap().as_i64().unwrap_or(0);
    assert!(paints_after_first >= 1, "first tick paints");
    post_tick(&iso, 2, true);
    post_tick(&iso, 3, true);
    assert_eq!(fires(&iso), 0, "hold does not fire listeners");
    assert_eq!(
        iso.probe("globalThis.__rs_loops || 0").unwrap(),
        1,
        "hold freezes loop"
    );
    let paints: i64 = iso.probe("__rs_paints").unwrap().as_i64().unwrap();
    assert!(
        paints >= 3,
        "onPaint still runs while held (paints={paints})"
    );
    let frame = iso.paint().expect("paint forwarded while held");
    assert_eq!(frame.title.as_deref(), Some("held"));
    iso.join();
}

#[test]
fn unsubscribe_stops_fires() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        globalThis.__unsub = BotHost.addTickListener(() => { globalThis.__fires += 1; });
    }
    loop() {}
}
"#;
    let iso = spawn(src);
    post_tick(&iso, 1, false);
    post_tick(&iso, 2, false);
    assert_eq!(fires(&iso), 1);
    iso.probe("globalThis.__unsub()").unwrap();
    post_tick(&iso, 3, false);
    post_tick(&iso, 4, false);
    assert_eq!(fires(&iso), 1, "unsubscribe stops further fires");
    iso.join();
}

#[test]
fn same_callback_added_twice_fires_once() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        const cb = () => { globalThis.__fires += 1; };
        BotHost.addTickListener(cb);
        BotHost.addTickListener(cb);
    }
    loop() {}
}
"#;
    let iso = spawn(src);
    post_tick(&iso, 1, false);
    post_tick(&iso, 2, false);
    assert_eq!(fires(&iso), 1, "Set dedup: same function fires once");
    iso.join();
}

#[test]
fn throwing_listener_is_isolated_from_others_and_last_error() {
    let src = r#"
import { BotHost } from '../../runtime/BotHost.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__fires = 0;
        BotHost.addTickListener(() => { throw new Error('listener boom'); });
        BotHost.addTickListener(() => { globalThis.__fires += 1; });
    }
    loop() {}
}
"#;
    let iso = spawn(src);
    post_tick(&iso, 1, false);
    post_tick(&iso, 2, false);
    assert_eq!(fires(&iso), 1, "later listener still runs");
    let tick_errs = tick_error_logs(&iso);
    assert!(
        tick_errs.is_empty(),
        "listener throw must not become tick last_error: {tick_errs:?}"
    );
    let last = iso
        .probe("globalThis.__rs2b0t_host.lastError || null")
        .unwrap();
    assert!(last.is_null(), "must not set host.lastError, got {last:?}");
    iso.join();
}

#[test]
fn isolates_do_not_share_tick_listeners() {
    let iso_a = spawn(COUNTING);
    let iso_b = spawn(COUNTING);
    post_tick(&iso_a, 1, false);
    post_tick(&iso_b, 1, false);
    post_tick(&iso_a, 2, false);
    assert_eq!(fires(&iso_a), 1);
    assert_eq!(fires(&iso_b), 0, "module Set is per isolate");
    post_tick(&iso_b, 2, false);
    assert_eq!(fires(&iso_b), 1);
    iso_a.join();
    iso_b.join();
}

#[test]
fn reset_session_work_skips_until_next_snapshot_tick() {
    let iso = spawn(COUNTING);
    post_tick(&iso, 1, false);
    assert_eq!(fires(&iso), 0);
    iso.reset_session_work();
    assert_eq!(
        fires(&iso),
        0,
        "reset does not fire; in-flight generation is skipped"
    );
    post_tick(&iso, 2, false);
    assert_eq!(
        fires(&iso),
        1,
        "Set survives; next Snapshot+Tick fires once"
    );
    iso.join();
}
