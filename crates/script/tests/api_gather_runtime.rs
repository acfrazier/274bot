//! Consumer-visible runtime probes for the JS API v2 Gatherer session.
//! These drive the real V8 isolate and its machine host while supplying the
//! same typed FlatBuffer snapshot pages a Load slot publishes.
use std::sync::Arc;

use api::selected::RunKey;
use script::api_gather::{GatherCounts, GatherEnd, GatherFailure, GatherPage, GatherPhase};
use script::isolate_fb::{encode_snapshot_with_native, NativeFactsInput, SnapshotInput};
use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
use script::shim::InteractReq;
use script::{CompiledId, LoadIsolate, LoadShape};

const BASE_SCRIPT: &str = r#"
export const apiVersion = 2;
export function tick(api) { globalThis.__api = api; }
"#;

fn post_snapshot(
    iso: &LoadIsolate,
    tick: u64,
    api_gather: Option<&GatherPage>,
    api_gather_outcome: Option<&GatherEnd>,
) {
    let input = SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 7,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
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
    let native = NativeFactsInput {
        api_gather,
        api_gather_outcome,
        ..NativeFactsInput::default()
    };
    iso.post_snapshot(encode_snapshot_with_native(&input, native));
}

fn tick(
    iso: &LoadIsolate,
    tick: u64,
    api_gather: Option<&GatherPage>,
    api_gather_outcome: Option<&GatherEnd>,
) {
    post_snapshot(iso, tick, api_gather, api_gather_outcome);
    iso.on_game_tick(tick);
    let _ = iso.probe("true").unwrap();
}

fn isolate() -> LoadIsolate {
    let iso = LoadIsolate::spawn(BASE_SCRIPT.into(), LoadShape::NativeTick, vec![]).unwrap();
    tick(&iso, 1, None, None);
    iso
}

fn begin(iso: &LoadIsolate, label: &str, settings: &serde_json::Value) {
    let settings = serde_json::to_string(settings).unwrap();
    let expression = format!(
        r#"(() => {{
          globalThis.__{label}Result = null;
          globalThis.__{label}Settles = 0;
          const promise = __api.gather.run({settings});
          globalThis.__{label}Promise = promise;
          promise.then((result) => {{
            globalThis.__{label}Result = result;
            globalThis.__{label}Settles += 1;
          }});
          return true;
        }})()"#
    );
    assert_eq!(iso.probe(&expression).unwrap(), true);
}

fn begin_and_take_run(
    iso: &LoadIsolate,
    label: &str,
    settings: &serde_json::Value,
) -> (u64, serde_json::Map<String, serde_json::Value>) {
    begin(iso, label, settings);
    let requests = iso.drain_interacts();
    take_single_run(&requests)
}

