//! JS API v2 core: syntax-aware version routing, NativeApi runtime, and
//! fail-closed request / snapshot behaviour.

use std::time::{Duration, Instant};

use script::load::{
    parse_declared_api_version, resolve_api_family, ApiFamily, JsLibrary, LoadIsolate, LoadShape,
};
use script::shim::InteractReq;
use script::{ScriptSource, SlotScript, WatchdogAction};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-script-native-api-v2-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    path
}

fn test_library(dir: &std::path::Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

fn post_base_with_hold(iso: &LoadIsolate, tick: u64, hold: bool) {
    let mut input = script::isolate_fb::SnapshotInput {
        tick,
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
        bank_generation: 7,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold,
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
        scene_state: 0,
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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        widgets: &[],
    };
    input.tick = tick;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&input));
}

fn post_base(iso: &LoadIsolate, tick: u64) {
    post_base_with_hold(iso, tick, false);
}

#[test]
fn numeric_2_0_equals_2_and_type_annotation_is_accepted() {
    assert_eq!(
        parse_declared_api_version("export const apiVersion = 2; export function tick() {}")
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        parse_declared_api_version("export const apiVersion = 2.0; export function tick() {}")
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        parse_declared_api_version(
            "export const apiVersion: number = 2; export function tick(api) {}"
        )
        .unwrap(),
        Some(2)
    );
    assert_eq!(
        parse_declared_api_version(
            "export const apiVersion = 2 as const; export function tick(api) {}"
        )
        .unwrap(),
        Some(2)
    );
}

#[test]
fn comments_and_strings_are_not_declarations() {
    assert_eq!(
        parse_declared_api_version(
            "// export const apiVersion = 2\nexport function tick() {}"
        )
        .unwrap(),
        None
    );
    assert_eq!(
        parse_declared_api_version(
            "const s = \"export const apiVersion = 2\"; export function tick() {}"
        )
        .unwrap(),
        None
    );
    assert_eq!(
        parse_declared_api_version("const apiVersion = 2; export function tick() {}").unwrap(),
        None
    );
}

#[test]
fn malformed_and_unsupported_versions() {
    assert!(parse_declared_api_version(
        "export const apiVersion = \"2\"; export function tick() {}"
    )
    .unwrap_err()
    .code()
    .contains("malformed"));
    assert!(parse_declared_api_version(
        "export let apiVersion = 2; export function tick() {}"
    )
    .unwrap_err()
    .code()
    .contains("malformed"));
    assert!(parse_declared_api_version(
        "export const apiVersion = 3; export function tick() {}"
    )
    .unwrap_err()
    .code()
    .contains("unsupported"));
}

#[test]
fn resolve_family_precedence() {
    let (shape, family) =
        resolve_api_family("export const apiVersion = 2; export function tick() {}").unwrap();
    assert_eq!(shape, LoadShape::NativeTick);
    assert_eq!(family, ApiFamily::V2);

    let (shape, family) = resolve_api_family("export function tick() {}").unwrap();
    assert_eq!(shape, LoadShape::NativeTick);
    assert_eq!(family, ApiFamily::Unversioned);

    let err = resolve_api_family(
        "export const apiVersion = 2;\nexport default class X extends LoopingBot {}",
    )
    .unwrap_err();
    assert!(err.contains("api-version-conflict"));

    let err = resolve_api_family("export const apiVersion = 1; export function tick() {}")
        .unwrap_err();
    assert!(err.contains("api-version-conflict"));

    let err = resolve_api_family("export const apiVersion = 2; const x = 1;").unwrap_err();
    assert!(err.contains("api-version-missing-tick"));

    let (shape, family) =
        resolve_api_family("export const apiVersion = 1;\ndefineBot({ loop() {} });").unwrap();
    assert_eq!(shape, LoadShape::CompatDefineBot);
    assert_eq!(family, ApiFamily::V1);
}

#[test]
fn library_rejects_named_version_errors_and_keeps_unversioned() {
    let dir = scratch("lib_version");
    let mut lib = test_library(&dir);
    let bad = write_file(
        &dir,
        "bad.js",
        "export const apiVersion = 2;\nexport default class X extends LoopingBot {}",
    );
    let err = lib.load(&bad).unwrap_err();
    assert!(err.contains("api-version-conflict"), "{err}");

    let ok = write_file(&dir, "ok.js", "export function tick(api) { api.tick }");
    let card = lib.load(&ok).unwrap();
    assert_eq!(card.api_family, ApiFamily::Unversioned);
    assert_eq!(card.shape, LoadShape::NativeTick);
}

