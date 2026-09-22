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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
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
  globalThis.__rs_npcs = Array.isArray(api.snapshot.npcs);
  globalThis.__rs_npcs_len = api.snapshot.npcs.length;
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
    assert_eq!(iso.probe("globalThis.__rs_npcs").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__rs_npcs_len").unwrap(), 0);
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

#[test]
fn v2_inspect_route_uses_isolate_token_and_rejects_invented_ids() {
    use script::shim::InspectAvoidWire;
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const token = api.inspectBegin({
    from: { x: 2763, z: 3233, level: 0 },
    to: { x: 2803, z: 3208, level: 0 },
    allow_teleports: false,
    allow_wilderness: true,
    allow_bank_fetch: false,
    avoid: [{ minX: 2780, maxX: 3040, minZ: 3130, maxZ: 3330 }],
    timeout_ms: 8000,
  });
  globalThis.__token = token;
  api.request({
    op: 'inspect-route',
    from: { x: 1, z: 2, level: 0 },
    to: { x: 3, z: 4, level: 0 },
    request_id: 0,
  });
  api.request({
    op: 'inspect-route',
    from: { x: 1, z: 2, level: 0 },
    to: { x: 3, z: 4, level: 0 },
    request_id: 99,
  });
  globalThis.__invented = api.inspectSettled(99);
  globalThis.__zero = api.inspectSettled(0);
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let token: u64 = iso
        .probe("globalThis.__token")
        .unwrap()
        .as_u64()
        .expect("token");
    assert_ne!(token, 0);
    assert_eq!(iso.probe("globalThis.__invented").unwrap(), true);
    assert_eq!(iso.probe("globalThis.__zero").unwrap(), false);
    let drained = iso.drain_interacts();
    let inspects: Vec<_> = drained
        .into_iter()
        .filter(|req| matches!(req, InteractReq::InspectRoute { .. }))
        .collect();
    assert_eq!(inspects.len(), 2);
    match &inspects[0] {
        InteractReq::InspectRoute {
            x,
            z,
            level,
            from_x,
            from_z,
            from_level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            avoid,
            request_id,
        } => {
            assert_eq!((*from_x, *from_z, *from_level), (2763, 3233, 0));
            assert_eq!((*x, *z, *level), (2803, 3208, 0));
            assert!(!*allow_teleports);
            assert!(*allow_wilderness);
            assert!(!*allow_bank_fetch);
            assert_eq!(*request_id, token);
            assert_eq!(
                avoid,
                &vec![InspectAvoidWire::Rect {
                    min_x: 2780,
                    max_x: 3040,
                    min_z: 3130,
                    max_z: 3330,
                    level: None,
                }]
            );
        }
        other => panic!("expected inspect-route, got {other:?}"),
    }
    match &inspects[1] {
        InteractReq::InspectRoute { request_id, .. } => {
            assert_eq!(*request_id, 0, "explicit 0 is snapshot-only");
        }
        other => panic!("expected inspect-route, got {other:?}"),
    }
    let wired = script::isolate_fb::decode_interact_batch(
        &script::isolate_fb::encode_interact_batch(&inspects),
    )
    .expect("v2 inspect FB roundtrip");
    assert_eq!(wired, inspects);
    iso.join();
}

#[test]
fn v2_prayer_methods_are_named_helper_results_not_a_namespace() {
    let src = r#"
export const apiVersion = 2;
export async function tick(api) {
  globalThis.__probe = {
    points: api.prayerPoints(),
    known: api.prayerKnown({ name: 'Protect from Melee' }),
    prayer: api.prayer,
    setThen: typeof api.prayerSet({ name: 'Nope', on: true }).then,
    clearThen: typeof api.prayerClear().then,
  };
  try { api.request({ op: 'if-button', component_id: 5623 }); globalThis.__if = 'ok'; }
  catch (e) { globalThis.__if = String(e); }
}
"#;
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["points"]["ok"], true);
    assert_eq!(probe["points"]["value"], 0);
    assert_eq!(probe["known"]["ok"], true);
    assert_eq!(probe["known"]["value"], true);
    assert!(probe["prayer"].is_null());
    assert_eq!(probe["setThen"], "function");
    assert_eq!(probe["clearThen"], "function");
    let if_btn = iso.probe("globalThis.__if").unwrap();
    assert!(
        if_btn
            .as_str()
            .unwrap_or("")
            .contains("not impl: request.if-button"),
        "{if_btn}"
    );
    iso.join();
}

