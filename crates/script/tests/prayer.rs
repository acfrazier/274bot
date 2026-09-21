use script::isolate_fb::{ReachViewInput, SnapshotInput, StatInput, TileInput, VarpInput};
use script::load::{ApiFamily, JsLibrary, LoadIsolate, LoadShape};
use script::shim::InteractReq;

mod common;
use common::post_snapshot_input;

fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()
}

fn data_289() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()
}

fn both_data() -> [std::sync::Arc<api::game_data::SelectedGameData>; 2] {
    [data(), data_289()]
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3222,
            z: 3222,
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
        widgets: &[],
    }
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn spawn_v1(src: &str, game_data: std::sync::Arc<api::game_data::SelectedGameData>) -> LoadIsolate {
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], game_data)
        .unwrap()
}

fn spawn_v2(src: &str, game_data: std::sync::Arc<api::game_data::SelectedGameData>) -> LoadIsolate {
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::NativeTick, vec![], game_data)
        .unwrap()
}

fn if_button(id: i32) -> InteractReq {
    InteractReq::IfButton { component_id: id }
}

fn prayer_stat(base: i32, effective: i32) -> StatInput<'static> {
    StatInput {
        index: 5,
        name: "prayer",
        xp: 0,
        base,
        effective,
    }
}

const V1_QUERY: &str = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            known: Prayer.known('Protect from Melee'),
            trimmed: Prayer.known('  protect from melee  '),
            unknown: Prayer.known('Nope'),
            avail: Prayer.available('Protect from Melee'),
            active: Prayer.active('Protect from Melee'),
            points: Prayer.points(),
            max: Prayer.max(),
            full: Prayer.full(),
            protect: typeof globalThis.rustyscript === 'object',
        };
    }
}
"#;

#[test]
fn v1_queries_trim_case_and_stats_on_both_caches() {
    for game_data in both_data() {
        let iso = spawn_v1(V1_QUERY, game_data);
        let stats = [prayer_stat(43, 40)];
        let varps = [VarpInput {
            index: 97,
            value: 1,
        }];
        let mut snap = base_snapshot();
        snap.stats = &stats;
        snap.varps = &varps;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 1);
        let probe = iso.probe("__probe").unwrap();
        assert_eq!(probe["known"], true);
        assert_eq!(probe["trimmed"], true);
        assert_eq!(probe["unknown"], false);
        assert_eq!(probe["avail"], true);
        assert_eq!(probe["active"], true);
        assert_eq!(probe["points"], 40);
        assert_eq!(probe["max"], 43);
        assert_eq!(probe["full"], false);
        iso.join();
    }
}

#[test]
fn v1_available_false_when_points_are_zero() {
    let iso = spawn_v1(V1_QUERY, data());
    let stats = [prayer_stat(43, 0)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["avail"], false);
    assert_eq!(probe["points"], 0);
    iso.join();
}