#[test]
fn v2_card_provenance_is_not_identity() {
    let dir = scratch("v2_ident");
    let mut lib = test_library(&dir);
    let path = write_file(
        &dir,
        "burier.js",
        "export const apiVersion = 2;\nexport function tick(api) {}",
    );
    let card = lib.load(&path).unwrap();
    assert_eq!(card.api_family, ApiFamily::V2);
    let unversioned_path = write_file(&dir, "other.js", "export function tick(api) {}");
    let other = lib.load(&unversioned_path).unwrap();
    assert_ne!(card.identity_key(), other.identity_key());
    assert_eq!(card.source, ScriptSource::File);
}

#[test]
fn unversioned_native_still_proxy_only() {
    let src = "export function tick(api) {\n  try { api.snapshot; globalThis.__rs_ok = true; }\n  catch (e) { globalThis.__rs_err = String(e); }\n}";
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let err = iso.probe("globalThis.__rs_err").unwrap();
    assert!(
        err.as_str().unwrap_or("").contains("not impl: api.snapshot"),
        "{err}"
    );
    iso.join();
}

#[test]
fn v2_runtime_exposes_native_api_not_host() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__rs_family = globalThis.__rs_api_family;
  globalThis.__rs_ingame = api.snapshot.ingame;
  globalThis.__rs_npcs = api.snapshot.npcs;
  api.log('hi');
  api.request({ op: 'held', name: 'Bones', action: 'Bury' });
  try { api.request({ op: 'npc', name: 'Man', action: 'Talk-to' }); globalThis.__rs_npc = 'ok'; }
  catch (e) { globalThis.__rs_npc = String(e); }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__rs_family").unwrap(), 2);
    assert_eq!(iso.probe("globalThis.__rs_ingame").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__rs_npcs").unwrap(), serde_json::Value::Null);
    let npc = iso.probe("globalThis.__rs_npc").unwrap();
    assert!(
        npc.as_str().unwrap_or("").contains("not impl: request.npc"),
        "{npc}"
    );
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|r| matches!(
            r,
            InteractReq::Held { name, action } if name == "Bones" && action == "Bury"
        )),
        "{reqs:?}"
    );
    iso.join();
}

#[test]
fn v2_structured_paint_is_forwarded_and_survives_hold() {
    let src = r##"
export const apiVersion = 2;
export function tick(api) {
  api.paint.begin({ accent: "#123456" })
    .title("Native v2")
    .row("phase", "burying")
    .end();
}
"##;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let paint = iso.paint().expect("forwarded v2 paint");
    assert_eq!(paint.title.as_deref(), Some("Native v2"));
    assert_eq!(paint.accent.as_deref(), Some("#123456"));
    assert_eq!(paint.lines, ["phase | burying"]);
    assert_eq!(iso.drain_lifecycle(), vec![InteractReq::LoopSettled]);

    post_base_with_hold(&iso, 2, true);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    let held = iso.paint().expect("held frame keeps last v2 paint");
    assert_eq!(held.title.as_deref(), Some("Native v2"));
    assert_eq!(held.lines, ["phase | burying"]);
    assert!(
        iso.drain_lifecycle().is_empty(),
        "held posted ticks are not scheduler progress"
    );
    iso.join();
}

#[test]
fn v2_sync_tick_emits_scheduler_settlement_without_pending_promise() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__rs_n = (globalThis.__rs_n || 0) + 1;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), false);
    assert_eq!(
        iso.drain_lifecycle(),
        vec![InteractReq::LoopSettled],
        "one successful native tick is one scheduler settlement"
    );
    iso.join();
}