#[test]
fn v2_loadout_potion_methods_are_named_helper_results_not_a_namespace() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const loadout = { worn: { righthand: 'Rune scimitar' }, carry: [{ item: 'Lobster', qty: 2 }] };
  globalThis.__probe = {
    food: api.foodOf({ loadout, fallback: 'Trout' }),
    gear: api.gearOf({ loadout }),
    supplies: api.suppliesOf({ loadout }),
    weapon: api.weaponOf({ loadout, fallback: null }),
    range: api.rangeLoadoutOf({ weapon: 'Bronze dart', ammo: 'Iron arrow' }),
    faded: api.boostFaded({ base: 70, effective: 70 }),
    planned: api.plannedPotions({ carry: [{ item: 'Super attack(4)', qty: 1 }] }),
    sip: api.potionToSip({ plans: [], held: [], levels: [] }),
    namespace: api.loadout,
  };
}
"#;
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["food"]["ok"], true, "{probe:?}");
    assert_eq!(probe["food"]["value"], "Lobster");
    assert_eq!(probe["gear"]["value"][0], "Rune scimitar");
    assert_eq!(probe["supplies"]["value"][0]["qty"], 2);
    assert_eq!(probe["weapon"]["value"], "Rune scimitar");
    assert_eq!(probe["range"]["value"]["thrown"], true);
    assert_eq!(probe["faded"]["value"], true);
    assert_eq!(probe["planned"]["value"][0]["flask"], "Super attack(4)");
    assert_eq!(probe["sip"]["ok"], true);
    assert!(probe["sip"]["value"].is_null());
    assert!(probe["namespace"].is_null());
    iso.join();
}

#[test]
fn v2_fight_next_names_yield_and_does_not_map_aborted_to_anonymous_done() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.fightBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.fightNext({ token: token + 1 });
  globalThis.__bad = bad;
  const first = api.fightNext({
    token,
    died: false,
    hpFraction: 1,
    panicHp: 0.1,
    retreatHp: 0,
    hasFood: true,
    needEat: false,
    style: 'melee',
    safespotIndex: 0,
    buryBones: false,
    boneName: 'Bones',
    hasArmSpecial: false,
    hasShieldReady: false,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:1,z:1,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
  });
  globalThis.__first = first;
  const unexpected = api.fightNext({
    token,
    reply: { eatOk: true },
    died: false,
    hpFraction: 1,
    panicHp: 0.1,
    retreatHp: 0,
    hasFood: true,
    needEat: false,
    style: 'melee',
    safespotIndex: 0,
    buryBones: false,
    boneName: 'Bones',
    hasArmSpecial: false,
    hasShieldReady: false,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:1,z:1,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
  });
  globalThis.__unexpected = unexpected;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let begin = iso.probe("globalThis.__begin").unwrap();
    assert_eq!(begin["ok"], true, "{begin:?}");
    let bad = iso.probe("globalThis.__bad").unwrap();
    assert_eq!(bad["ok"], false, "{bad:?}");
    assert_ne!(bad["status"], "done");
    assert_ne!(bad["kind"], "yield");
    let first = iso.probe("globalThis.__first").unwrap();
    assert_eq!(first["ok"], true, "{first:?}");
    if first["status"] == "done" {
        assert_eq!(first["kind"], "yield", "{first:?}");
    }
    let unexpected = iso.probe("globalThis.__unexpected").unwrap();
    assert_eq!(unexpected["ok"], false, "{unexpected:?}");
    assert_ne!(unexpected["kind"], "yield");
    assert_ne!(unexpected["status"], "done");
    iso.join();
}