#[test]
fn v1_non_string_name_throws_type_error() {
    let src = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    async loop() {
        try { Prayer.known(undefined); globalThis.__known = 'no-throw'; }
        catch (e) { globalThis.__known = e && e.name ? e.name : String(e); }
        try {
            await Prayer.set(1, true);
            globalThis.__set = 'resolved';
        } catch (e) {
            globalThis.__set = e && e.name ? e.name : String(e);
        }
    }
}
"#;
    let iso = spawn_v1(src, data());
    post_snapshot_input(&iso, &base_snapshot());
    tick(&iso, 1);
    assert_eq!(iso.probe("__known").unwrap(), "TypeError");
    assert_eq!(iso.probe("__set").unwrap(), "TypeError");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v1_set_matching_and_unknown_do_not_click() {
    let src = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Prayer.set('Protect from Melee', true);
        globalThis.__unknown = await Prayer.set('Nope', true);
    }
}
"#;
    let iso = spawn_v1(src, data());
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 1,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert_eq!(iso.probe("__unknown").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v1_set_on_unavailable_does_not_click() {
    let src = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Prayer.set('Protect from Melee', true);
    }
}
"#;
    let iso = spawn_v1(src, data());
    let stats = [prayer_stat(1, 1)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v1_set_off_ignores_available_and_clicks() {
    let src = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Prayer.set('Protect from Melee', false);
    }
}
"#;
    let iso = spawn_v1(src, data());
    let stats = [prayer_stat(1, 0)];
    let varps = [VarpInput {
        index: 97,
        value: 1,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    snap.varps = &[VarpInput {
        index: 97,
        value: 0,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    iso.join();
}

#[test]
fn v1_omitted_on_known_clicks_then_times_out_false() {
    let src = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Prayer.set('Protect from Melee');
    }
}
"#;
    let iso = spawn_v1(src, data());
    let stats = [prayer_stat(43, 43)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    std::thread::sleep(std::time::Duration::from_millis(2_200));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("globalThis.__ok").unwrap(), false);
    iso.join();
}

#[test]
fn v1_set_on_clicks_then_observed_varp() {
    let src = r#"
import { Prayer } from '../../api/prayer/Prayer.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Prayer.set('Protect from Melee', true);
    }
}
"#;
    for game_data in both_data() {
        let iso = spawn_v1(src, game_data);
        let stats = [prayer_stat(43, 43)];
        let varps = [VarpInput {
            index: 97,
            value: 0,
        }];
        let mut snap = base_snapshot();
        snap.stats = &stats;
        snap.varps = &varps;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 1);
        assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
        snap.varps = &[VarpInput {
            index: 97,
            value: 1,
        }];
        snap.tick = 2;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 2);
        assert_eq!(iso.probe("__ok").unwrap(), true);
        iso.join();
    }
}

const V2_QUERY: &str = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = {
    points: api.prayerPoints(),
    max: api.prayerMax(),
    full: api.prayerFull(),
    known: api.prayerKnown({ name: 'Protect from Melee' }),
    trimmed: api.prayerKnown({ name: '  protect from melee  ' }),
    unknown: api.prayerKnown({ name: 'Nope' }),
    avail: api.prayerAvailable({ name: 'Protect from Melee' }),
    active: api.prayerActive({ name: 'Protect from Melee' }),
    prayerNs: api.prayer,
  };
  try { api.request({ op: 'if-button', component_id: 5623 }); globalThis.__if = 'ok'; }
  catch (e) { globalThis.__if = String(e); }
}
"#;

#[test]
fn v2_queries_are_helper_result_and_if_button_stays_private() {
    let iso = spawn_v2(V2_QUERY, data());
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["points"]["ok"], true);
    assert_eq!(probe["points"]["value"], 43);
    assert_eq!(probe["max"]["value"], 43);
    assert_eq!(probe["full"]["value"], true);
    assert_eq!(probe["known"]["ok"], true);
    assert_eq!(probe["known"]["value"], true);
    assert_eq!(probe["trimmed"]["value"], true);
    assert_eq!(probe["unknown"]["ok"], true);
    assert_eq!(probe["unknown"]["value"], false);
    assert_eq!(probe["avail"]["value"], true);
    assert_eq!(probe["active"]["value"], false);
    assert!(probe["prayerNs"].is_null());
    let if_btn = iso.probe("__if").unwrap();
    assert!(
        if_btn
            .as_str()
            .unwrap_or("")
            .contains("not impl: request.if-button"),
        "{if_btn}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v2_invalid_args_do_not_click_or_throw() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__set = await api.prayerSet({ name: 'Protect from Melee' });
  globalThis.__known = api.prayerKnown({ name: 1 });
  globalThis.__on = await api.prayerSet({ name: 'Protect from Melee', on: 1 });
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let set = iso.probe("__set").unwrap();
    let known = iso.probe("__known").unwrap();
    let on = iso.probe("__on").unwrap();
    assert_eq!(set["error"], "invalid-args");
    assert_eq!(known["error"], "invalid-args");
    assert_eq!(on["error"], "invalid-args");
    assert_eq!(set["ok"], false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v2_set_unknown_and_unavailable_are_errors_without_click() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__unknown = await api.prayerSet({ name: 'Nope', on: true });
  globalThis.__unavail = await api.prayerSet({ name: 'Protect from Melee', on: true });
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(1, 1)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let unknown = iso.probe("__unknown").unwrap();
    let unavail = iso.probe("__unavail").unwrap();
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["error"], "unknown-prayer");
    assert_eq!(unavail["error"], "unavailable");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v2_set_clicks_then_observed_varp() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__set = await api.prayerSet({ name: 'Protect from Melee', on: true });
}
"#;
    let iso = spawn_v2(src, data_289());
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    assert_eq!(iso.probe("__rs_v2_tick_pending").unwrap(), true);
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let set = iso.probe("__set").unwrap();
    assert_eq!(set["ok"], true);
    assert_eq!(set["value"], true);
    assert!(set.get("reason").is_none() || set["reason"].is_null());
    iso.join();
}