fn take_single_run(
    requests: &[InteractReq],
) -> (u64, serde_json::Map<String, serde_json::Value>) {
    let runs: Vec<_> = requests
        .iter()
        .filter_map(|request| match request {
            InteractReq::GatherRun {
                request_id,
                settings,
            } => Some((*request_id, settings.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 1, "expected one gather-run in {requests:?}");
    runs.into_iter().next().unwrap()
}


fn counts() -> GatherCounts {
    GatherCounts {
        yielded: 17,
        dropped: 16,
        deposited: 8,
        trips: 2,
        xp: 1234,
    }
}

fn clear_page() -> GatherPage {
    GatherPage {
        token: 0,
        phase: GatherPhase::Running,
        status: None,
    }
}

fn stop_end(token: u64) -> GatherEnd {
    GatherEnd::Stopped {
        token,
        counts: counts(),
    }
}

fn result(iso: &LoadIsolate, label: &str) -> serde_json::Value {
    iso.probe(&format!("globalThis.__{label}Result")).unwrap()
}

fn settles(iso: &LoadIsolate, label: &str) -> u64 {
    iso.probe(&format!("globalThis.__{label}Settles"))
        .unwrap()
        .as_u64()
        .unwrap()
}

fn status() -> Arc<ScriptStatus> {
    let text = |key, label, value: &'static str| StatusField {
        key,
        label,
        value: StatusValue::Text(Arc::from(value)),
    };
    let integer = |key, label, value| StatusField {
        key,
        label,
        value: StatusValue::Integer(value),
    };
    Arc::new(ScriptStatus {
        run: RunKey {
            slot: 9,
            run: 4,
            session: 2,
        },
        card: CompiledId("Gatherer"),
        phase: NativePhase::Working,
        active_settings: 3,
        pending_settings: None,
        fields: vec![
            text("skill", "Skill", "Mining"),
            text("method", "Method", "Copper"),
            text("phase", "Phase", "gathering"),
            text("area", "Area", "Lumbridge"),
            text("target", "Target", "Copper rocks"),
            text("tool", "Tool", "Bronze pickaxe"),
            integer("bait", "Bait", 1),
            integer("food", "Food", 2),
            integer("coins", "Coins", 3),
            integer("yielded", "Yielded", 4),
            integer("dropped", "Dropped", 5),
            integer("deposited", "Deposited", 6),
            integer("trips", "Trips", 7),
            integer("xp", "XP", 8),
            integer("xp_per_hour", "XP/hour", 9),
            text("bank", "Bank", "Varrock"),
            integer("last_progress", "Last progress", 10),
            integer("deaths", "Deaths", 11),
            integer("absent", "Absent", 12),
            integer("zone_gated", "Zone gated", 13),
            text("excluded_targets", "Excluded targets", "tree A"),
            text("last_event", "Last event", "mined copper"),
        ]
        .into(),
        failure: None,
    })
}

#[test]
fn run_marshals_every_settings_kind_and_snapshot_materializes_every_status_member() {
    let iso = isolate();
    assert_eq!(iso.probe("__api.snapshot.gather ?? null").unwrap(), serde_json::Value::Null);

    let settings = serde_json::json!({
        "skill": "Mining",
        "woodcuttingResources": [],
        "miningResources": ["copper"],
        "fishingMethod": "fishing.saltfish.op1",
        "targetPreference": "Nearest",
        "location": "Custom",
        "customTile": {"x": 3200, "z": 3201, "level": 0},
        "radius": 16,
        "disposition": "Power",
        "allowTeleports": true,
        "allowWilderness": true,
        "deathPolicy": "Stop",
        "maxDeaths": 4
    });
    let (token, sent_settings) = begin_and_take_run(&iso, "status", &settings);
    assert_ne!(token, 0);
    assert!(token <= (1_u64 << 53) - 1);
    assert_eq!(sent_settings, settings.as_object().unwrap().clone());

    let preparing = GatherPage {
        token,
        phase: GatherPhase::Preparing,
        status: None,
    };
    tick(&iso, 2, Some(&preparing), None);
    assert_eq!(
        iso.probe("__api.snapshot.gather").unwrap(),
        serde_json::json!({"token": token, "phase": "preparing", "status": null})
    );

    let running = GatherPage {
        token,
        phase: GatherPhase::Running,
        status: Some(status()),
    };
    tick(&iso, 3, Some(&running), None);
    let snapshot = iso.probe("__api.snapshot.gather").unwrap();
    assert_eq!(snapshot["token"], token);
    assert_eq!(snapshot["phase"], "running");
    let live = &snapshot["status"];
    assert_eq!(live["skill"], "Mining");
    assert_eq!(live["method"], "Copper");
    assert_eq!(live["phase"], "gathering");
    assert_eq!(live["area"], "Lumbridge");
    assert_eq!(live["target"], "Copper rocks");
    assert_eq!(live["tool"], "Bronze pickaxe");
    assert_eq!(live["bait"], 1);
    assert_eq!(live["food"], 2);
    assert_eq!(live["coins"], 3);
    assert_eq!(live["yielded"], 4);
    assert_eq!(live["dropped"], 5);
    assert_eq!(live["deposited"], 6);
    assert_eq!(live["trips"], 7);
    assert_eq!(live["xp"], 8);
    assert_eq!(live["xp_per_hour"], 9);
    assert_eq!(live["bank"], "Varrock");
    assert_eq!(live["last_progress"], 10);
    assert_eq!(live["deaths"], 11);
    assert_eq!(live["absent"], 12);
    assert_eq!(live["zone_gated"], 13);
    assert_eq!(live["excluded_targets"], "tree A");
    assert_eq!(live["last_event"], "mined copper");

    // An omitted api_gather delta keeps the materialized value; request_id 0
    // is the explicit live-page clear, not an outcome clear.
    tick(&iso, 4, None, None);
    assert_eq!(iso.probe("__api.snapshot.gather").unwrap(), snapshot);
    let empty = clear_page();
    tick(&iso, 5, Some(&empty), None);
    assert_eq!(iso.probe("__api.snapshot.gather").unwrap(), serde_json::Value::Null);

    let end = stop_end(token);
    tick(&iso, 6, Some(&empty), Some(&end));
    assert_eq!(
        result(&iso, "status"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "stopped", "token": token, "counts": {
                "yielded": 17, "dropped": 16, "deposited": 8, "trips": 2, "xp": 1234
            }}
        })
    );
    iso.join();
}