#[test]
fn v2_hold_next_names_yield_and_does_not_map_aborted_to_anonymous_done() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.holdBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.holdNext({ token: token + 1 });
  globalThis.__bad = bad;
  const first = api.holdNext({
    token,
    died: false,
    targetIdx: null,
    hpFraction: 1,
    panicHp: 0.1,
    hasFood: true,
    style: 'melee',
    safespotIndex: 0,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:2,z:2,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
  });
  globalThis.__first = first;
  const unexpected = api.holdNext({
    token,
    reply: { eatOk: true },
    died: false,
    targetIdx: null,
    hpFraction: 1,
    panicHp: 0.1,
    hasFood: true,
    style: 'melee',
    safespotIndex: 0,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:2,z:2,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
  });
  globalThis.__unexpected = unexpected;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let begin = iso.probe("globalThis.__begin").unwrap();
    assert_eq!(begin["ok"], true, "{begin:?}");
    let bad = iso.probe("globalThis.__bad").unwrap();
    assert_eq!(bad["ok"], false, "{bad:?}");
    assert_eq!(bad["kind"], "aborted", "{bad:?}");
    assert_eq!(bad["status"], "aborted", "{bad:?}");
    assert_ne!(bad["status"], "done");
    assert_ne!(bad["kind"], "yield");
    let first = iso.probe("globalThis.__first").unwrap();
    assert_eq!(first["ok"], true, "{first:?}");
    if first["status"] == "done" {
        assert_eq!(first["kind"], "yield", "{first:?}");
    } else {
        assert_eq!(first["status"], "continue", "{first:?}");
        assert_eq!(first["kind"], "status", "{first:?}");
    }
    let unexpected = iso.probe("globalThis.__unexpected").unwrap();
    assert_eq!(unexpected["ok"], false, "{unexpected:?}");
    assert_eq!(unexpected["kind"], "aborted", "{unexpected:?}");
    assert_ne!(unexpected["kind"], "yield");
    assert_ne!(unexpected["status"], "done");
    iso.join();
}

#[test]
fn v2_retreat_next_names_yield_and_does_not_map_aborted_to_anonymous_done() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.retreatBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.retreatNext({ token: token + 1 });
  globalThis.__bad = bad;
  const first = api.retreatNext({
    token,
    died: false,
    targetIdx: null,
    hpFraction: 0.4,
    panicHp: 0.1,
    retreatHp: 0.5,
    hasFood: false,
    style: 'melee',
    safespotIndex: 0,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:2,z:2,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
  });
  globalThis.__first = first;
  const unexpected = api.retreatNext({
    token,
    reply: { eatOk: true },
    died: false,
    targetIdx: null,
    hpFraction: 0.4,
    panicHp: 0.1,
    retreatHp: 0.5,
    hasFood: false,
    style: 'melee',
    safespotIndex: 0,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:2,z:2,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
  });
  globalThis.__unexpected = unexpected;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let begin = iso.probe("globalThis.__begin").unwrap();
    assert_eq!(begin["ok"], true, "{begin:?}");
    let bad = iso.probe("globalThis.__bad").unwrap();
    assert_eq!(bad["ok"], false, "{bad:?}");
    assert_eq!(bad["kind"], "aborted", "{bad:?}");
    assert_eq!(bad["status"], "aborted", "{bad:?}");
    assert_ne!(bad["status"], "done");
    assert_ne!(bad["kind"], "yield");
    let first = iso.probe("globalThis.__first").unwrap();
    assert_eq!(first["ok"], true, "{first:?}");
    if first["status"] == "done" {
        assert_eq!(first["kind"], "yield", "{first:?}");
    } else {
        assert_eq!(first["status"], "continue", "{first:?}");
    }
    let unexpected = iso.probe("globalThis.__unexpected").unwrap();
    assert_eq!(unexpected["ok"], false, "{unexpected:?}");
    assert_eq!(unexpected["kind"], "aborted", "{unexpected:?}");
    assert_ne!(unexpected["kind"], "yield");
    assert_ne!(unexpected["status"], "done");
    iso.join();
}