#[test]
fn v2_set_timeout_is_error() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__set = await api.prayerSet({ name: 'Protect from Melee', on: true });
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    std::thread::sleep(std::time::Duration::from_millis(2_050));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let set = iso.probe("__set").unwrap();
    assert_eq!(set["ok"], false);
    assert_eq!(set["error"], "toggle-timeout");
    iso.join();
}

#[test]
fn v2_clear_continues_after_timeout_with_counts() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__clear = await api.prayerClear();
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let varps = [
        VarpInput {
            index: 96,
            value: 1,
        },
        VarpInput {
            index: 97,
            value: 1,
        },
    ];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5622)]);
    std::thread::sleep(std::time::Duration::from_millis(2_050));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    snap.varps = &[
        VarpInput {
            index: 96,
            value: 1,
        },
        VarpInput {
            index: 97,
            value: 0,
        },
    ];
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    let clear = iso.probe("__clear").unwrap();
    assert_eq!(clear["ok"], true);
    assert_eq!(clear["value"]["clicked"], 2);
    assert_eq!(clear["value"]["timed_out"], 1);
    iso.join();
}

#[test]
fn v2_reset_does_not_cross_settle_a_stale_promise() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  const set = await api.prayerSet({ name: 'Protect from Melee', on: true });
  if (!globalThis.__first) globalThis.__first = set;
  globalThis.__set = set;
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    iso.reset_session_work();
    let _ = iso.probe("true");
    let set = iso.probe("globalThis.__set").unwrap();
    assert_eq!(set["ok"], false);
    assert_eq!(set["error"], "aborted");
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let first = iso.probe("globalThis.__first").unwrap();
    assert_eq!(first["ok"], false);
    assert_eq!(
        first["error"], "aborted",
        "stale promise must not take a later matching snapshot: {first}"
    );
    iso.join();
}