#[test]
fn terminal_envelopes_busy_stale_tokens_repeated_keyframes_and_normal_drop() {
    let iso = isolate();
    let (blocked_token, _) = begin_and_take_run(&iso, "blocked", &serde_json::json!({}));
    begin(&iso, "busy", &serde_json::json!({}));
    assert_eq!(
        iso.drain_interacts(),
        Vec::<InteractReq>::new(),
        "a second run while live is a synchronous Busy refusal, not a host row"
    );
    tick(&iso, 2, None, None);
    assert_eq!(
        result(&iso, "busy"),
        serde_json::json!({"kind": "refused", "reason": "busy"})
    );
    assert_eq!(result(&iso, "blocked"), serde_json::Value::Null);

    let stale = GatherEnd::Failed {
        token: blocked_token + 1,
        reason: Arc::from("stale token"),
        counts: counts(),
    };
    let empty = clear_page();
    tick(&iso, 3, Some(&empty), Some(&stale));
    assert_eq!(result(&iso, "blocked"), serde_json::Value::Null);
    assert_eq!(settles(&iso, "blocked"), 0);

    let blocked = GatherEnd::Blocked {
        token: blocked_token,
        failure: GatherFailure {
            code: Arc::from("missing-resource"),
            message: Arc::from("selected resource disappeared"),
            retryable: true,
        },
        counts: counts(),
    };
    tick(&iso, 4, Some(&empty), Some(&blocked));
    let blocked_value = serde_json::json!({
        "kind": "done",
        "value": {"end": "blocked", "token": blocked_token,
            "failure": {"code": "missing-resource", "message": "selected resource disappeared", "retryable": true},
            "counts": {"yielded": 17, "dropped": 16, "deposited": 8, "trips": 2, "xp": 1234}}
    });
    assert_eq!(result(&iso, "blocked"), blocked_value);
    assert_eq!(settles(&iso, "blocked"), 1);

    // Repeated retained-terminal keyframes do not settle a removed row again.
    tick(&iso, 5, Some(&empty), Some(&blocked));
    assert_eq!(result(&iso, "blocked"), blocked_value);
    assert_eq!(settles(&iso, "blocked"), 1);

    // Normal Done dropped the old row and released its isolate-local Busy
    // record. The previous terminal is stale for this distinct token.
    let (refused_token, _) = begin_and_take_run(&iso, "host_refused", &serde_json::json!({}));
    assert_ne!(refused_token, blocked_token);
    tick(&iso, 6, Some(&empty), Some(&blocked));
    assert_eq!(result(&iso, "host_refused"), serde_json::Value::Null);
    let refused = GatherEnd::Refused {
        token: refused_token,
        reason: Arc::from("unavailable:selected game data unavailable"),
    };
    tick(&iso, 7, Some(&empty), Some(&refused));
    assert_eq!(
        result(&iso, "host_refused"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "refused", "token": refused_token,
                "reason": "unavailable:selected game data unavailable"}
        })
    );

    let (failed_token, _) = begin_and_take_run(&iso, "failed", &serde_json::json!({}));
    let failed = GatherEnd::Failed {
        token: failed_token,
        reason: Arc::from("card panic"),
        counts: counts(),
    };
    tick(&iso, 8, Some(&empty), Some(&failed));
    assert_eq!(
        result(&iso, "failed"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "failed", "token": failed_token, "reason": "card panic",
                "counts": {"yielded": 17, "dropped": 16, "deposited": 8, "trips": 2, "xp": 1234}}
        })
    );

    let (stopped_token, _) = begin_and_take_run(&iso, "stopped", &serde_json::json!({}));
    let stopped = stop_end(stopped_token);
    tick(&iso, 9, Some(&empty), Some(&stopped));
    assert_eq!(
        result(&iso, "stopped"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "stopped", "token": stopped_token,
                "counts": {"yielded": 17, "dropped": 16, "deposited": 8, "trips": 2, "xp": 1234}}
        })
    );
    iso.join();
}