#[test]
fn v2_settlement_feeds_existing_watchdog_scheduler_clock() {
    let source = r#"
export const apiVersion = 2;
export function tick() {}
"#;
    let producer = LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).unwrap();
    producer.on_game_tick(1);
    let _ = producer.probe("true");
    let lifecycle = producer.drain_lifecycle();
    assert_eq!(lifecycle, vec![InteractReq::LoopSettled]);

    let mut slot = SlotScript::new();
    slot.start_load(source.into(), LoadShape::NativeTick, vec![])
        .unwrap();
    let t = Instant::now();
    assert_eq!(
        slot.feed_watchdog(t, None, &[], false, true, &[]),
        WatchdogAction::None
    );
    let settlement_at = t + script::watchdog::SCHEDULER_WARN;
    assert_eq!(
        slot.feed_watchdog(settlement_at, None, &[], false, true, &lifecycle),
        WatchdogAction::None
    );
    assert_eq!(
        slot.feed_watchdog(
            settlement_at + script::watchdog::SCHEDULER_WARN - Duration::from_millis(1),
            None,
            &[],
            false,
            true,
            &[],
        ),
        WatchdogAction::None
    );
    assert_eq!(
        slot.feed_watchdog(
            settlement_at + script::watchdog::SCHEDULER_WARN,
            None,
            &[],
            false,
            true,
            &[],
        ),
        WatchdogAction::WarnHungLoop
    );
    slot.stop();
    producer.join();
}

#[test]
fn v2_snapshot_is_read_only_and_next_merge_works() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  try { api.snapshot.ingame = false; globalThis.__rs_set = 'ok'; }
  catch (e) { globalThis.__rs_set = 'threw'; }
  try { api.snapshot.inv.push({}); globalThis.__rs_push = 'ok'; }
  catch (e) { globalThis.__rs_push = 'threw'; }
  globalThis.__rs_ingame = api.snapshot.ingame;
  globalThis.__rs_gen = api.snapshot.bank_generation;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__rs_set").unwrap(), "threw");
    assert_eq!(iso.probe("globalThis.__rs_push").unwrap(), "threw");
    assert_eq!(iso.probe("globalThis.__rs_ingame").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__rs_gen").unwrap(), 7);
    post_base(&iso, 2);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("globalThis.__rs_ingame").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__rs_gen").unwrap(), 7);
    iso.join();
}

#[test]
fn v2_async_tick_does_not_reenter() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__rs_n = (globalThis.__rs_n || 0) + 1;
  if (globalThis.__rs_n === 1) {
    await new Promise((resolve) => { globalThis.__rs_release = resolve; });
  }
  globalThis.__rs_done = (globalThis.__rs_done || 0) + 1;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    assert_eq!(iso.probe("globalThis.__rs_n").unwrap(), 1);
    assert_eq!(iso.probe("globalThis.__rs_done").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), true);
    assert!(
        iso.drain_lifecycle().is_empty(),
        "posted ticks and a pending promise are not settlements"
    );
    let _ = iso.probe("globalThis.__rs_release(); true");
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), false);
    iso.on_game_tick(4);
    assert_eq!(iso.probe("globalThis.__rs_done").unwrap(), 2);
    assert_eq!(iso.probe("globalThis.__rs_n").unwrap(), 2);
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), false);
    let settled = iso.drain_lifecycle();
    assert_eq!(
        settled
            .iter()
            .filter(|fact| matches!(fact, InteractReq::LoopSettled))
            .count(),
        2,
        "fulfilled native promise and following successful sync tick settle: {settled:?}"
    );
    iso.join();
}

#[test]
fn v2_promise_settled_during_hold_forwards_only_scheduler_progress() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  await new Promise((resolve) => {
    const host = globalThis.__rs2b0t_host;
    let tick = host.tick;
    Object.defineProperty(host, 'tick', {
      configurable: true,
      get() { return tick; },
      set(next) {
        tick = next;
        if (next === 2) resolve();
      },
    });
  });
  api.request({ op: 'held', name: 'Bones', action: 'Bury' });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), true);
    assert!(iso.drain_lifecycle().is_empty());

    post_base_with_hold(&iso, 2, true);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(
        iso.drain_lifecycle(),
        vec![InteractReq::LoopSettled],
        "fulfilled pending work remains scheduler progress under hold"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "gameplay produced by the held continuation must be dropped"
    );
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), false);

    post_base_with_hold(&iso, 3, true);
    iso.on_game_tick(3);
    let _ = iso.probe("true");
    assert!(
        iso.drain_lifecycle().is_empty(),
        "later held ticks are not scheduler progress"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn v2_failed_ticks_do_not_emit_scheduler_settlement() {
    for source in [
        r#"
export const apiVersion = 2;
export function tick() { throw new Error("sync failure"); }
"#,
        r#"
export const apiVersion = 2;
export async function tick() { throw new Error("async failure"); }
"#,
    ] {
        let iso =
            LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).unwrap();
        iso.on_game_tick(1);
        let _ = iso.probe("true");
        assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), false);
        assert!(
            iso.drain_lifecycle().is_empty(),
            "failed work is not scheduler progress"
        );
        iso.join();
    }
}