#[test]
fn published_example_queries_sets_clears_and_stops() {
    let dir = std::env::temp_dir().join(format!("274bot-prayer-v2-example-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut library = JsLibrary::with_cache(dir.join("cards.json"), dir.join("cache"));
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("prayer_v2.ts");
    let card = library.load(&path).unwrap();
    assert_eq!(card.shape, LoadShape::NativeTick);
    assert_eq!(card.api_family, ApiFamily::V2);
    assert!(card.unloadable.is_none(), "{:?}", card.unloadable);
    let iso = LoadIsolate::spawn_with_game_data(card.js, card.shape, vec![], data()).unwrap();
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    snap.varps = &[VarpInput {
        index: 97,
        value: 0,
    }];
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    let receipt = iso.script_stop_receipt().expect("named helper stop");
    assert_eq!(receipt.reason, "prayer v2 qualification complete");
    assert!(
        iso.stopped(),
        "example must stop after sequential set+clear"
    );
    iso.join();
}

#[test]
fn published_v1_adapter_imports_prayer_and_stops() {
    let dir = std::env::temp_dir().join(format!("274bot-prayer-v1-example-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut library = JsLibrary::with_cache(dir.join("cards.json"), dir.join("cache"));
    let path = script::load::live_example_path("prayer_v1.ts").expect("File allowlist");
    let card = library.load(&path).unwrap();
    assert_eq!(card.shape, LoadShape::CompatClass);
    assert!(card.unloadable.is_none(), "{:?}", card.unloadable);
    let iso = LoadIsolate::spawn_with_game_data(card.js, card.shape, vec![], data()).unwrap();
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    snap.varps = &[VarpInput {
        index: 97,
        value: 0,
    }];
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    let receipt = iso.script_stop_receipt().expect("named helper stop");
    assert_eq!(receipt.reason, "prayer v1 qualification complete");
    assert!(iso.stopped());
    iso.join();
}

#[test]
fn v2_overlap_set_and_clear_busy_settles_without_mutating_first() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  if (globalThis.__started) return;
  globalThis.__started = true;
  const setP = api.prayerSet({ name: 'Protect from Melee', on: true });
  const clearP = api.prayerClear();
  setP.then((r) => { globalThis.__set = r; });
  clearP.then((r) => { globalThis.__clear = r; });
  await Promise.all([setP, clearP]);
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 95,
        value: 1,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(5623)],
        "refused clear must not click the already-active Magic row"
    );
    let clear = iso.probe("globalThis.__clear").unwrap();
    assert_eq!(clear["ok"], false);
    assert_eq!(clear["error"], "busy");
    assert!(
        iso.probe("globalThis.__set").unwrap().is_null(),
        "original set must still own the admitted pump"
    );
    snap.varps = &[
        VarpInput {
            index: 95,
            value: 1,
        },
        VarpInput {
            index: 97,
            value: 1,
        },
    ];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let set = iso.probe("globalThis.__set").unwrap();
    assert_eq!(set["ok"], true);
    assert_eq!(set["value"], true);
    assert_eq!(iso.probe("globalThis.__clear").unwrap()["error"], "busy");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v2_second_set_is_busy_and_original_completes() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  if (globalThis.__started) return;
  globalThis.__started = true;
  const first = api.prayerSet({ name: 'Protect from Melee', on: true });
  const second = api.prayerSet({ name: 'Protect from Magic', on: true });
  first.then((r) => { globalThis.__first = r; });
  second.then((r) => { globalThis.__second = r; });
  await Promise.all([first, second]);
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    let second = iso.probe("globalThis.__second").unwrap();
    assert_eq!(second["ok"], false);
    assert_eq!(second["error"], "busy");
    assert!(iso.probe("globalThis.__first").unwrap().is_null());
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let first = iso.probe("globalThis.__first").unwrap();
    assert_eq!(first["ok"], true);
    assert_eq!(first["value"], true);
    assert_eq!(iso.probe("globalThis.__second").unwrap()["error"], "busy");
    assert!(
        iso.drain_interacts().is_empty(),
        "refused second set must not click Magic"
    );
    iso.join();
}

#[test]
fn v2_sync_tick_fire_and_forget_set_progresses_without_new_admission() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  if (globalThis.__started) return;
  globalThis.__started = true;
  api.prayerSet({ name: 'Protect from Melee', on: true }).then((r) => {
    globalThis.__set = r;
  });
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let varps = [VarpInput {
        index: 97,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    assert_eq!(iso.probe("__rs_v2_tick_pending").unwrap(), false);
    assert!(iso.probe("globalThis.__set").unwrap().is_null());
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let set = iso.probe("globalThis.__set").unwrap();
    assert_eq!(set["ok"], true);
    assert_eq!(set["value"], true);
    assert_eq!(
        iso.probe("typeof globalThis.__rs_prayer_pump").unwrap(),
        "undefined"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "matching snapshot must settle the admitted set, not admit another click"
    );
    iso.join();
}

#[test]
fn v2_pause_and_hold_do_not_progress_admitted_set() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  if (globalThis.__done) return;
  globalThis.__set = await api.prayerSet({ name: 'Protect from Melee', on: true });
  globalThis.__done = true;
}
"#;
    let iso = spawn_v2(src, data());
    let stats = [prayer_stat(43, 43)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(5623)]);
    assert!(iso.probe("globalThis.__set").unwrap().is_null());

    iso.pause();
    std::thread::sleep(std::time::Duration::from_millis(2_050));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "paused ticks must not click or settle"
    );
    assert!(iso.probe("globalThis.__set").unwrap().is_null());
    iso.resume();

    snap.hold = true;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    std::thread::sleep(std::time::Duration::from_millis(2_050));
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "held ticks must not click or settle"
    );
    assert!(iso.probe("globalThis.__set").unwrap().is_null());

    snap.hold = false;
    snap.varps = &[VarpInput {
        index: 97,
        value: 1,
    }];
    snap.tick = 4;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    let set = iso.probe("globalThis.__set").unwrap();
    assert_eq!(
        set["ok"], true,
        "deadline must stay frozen while blocked: {set}"
    );
    assert_eq!(set["value"], true);
    iso.join();
}