#[test]
fn synchronous_refusals_stop_idempotence_settings_limit_and_control_forgery() {
    let iso = isolate();
    assert_eq!(
        iso.probe("__api.gather.stop()").unwrap(),
        serde_json::json!({"ok": false, "error": "no-session"})
    );
    let forged = iso
        .probe(
            r#"(() => { try { __api.request({op:'gather-run', request_id:99, settings:{}}); return 'accepted'; }
            catch (e) { return String(e); } })()"#,
        )
        .unwrap();
    assert!(forged.as_str().unwrap().contains("not impl: request.gather-run"));

    for (tick_no, (label, args, expected)) in [
        (
            "bad_shape",
            serde_json::json!(["not settings"]),
            serde_json::json!({"kind": "refused", "reason": "invalid-args"}),
        ),
        (
            "bad_semantic",
            serde_json::json!({"radius": 1}),
            serde_json::json!({"kind": "refused", "reason": "invalid-setting:radius:invalid-radius"}),
        ),
        (
            "bad_shape_field",
            serde_json::json!({"unknownField": true}),
            serde_json::json!({"kind": "refused", "reason": "invalid-settings"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        begin(&iso, label, &args);
        tick(&iso, 2 + tick_no as u64, None, None);
        assert_eq!(result(&iso, label), expected, "{label}");
        assert_eq!(iso.drain_interacts(), Vec::<InteractReq>::new());
    }

    let long_values = vec!["x".repeat(160); 48];
    let too_large = serde_json::json!({"woodcuttingResources": long_values});
    begin(&iso, "too_large", &too_large);
    tick(&iso, 5, None, None);
    assert_eq!(
        result(&iso, "too_large"),
        serde_json::json!({"kind": "refused", "reason": "invalid-settings"})
    );
    assert_eq!(iso.drain_interacts(), Vec::<InteractReq>::new());

    let (token, _) = begin_and_take_run(&iso, "stop", &serde_json::json!({}));
    let first_stop = iso.probe("__api.gather.stop()").unwrap();
    let second_stop = iso.probe("__api.gather.stop()").unwrap();
    assert_eq!(first_stop, serde_json::json!({"ok": true, "value": null}));
    assert_eq!(second_stop, first_stop, "repeated Stop is idempotent");
    let stop_rows: Vec<_> = iso
        .drain_interacts()
        .into_iter()
        .filter_map(|request| match request {
            InteractReq::GatherStop { request_id } => Some(request_id),
            _ => None,
        })
        .collect();
    assert_eq!(stop_rows, vec![token], "idempotent Stop emits only one host row");

    let empty = clear_page();
    let end = stop_end(token);
    tick(&iso, 6, Some(&empty), Some(&end));
    assert_eq!(
        result(&iso, "stop"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "stopped", "token": token,
                "counts": {"yielded": 17, "dropped": 16, "deposited": 8, "trips": 2, "xp": 1234}}
        })
    );
    iso.join();
}

#[test]
fn explicit_reset_aborts_waiter_and_drop_allows_a_fresh_session() {
    let iso = isolate();
    let (first_token, _) = begin_and_take_run(&iso, "reset", &serde_json::json!({}));
    iso.reset_session_work();
    let empty = clear_page();
    tick(&iso, 2, Some(&empty), None);
    assert_eq!(
        result(&iso, "reset"),
        serde_json::json!({"kind": "aborted", "reason": "reset"})
    );
    assert_eq!(
        iso.probe("__api.gather.stop()").unwrap(),
        serde_json::json!({"ok": false, "error": "no-session"})
    );

    let (next_token, _) = begin_and_take_run(&iso, "after_reset", &serde_json::json!({}));
    assert_ne!(next_token, first_token);
    let end = stop_end(next_token);
    tick(&iso, 3, Some(&empty), Some(&end));
    assert_eq!(
        result(&iso, "after_reset")["kind"],
        "done",
        "reset Drop released the Busy record for a new session"
    );
    iso.join();
}

#[test]
fn reconnect_discards_queued_start_then_reemits_once_and_preserves_waiter() {
    let iso = isolate();
    begin(&iso, "queued", &serde_json::json!({}));
    iso.reconnect_session_work();
    assert!(
        iso.drain_interacts().is_empty(),
        "the unforwarded old-epoch start is discarded at reconnect"
    );
    tick(&iso, 2, None, None);
    let (token, _) = take_single_run(&iso.drain_interacts());
    assert_ne!(token, 0);
    assert_eq!(result(&iso, "queued"), serde_json::Value::Null);
    tick(&iso, 3, None, None);
    assert!(iso.drain_interacts().is_empty(), "one carry per new epoch");

    let end = stop_end(token);
    let empty = clear_page();
    tick(&iso, 4, Some(&empty), Some(&end));
    assert_eq!(result(&iso, "queued")["kind"], "done");
    iso.join();
}