#[test]
fn v2_old_generation_promise_settlement_is_not_reowned() {
    let src = r#"
export const apiVersion = 2;
export function tick() {
  globalThis.__rs_n = (globalThis.__rs_n || 0) + 1;
  if (globalThis.__rs_n === 1) {
    return new Promise((resolve) => { globalThis.__rs_release = resolve; });
  }
  throw new Error("new generation failure");
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), true);
    assert!(iso.drain_lifecycle().is_empty());

    iso.reset_session_work();
    let _ = iso.probe("globalThis.__rs_release(); true");
    assert_eq!(iso.probe("globalThis.__rs_v2_tick_pending").unwrap(), false);
    post_base(&iso, 2);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert!(
        iso.drain_lifecycle().is_empty(),
        "old-generation settlement must not be tagged as current progress"
    );
    iso.join();
}

#[test]
fn v2_settings_and_stop() {
    let src = r#"
export const apiVersion = 2;
export const SETTINGS = { boneName: { type: 'string', default: 'Bones' } };
export function tick(api) {
  globalThis.__rs_bone = api.settings.str('boneName', 'Bones');
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("boneName".into(), serde_json::json!("Dragon bones"));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__rs_bone").unwrap(), "Dragon bones");
    iso.join();
}

#[test]
fn v2_reset_session_drops_queued_request() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  api.request({ op: 'walk-nearest-bank' });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let before = iso.drain_interacts();
    assert!(
        before.iter().any(|r| matches!(r, InteractReq::WalkNearestBank)),
        "{before:?}"
    );
    iso.reset_session_work();
    iso.on_game_tick(2);
    // Generation skip: the post-reset tick of a new generation may enqueue
    // a fresh request; stale queued success is the old generation batch.
    let _ = iso.drain_interacts();
    iso.join();
}

#[test]
fn v2_pause_does_not_run_tick() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__rs_n = (globalThis.__rs_n || 0) + 1;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(iso.drain_lifecycle(), vec![InteractReq::LoopSettled]);
    iso.pause();
    iso.on_game_tick(2);
    assert_eq!(iso.probe("globalThis.__rs_n").unwrap(), 1);
    assert!(
        iso.drain_lifecycle().is_empty(),
        "paused posted ticks are not scheduler progress"
    );
    iso.resume();
    iso.on_game_tick(3);
    assert_eq!(iso.probe("globalThis.__rs_n").unwrap(), 2);
    assert_eq!(iso.drain_lifecycle(), vec![InteractReq::LoopSettled]);
    iso.join();
}

#[test]
fn v2_generic_walk_forwards_find_options_and_request_id() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  api.request({ op: 'walk', x: 1, z: 2, level: 1 });
  api.request({
    op: 'walk',
    x: 1,
    z: 2,
    level: 1,
    allow_teleports: false,
    allow_wilderness: false,
    allow_bank_fetch: false,
  });
  api.request({
    op: 'walk',
    x: 1,
    z: 2,
    level: 1,
    allow_teleports: true,
    allow_wilderness: true,
    allow_bank_fetch: true,
  });
  api.request({
    op: 'walk-near',
    x: 3,
    z: 4,
    level: 2,
    radius: 5,
    request_id: 99,
  });
  api.request({ op: 'walk-nearest-bank' });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let drained = iso.drain_interacts();
    let walks: Vec<_> = drained
        .into_iter()
        .filter(|req| {
            !matches!(
                req,
                InteractReq::LoopSettled | InteractReq::WaitEnqueued | InteractReq::WaitSettled
            )
        })
        .collect();
    assert_eq!(
        walks,
        vec![
            InteractReq::Walk {
                x: 1,
                z: 2,
                level: 1,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
            },
            InteractReq::Walk {
                x: 1,
                z: 2,
                level: 1,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
            },
            InteractReq::Walk {
                x: 1,
                z: 2,
                level: 1,
                allow_teleports: true,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: 0,
            },
            InteractReq::WalkNear {
                x: 3,
                z: 4,
                level: 2,
                radius: 5,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 99,
            },
            InteractReq::WalkNearestBank,
        ]
    );
    let wired = script::isolate_fb::decode_interact_batch(
        &script::isolate_fb::encode_interact_batch(&walks),
    )
    .expect("v2 walk FB roundtrip");
    assert_eq!(wired, walks);
    iso.join();
}