fn reserved_only_varps() -> [VarpInput; 3] {
    [
        VarpInput {
            index: 108,
            value: 0,
        },
        VarpInput {
            index: 300,
            value: 0,
        },
        VarpInput {
            index: 301,
            value: 0,
        },
    ]
}

/// A posted varps vector that omits the selected prayer band must not keep
/// a prior ON, and must not look like a proven 0 (no click-to-clear, no
/// Set(off) settle).
#[test]
fn truncated_posted_varps_do_not_retain_on_or_fabricate_off() {
    let query = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__active = api.prayerActive({ name: 'Protect from Melee' }).value;
  globalThis.__n = (globalThis.__n || 0) + 1;
  if (globalThis.__n === 2) {
    globalThis.__clear = await api.prayerClear();
  }
}
"#;
    let iso = spawn_v2(query, data_289());
    let stats = [prayer_stat(43, 43)];
    let on = [VarpInput {
        index: 97,
        value: 1,
    }];
    let truncated = reserved_only_varps();
    let mut snap = base_snapshot();
    snap.stats = &stats;
    snap.varps = &on;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__active").unwrap(), true);
    assert!(iso.drain_interacts().is_empty());

    snap.varps = &truncated;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__active").unwrap(),
        false,
        "omitted 97 is unobserved, not a retained ON"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "clear must not click a truncated ON"
    );
    let clear = iso.probe("__clear").unwrap();
    assert_eq!(clear["ok"], true);
    assert_eq!(clear["value"]["clicked"], 0);
    assert_eq!(clear["value"]["timed_out"], 0);
    iso.join();

    let off_src = r#"
export const apiVersion = 2;
export async function tick(api) {
  if (!globalThis.__started) {
    globalThis.__off = await api.prayerSet({ name: 'Protect from Melee', on: false });
    globalThis.__started = true;
  }
}
"#;
    let off = spawn_v2(off_src, data_289());
    let mut off_snap = base_snapshot();
    off_snap.stats = &stats;
    off_snap.varps = &on;
    post_snapshot_input(&off, &off_snap);
    tick(&off, 1);
    assert_eq!(off.drain_interacts(), vec![if_button(5623)]);
    assert!(off.probe("globalThis.__off").unwrap().is_null());

    off_snap.varps = &truncated;
    off_snap.tick = 2;
    post_snapshot_input(&off, &off_snap);
    tick(&off, 2);
    assert!(
        off.probe("globalThis.__off").unwrap().is_null(),
        "truncated missing 97 is not a proven off"
    );
    assert!(off.drain_interacts().is_empty());
    off.join();
}