#[test]
fn reconnect_reemits_same_unacknowledged_token_after_a_dropped_delivery() {
    let iso = isolate();
    let (token, original_settings) = begin_and_take_run(&iso, "lost_start", &serde_json::json!({}));
    // The test deliberately drops the delivered batch instead of admitting
    // it to a host seat. The row remains pending at the isolate.
    iso.reconnect_session_work();
    assert!(iso.drain_interacts().is_empty(), "no pre-reemit stale row remains");
    tick(&iso, 2, None, None);
    let (replayed_token, replayed_settings) = take_single_run(&iso.drain_interacts());
    assert_eq!(replayed_token, token);
    assert_eq!(replayed_settings, original_settings);
    assert_eq!(result(&iso, "lost_start"), serde_json::Value::Null);

    let end = stop_end(token);
    let empty = clear_page();
    tick(&iso, 3, Some(&empty), Some(&end));
    assert_eq!(result(&iso, "lost_start")["kind"], "done");
    iso.join();
}

#[test]
fn reconnect_carries_lost_stop_without_restarting_an_acknowledged_session() {
    let iso = isolate();
    let (token, _) = begin_and_take_run(&iso, "lost_stop", &serde_json::json!({}));
    let live = GatherPage {
        token,
        phase: GatherPhase::Running,
        status: None,
    };
    tick(&iso, 2, Some(&live), None); // host admission acknowledges start
    assert_eq!(
        iso.probe("__api.gather.stop()").unwrap(),
        serde_json::json!({"ok": true, "value": null})
    );
    // Leave the first Stop in the isolate queue so reconnect must lose it.
    iso.reconnect_session_work();
    assert!(iso.drain_interacts().is_empty(), "old queued Stop is discarded");
    tick(&iso, 3, Some(&live), None); // keyframe re-posts admission
    let carried = iso.drain_interacts();
    let carried_stops: Vec<_> = carried
        .iter()
        .filter_map(|request| match request {
            InteractReq::GatherStop { request_id } => Some(*request_id),
            _ => None,
        })
        .collect();
    assert_eq!(carried_stops, [token]);
    assert!(
        carried
            .iter()
            .all(|request| !matches!(request, InteractReq::GatherRun { .. })),
        "an acknowledged session is not restarted to carry Stop"
    );

    let empty = clear_page();
    let end = stop_end(token);
    tick(&iso, 4, Some(&empty), Some(&end));
    assert_eq!(result(&iso, "lost_stop")["kind"], "done");
    iso.join();
}

#[test]
fn lost_start_and_stop_in_one_epoch_reemit_in_start_then_stop_order() {
    let iso = isolate();
    let (token, _) = begin_and_take_run(&iso, "both_lost", &serde_json::json!({}));
    assert_eq!(
        iso.probe("__api.gather.stop()").unwrap(),
        serde_json::json!({"ok": true, "value": null})
    );
    iso.reconnect_session_work();
    assert!(iso.drain_interacts().is_empty(), "both old-epoch rows were lost");

    tick(&iso, 2, None, None);
    let carried = iso.drain_interacts();
    let controls: Vec<_> = carried
        .iter()
        .filter_map(|request| match request {
            InteractReq::GatherRun { request_id, .. } => Some(("run", *request_id)),
            InteractReq::GatherStop { request_id } => Some(("stop", *request_id)),
            _ => None,
        })
        .collect();
    assert_eq!(controls, vec![("run", token), ("stop", token)]);
    tick(&iso, 3, None, None);
    assert!(iso.drain_interacts().is_empty(), "start and Stop carry once in this epoch");

    let empty = clear_page();
    let end = stop_end(token);
    tick(&iso, 4, Some(&empty), Some(&end));
    assert_eq!(result(&iso, "both_lost")["kind"], "done");
    iso.join();
}

#[test]
fn terminal_in_reconnect_keyframe_settles_before_any_start_or_stop_carry() {
    let iso = isolate();
    let (token, _) = begin_and_take_run(&iso, "terminal_keyframe", &serde_json::json!({}));
    iso.reconnect_session_work();
    assert!(iso.drain_interacts().is_empty());

    let empty = clear_page();
    let terminal = stop_end(token);
    tick(&iso, 2, Some(&empty), Some(&terminal));
    assert!(iso.drain_interacts().is_empty(), "matching terminal wins before carry");
    assert_eq!(
        result(&iso, "terminal_keyframe"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "stopped", "token": token,
                "counts": {"yielded": 17, "dropped": 16, "deposited": 8, "trips": 2, "xp": 1234}}
        })
    );
    iso.join();
}