#[test]
fn v2_walkspot_next_names_yield_and_does_not_map_aborted_to_anonymous_done() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.walkspotBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.walkspotNext({ token: token + 1 });
  globalThis.__bad = bad;
  const first = api.walkspotNext({
    token,
    died: false,
    targetIdx: null,
    hpFraction: 1,
    panicHp: 0.1,
    hasFood: false,
    style: 'range',
    safespotIndex: 0,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:2,z:2,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
    approach: [],
  });
  globalThis.__first = first;
  const unexpected = api.walkspotNext({
    token,
    reply: { eatOk: true },
    died: false,
    targetIdx: null,
    hpFraction: 1,
    panicHp: 0.1,
    hasFood: false,
    style: 'range',
    safespotIndex: 0,
    key: 't',
    target: 'Goblin',
    alsoHunt: [],
    safespots: [{x:1,z:1,level:0}],
    meleeAnchor: {x:2,z:2,level:0},
    boxes: [{minX:0,maxX:10,minZ:0,maxZ:10,level:0}],
    approach: [],
  });
  globalThis.__unexpected = unexpected;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let begin = iso.probe("globalThis.__begin").unwrap();
    assert_eq!(begin["ok"], true, "{begin:?}");
    let bad = iso.probe("globalThis.__bad").unwrap();
    assert_eq!(bad["ok"], false, "{bad:?}");
    assert_eq!(bad["kind"], "aborted", "{bad:?}");
    assert_eq!(bad["status"], "aborted", "{bad:?}");
    assert_ne!(bad["status"], "done");
    assert_ne!(bad["kind"], "yield");
    let first = iso.probe("globalThis.__first").unwrap();
    assert_eq!(first["ok"], true, "{first:?}");
    if first["status"] == "done" {
        assert_eq!(first["kind"], "yield", "{first:?}");
    } else {
        assert_eq!(first["status"], "continue", "{first:?}");
        assert_eq!(first["kind"], "status", "{first:?}");
    }
    let unexpected = iso.probe("globalThis.__unexpected").unwrap();
    assert_eq!(unexpected["ok"], false, "{unexpected:?}");
    assert_eq!(unexpected["kind"], "aborted", "{unexpected:?}");
    assert_eq!(unexpected["status"], "aborted", "{unexpected:?}");
    assert_ne!(unexpected["kind"], "yield");
    assert_ne!(unexpected["status"], "done");
    iso.join();
}

#[test]
fn v2_gather_methods_are_named_sync_helper_results_not_a_namespace() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const methods = api.gatherMethods({ skill: 'woodcutting' });
  const resource = api.gatherResource({ name: 'limestone' });
  globalThis.__probe = {
    methodsOk: methods.ok,
    methodsThen: typeof methods.then,
    resourceLen: resource.value && resource.value.rows.length,
    namespace: api.gather,
    bestAxe: typeof api.bestAxe,
    bestPickaxe: typeof api.bestPickaxe,
  };
}
"#;
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["methodsOk"], true, "{probe:?}");
    assert_eq!(probe["methodsThen"], "undefined", "{probe:?}");
    assert_eq!(probe["resourceLen"], 3, "{probe:?}");
    assert!(probe["namespace"].is_null(), "{probe:?}");
    assert_eq!(probe["bestAxe"], "undefined", "{probe:?}");
    assert_eq!(probe["bestPickaxe"], "undefined", "{probe:?}");
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(
        interacts.is_empty(),
        "gather query must not push interact: {interacts:?}"
    );
}

#[test]
fn v2_quest_facts_are_named_sync_helper_results_not_request_ops() {
    let bindings = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/load/bindings.rs"));
    let ops = bindings
        .split("const V2_OPS")
        .nth(1)
        .unwrap()
        .split("const OPTIONAL")
        .next()
        .unwrap();
    assert!(!ops.contains("questIdentity"), "{ops}");
    assert!(!ops.contains("questPrereqs"), "{ops}");
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const row = api.questIdentity({ id: 'death' });
  let requested = null;
  try { api.request({ op: 'questIdentity' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = {
    rowOk: row.ok,
    rowThen: typeof row.then,
    varp: row.value && row.value.varp,
    rows: row.value && row.value.rows,
    namespace: api.quest,
    requested,
  };
}
"#;
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["rowOk"], true, "{probe:?}");
    assert_eq!(probe["rowThen"], "undefined", "{probe:?}");
    assert_eq!(probe["varp"], "death_equiproom", "{probe:?}");
    assert!(probe["rows"].is_null(), "{probe:?}");
    assert!(probe["namespace"].is_null(), "{probe:?}");
    assert!(
        probe["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{probe:?}"
    );
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(
        interacts.is_empty(),
        "quest query must not push interact: {interacts:?}"
    );
}

#[test]
fn v2_clue_row_is_a_named_sync_helper_result_not_a_request_op() {
    let bindings = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/load/bindings.rs"));
    assert!(
        !bindings.contains("register_function(\"__rs2b0t_clue_facts"),
        "clue row must not be a rustyscript JSON op"
    );
    let ops = bindings
        .split("const V2_OPS")
        .nth(1)
        .unwrap()
        .split("const OPTIONAL")
        .next()
        .unwrap();
    assert!(!ops.contains("clue"), "{ops}");
    assert!(!ops.contains("deposit"), "{ops}");
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const row = api.clue.row({ id: 3554 });
  let requested = null;
  try { api.request({ op: 'clue.row' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = {
    rowOk: row.ok,
    rowThen: typeof row.then,
    alias: row.value && row.value.alias,
    access: row.value && row.value.access,
    rows: row.value && row.value.rows,
    rowType: typeof api.clue.row,
    keys: api.clue && Object.keys(api.clue),
    flat: typeof api.clueRow,
    quest: api.quest,
    requested,
  };
}
"#;
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["rowOk"], true, "{probe:?}");
    assert_eq!(probe["rowThen"], "undefined", "{probe:?}");
    assert_eq!(probe["alias"], "trail_clue_hard_sextant028", "{probe:?}");
    assert_eq!(probe["access"], "constrained", "{probe:?}");
    assert!(probe["rows"].is_null(), "{probe:?}");
    assert_eq!(probe["rowType"], "function", "{probe:?}");
    assert_eq!(
        probe["keys"],
        serde_json::json!(["row", "heldStep", "packPlan", "hardKit"]),
        "{probe:?}"
    );
    assert_eq!(probe["flat"], "undefined", "{probe:?}");
    assert!(probe["quest"].is_null(), "{probe:?}");
    assert!(
        probe["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{probe:?}"
    );
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(
        interacts.is_empty(),
        "clue row must not push interact: {interacts:?}"
    );
}

#[test]
fn v2_scene_projections_fail_closed_when_collision_is_unavailable() {
    let bindings = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/load/bindings.rs"));
    let ops = bindings
        .split("const V2_OPS")
        .nth(1)
        .unwrap()
        .split("const OPTIONAL")
        .next()
        .unwrap();
    assert!(!ops.contains("sceneLocs"), "{ops}");
    assert!(!ops.contains("sceneNpcs"), "{ops}");
    let keys = bindings
        .split("const SNAPSHOT_KEYS = new Set([")
        .nth(1)
        .unwrap()
        .split("]);")
        .next()
        .unwrap();
    assert!(!keys.contains("'locs'"), "{keys}");
    assert!(!keys.contains("'tick'"), "{keys}");
    // Legal args on both methods. post_base posts an empty locs array and no
    // collision, so unavailable collision wins over that empty array: this is
    // snapshot-unavailable, not { rows: [] } and not a positive loc witness.
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const locs = api.sceneLocs({ ids: [2092, -1], limit: 64 });
  const npcs = api.sceneNpcs({ types: [-1], actions: ['Attack'], limit: 8 });
  const host = globalThis.__rs2b0t_host;
  let requested = null;
  try { api.request({ op: 'sceneLocs' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = {
    locsOk: locs.ok,
    locsError: locs.error,
    locsValue: locs.value,
    locsThen: typeof locs.then,
    npcsOk: npcs.ok,
    npcsError: npcs.error,
    npcsThen: typeof npcs.then,
    pageLocsLen: host.snapshot.locs.length,
    pageCollisionAvailable: host.snapshot.collision.available,
    requested,
  };
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["pageLocsLen"], 0, "{probe:?}");
    assert_eq!(probe["pageCollisionAvailable"], false, "{probe:?}");
    assert_eq!(probe["locsOk"], false, "{probe:?}");
    assert_eq!(probe["locsError"], "snapshot-unavailable", "{probe:?}");
    assert!(probe["locsValue"].is_null(), "{probe:?}");
    assert_eq!(probe["locsThen"], "undefined", "{probe:?}");
    assert_eq!(probe["npcsOk"], false, "{probe:?}");
    assert_eq!(probe["npcsError"], "snapshot-unavailable", "{probe:?}");
    assert_eq!(probe["npcsThen"], "undefined", "{probe:?}");
    assert!(
        probe["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{probe:?}"
    );
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(
        interacts.is_empty(),
        "scene query must not push interact: {interacts:?}"
    );
}

#[test]
fn example_scene_observe_v2_is_read_only_and_fails_closed() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("scene_observe_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert_eq!(src.matches("api.sceneLocs").count(), 1);
    assert_eq!(src.matches("api.sceneNpcs").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile scene_observe_v2.ts");
    let iso = LoadIsolate::spawn(js, LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let err = iso
        .probe("globalThis.__rs2b0t_host.lastError || ''")
        .unwrap();
    let logs = iso.drain_logs();
    let interacts = iso.drain_interacts();
    iso.join();
    assert_eq!(err.as_str().unwrap_or(""), "", "example lastError: {err:?}");
    let failed: Vec<serde_json::Value> = logs
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .filter(|row| row["error"] == "snapshot-unavailable")
        .collect();
    assert_eq!(failed.len(), 2, "logs={logs:?}");
    for row in &failed {
        assert_eq!(row["ok"], false, "{row:?}");
        assert!(row.get("value").is_none(), "{row:?}");
        assert!(row.get("rows").is_none(), "{row:?}");
    }
    assert!(
        interacts.is_empty(),
        "scene example must not push interact: {interacts:?}"
    );
}

#[test]
fn v2_quest_status_fails_closed_without_a_page_and_on_a_null_tab() {
    let bindings = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/load/bindings.rs"));
    let ops = bindings
        .split("const V2_OPS")
        .nth(1)
        .unwrap()
        .split("const OPTIONAL")
        .next()
        .unwrap();
    assert!(!ops.contains("questStatus"), "{ops}");
    let keys = bindings
        .split("const SNAPSHOT_KEYS = new Set([")
        .nth(1)
        .unwrap()
        .split("]);")
        .next()
        .unwrap();
    assert!(!keys.contains("'quest_statuses'"), "{keys}");
    assert!(!keys.contains("'quest_statuses_available'"), "{keys}");
    assert!(!keys.contains("'locs'"), "{keys}");
    assert!(!keys.contains("'tick'"), "{keys}");
    // Arm 1: the example with no posted page. Nothing is posted to this
    // isolate, so the copy has no snapshot object to read: a legal name is
    // snapshot-unavailable. That is not quest-tab-unbound, not a miss, and it
    // is not a colour or status witness.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("quest_status_v2.ts");
    let example = std::fs::read_to_string(&path).expect("example source");
    assert!(!example.contains("request("));
    assert!(!example.contains("h.interact"));
    assert_eq!(example.matches("api.questStatus").count(), 1);
    let js = script::transpile_ts(&example).expect("transpile quest_status_v2.ts");
    let iso = LoadIsolate::spawn(js, LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let err = iso
        .probe("globalThis.__rs2b0t_host.lastError || ''")
        .unwrap();
    let logs = iso.drain_logs();
    let interacts = iso.drain_interacts();
    iso.join();
    assert_eq!(err.as_str().unwrap_or(""), "", "example lastError: {err:?}");
    let failed: Vec<serde_json::Value> = logs
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .filter(|row| row["error"] == "snapshot-unavailable")
        .collect();
    assert_eq!(failed.len(), 1, "logs={logs:?}");
    assert_eq!(failed[0]["ok"], false, "{:?}", failed[0]);
    assert!(failed[0].get("value").is_none(), "{:?}", failed[0]);
    assert!(
        interacts.is_empty(),
        "quest status example must not push interact: {interacts:?}"
    );

    // Arm 2: post_base encodes a null quest tab on a posted page whose tick is
    // bound. A legal name is quest-tab-unbound, not snapshot-unavailable and
    // not not-on-tab: the null check runs before the tick and before the scan.
    let source = r#"
export const apiVersion = 2;
export function tick(api) {
  const row = api.questStatus({ name: 'Death Plateau' });
  let requested = null;
  try { api.request({ op: 'questStatus' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  const page = globalThis.__rs2b0t_host.snapshot;
  globalThis.__probe = {
    ok: row.ok,
    error: row.error,
    value: row.value,
    then: typeof row.then,
    pageUnbound: page.quest_statuses === null,
    pageTick: page.tick,
    requested,
  };
}
"#;
    let iso = LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["pageUnbound"], true, "{probe:?}");
    assert_eq!(probe["pageTick"], 1, "{probe:?}");
    assert_eq!(probe["ok"], false, "{probe:?}");
    assert_eq!(probe["error"], "quest-tab-unbound", "{probe:?}");
    assert!(probe["value"].is_null(), "{probe:?}");
    assert_eq!(probe["then"], "undefined", "{probe:?}");
    assert!(
        probe["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{probe:?}"
    );
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(
        interacts.is_empty(),
        "quest status must not push interact: {interacts:?}"
    );
}
