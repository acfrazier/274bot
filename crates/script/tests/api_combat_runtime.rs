#![cfg(feature = "load")]

//! Consumer-visible runtime probes for the JS API v2 combat session.
//! These drive the real V8 isolate and its machine host while supplying the
//! same typed FlatBuffer snapshot pages a Load slot publishes.
use std::sync::Arc;

use api::selected::RunKey;
use script::api_combat::{
    CombatEnd, CombatPage, CombatSessionRequest, CombatSummary, InterruptCause, ReportEnd,
    TargetSpec,
};
use script::api_gather::GatherPhase;
use script::combat::{MeleeMode, Pick, Style};
use script::isolate_fb::{
    encode_snapshot_delta_with_native, NativeFactsInput, SnapshotFingerprint, SnapshotInput,
};
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
    api_combat: Option<&CombatPage>,
    api_combat_outcome: Option<&CombatEnd>,
    last: Option<&SnapshotFingerprint>,
) -> SnapshotFingerprint {
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
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    };
    let native = NativeFactsInput {
        api_combat,
        api_combat_outcome,
        ..NativeFactsInput::default()
    };
    let (bytes, fingerprint) = encode_snapshot_delta_with_native(last, &input, native, false);
    iso.post_snapshot(bytes);
    fingerprint
}

fn tick(
    iso: &LoadIsolate,
    tick: u64,
    api_combat: Option<&CombatPage>,
    api_combat_outcome: Option<&CombatEnd>,
) -> SnapshotFingerprint {
    tick_with_last(iso, tick, api_combat, api_combat_outcome, None)
}

fn tick_with_last(
    iso: &LoadIsolate,
    tick: u64,
    api_combat: Option<&CombatPage>,
    api_combat_outcome: Option<&CombatEnd>,
    last: Option<&SnapshotFingerprint>,
) -> SnapshotFingerprint {
    let fingerprint = post_snapshot(iso, tick, api_combat, api_combat_outcome, last);
    iso.on_game_tick(tick);
    let _ = iso.probe("true").unwrap();
    fingerprint
}

fn isolate() -> LoadIsolate {
    let iso = LoadIsolate::spawn(BASE_SCRIPT.into(), LoadShape::NativeTick, vec![]).unwrap();
    tick(&iso, 1, None, None);
    iso
}

fn begin(iso: &LoadIsolate, label: &str, request: &serde_json::Value) {
    let request = serde_json::to_string(request).unwrap();
    let expression = format!(
        r#"(() => {{
          globalThis.__{label}Result = null;
          globalThis.__{label}Settles = 0;
          const promise = __api.combat.fight({request});
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

fn call_raw(iso: &LoadIsolate, label: &str, args_src: &str) {
    let expression = format!(
        r#"(() => {{
          globalThis.__{label}Result = null;
          globalThis.__{label}Settles = 0;
          const promise = __api.combat.fight({args_src});
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

fn forward_queued_controls(iso: &LoadIsolate) {
    let current_tick = iso.probe("__api.tick").unwrap();
    let current_tick = current_tick
        .as_f64()
        .unwrap_or_else(|| panic!("non-numeric API tick: {current_tick}"))
        as u64;
    iso.on_game_tick(current_tick);
    let _ = iso.probe("true").unwrap();
}

fn begin_and_take_fight(
    iso: &LoadIsolate,
    label: &str,
    request: &serde_json::Value,
) -> (u64, Arc<CombatSessionRequest>) {
    begin(iso, label, request);
    forward_queued_controls(iso);
    let requests = iso.drain_interacts();
    take_single_fight(&requests)
}

fn take_single_fight(requests: &[InteractReq]) -> (u64, Arc<CombatSessionRequest>) {
    let fights: Vec<_> = requests
        .iter()
        .filter_map(|request| match request {
            InteractReq::CombatFight {
                request_id,
                request,
            } => Some((*request_id, Arc::clone(request))),
            _ => None,
        })
        .collect();
    assert_eq!(fights.len(), 1, "expected one combat-fight in {requests:?}");
    fights.into_iter().next().unwrap()
}

fn clear_page() -> CombatPage {
    CombatPage {
        token: 0,
        phase: GatherPhase::Running,
        status: None,
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

fn combat_status() -> Arc<ScriptStatus> {
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
        card: CompiledId("Combat"),
        phase: NativePhase::Working,
        active_settings: 3,
        pending_settings: None,
        fields: vec![
            text("stage", "Stage", "fighting"),
            text("engaged_kind", "Engaged kind", "npc"),
            integer("engaged_index", "Engaged index", 7),
        ]
        .into(),
        failure: None,
    })
}

fn killed_report() -> CombatSummary {
    CombatSummary {
        end: ReportEnd::Killed,
        reason: None,
        npc_type: 81,
        ticks: 40,
        swings: 9,
        casts: 0,
        damage_taken: 3,
        food: 1,
        prayer_doses: 0,
        boost_doses: 0,
        antifire_doses: 0,
        protect_switches: 0,
    }
}

#[test]
fn fight_marshals_the_request_and_settles_once_with_the_fought_report() {
    let iso = isolate();
    assert_eq!(
        iso.probe("__api.snapshot.combat ?? null").unwrap(),
        serde_json::Value::Null
    );

    let request = serde_json::json!({
        "target": { "npc": ["cow", "Chicken"] },
        "pick": "lowestHealth",
        "area": [{ "min_x": 3253, "min_z": 3255, "max_x": 3265, "max_z": 3297, "level": 0 }],
        "radius": 9,
        "style": "melee",
        "meleeMode": "aggressive",
        "prayer": false,
        "food": true,
        "potions": false,
        "retaliate": true,
        "budgetTicks": 300
    });
    let (token, sent) = begin_and_take_fight(&iso, "fight", &request);
    assert_ne!(token, 0);
    assert!(token < (1_u64 << 53));
    assert_eq!(
        sent.target,
        TargetSpec::Names(
            vec![Arc::<str>::from("cow"), Arc::<str>::from("Chicken")].into_boxed_slice()
        )
    );
    assert_eq!(sent.pick, Pick::LowestHealth);
    assert_eq!(sent.area.as_ref(), &[[3253, 3255, 3265, 3297, 0]]);
    assert_eq!(sent.radius, 9);
    assert_eq!(sent.style, Style::Melee);
    assert_eq!(sent.melee_mode, Some(MeleeMode::Aggressive));
    assert!(!sent.prayer);
    assert!(sent.food);
    assert!(!sent.potions);
    assert!(sent.retaliate);
    assert_eq!(sent.budget_ticks, 300);
    assert_eq!(sent.lost_radius, 20);

    let preparing = CombatPage {
        token,
        phase: GatherPhase::Preparing,
        status: None,
    };
    tick(&iso, 2, Some(&preparing), None);
    assert_eq!(
        iso.probe("__api.snapshot.combat").unwrap(),
        serde_json::json!({"token": token, "phase": "preparing", "status": null})
    );

    let running = CombatPage {
        token,
        phase: GatherPhase::Running,
        status: Some(combat_status()),
    };
    tick(&iso, 3, Some(&running), None);
    assert_eq!(
        iso.probe("__api.snapshot.combat").unwrap(),
        serde_json::json!({
            "token": token,
            "phase": "running",
            "status": {"stage": "fighting", "engaged_kind": "npc", "engaged_index": 7}
        })
    );

    let end = CombatEnd::Fought {
        token,
        report: killed_report(),
    };
    let empty = clear_page();
    tick(&iso, 4, Some(&empty), Some(&end));
    let expected = serde_json::json!({
        "kind": "done",
        "value": {
            "end": "fought",
            "token": token,
            "report": {
                "end": "killed",
                "reason": null,
                "npcType": 81,
                "ticks": 40,
                "swings": 9,
                "casts": 0,
                "damageTaken": 3,
                "food": 1,
                "prayerDoses": 0,
                "boostDoses": 0,
                "antifireDoses": 0,
                "protectSwitches": 0
            }
        }
    });
    assert_eq!(result(&iso, "fight"), expected);
    assert_eq!(settles(&iso, "fight"), 1);
    assert_eq!(
        iso.probe("__api.snapshot.combat").unwrap(),
        serde_json::Value::Null
    );

    // A repeated retained-terminal keyframe does not settle the removed row again.
    tick(&iso, 5, Some(&empty), Some(&end));
    assert_eq!(result(&iso, "fight"), expected);
    assert_eq!(settles(&iso, "fight"), 1);
    iso.join();
}

#[test]
fn sync_refusals_admit_no_session() {
    let iso = isolate();
    assert_eq!(
        iso.probe("__api.combat.stop()").unwrap(),
        serde_json::json!({"ok": false, "error": "no-session"})
    );

    for (offset, (label, args)) in [
        ("no_args", ""),
        ("numeric", "5"),
        ("array", "[]"),
        (
            "unknown_key",
            r#"{"target": {"npc": "cow"}, "tactic": "open"}"#,
        ),
        (
            "fractional_radius",
            r#"{"target": {"npc": "cow"}, "radius": 1.5}"#,
        ),
        ("missing_target", "{}"),
        (
            "ambiguous_target",
            r#"{"target": {"npc": "cow", "npcId": 81}}"#,
        ),
        (
            "pick_without_npc",
            r#"{"target": {"attackers": "npcs"}, "pick": "nearest"}"#,
        ),
        (
            "melee_mode_without_melee",
            r#"{"target": {"npc": "cow"}, "style": "ranged", "meleeMode": "accurate"}"#,
        ),
        (
            "spells_without_magic",
            r#"{"target": {"npc": "cow"}, "spells": ["wind_strike"]}"#,
        ),
        ("zero_radius", r#"{"target": {"npc": "cow"}, "radius": 0}"#),
    ]
    .into_iter()
    .enumerate()
    {
        call_raw(&iso, label, args);
        tick(&iso, 2 + offset as u64, None, None);
        let expected = match label {
            "no_args" | "numeric" | "array" => "invalid-args",
            "unknown_key" | "fractional_radius" => "invalid-settings",
            "missing_target" => "invalid-setting:target:required",
            "ambiguous_target" => "invalid-setting:target:ambiguous",
            "pick_without_npc" => "invalid-setting:pick:requires-npc-target",
            "melee_mode_without_melee" => "invalid-setting:meleeMode:requires-melee",
            "spells_without_magic" => "invalid-setting:spells:requires-magic",
            "zero_radius" => "invalid-setting:radius:out-of-range",
            _ => unreachable!(),
        };
        assert_eq!(
            result(&iso, label),
            serde_json::json!({"kind": "refused", "reason": expected}),
            "{label}"
        );
        assert_eq!(
            iso.drain_interacts(),
            Vec::<InteractReq>::new(),
            "{label} must not emit a host row"
        );
    }
    iso.join();
}

#[test]
fn second_fight_is_busy_and_stop_settles_stopped() {
    let iso = isolate();
    let valid = serde_json::json!({"target": {"npc": "cow"}});
    let (token, _) = begin_and_take_fight(&iso, "first", &valid);

    begin(&iso, "second", &valid);
    assert_eq!(
        iso.drain_interacts(),
        Vec::<InteractReq>::new(),
        "a second fight while live is a synchronous Busy refusal, not a host row"
    );
    tick(&iso, 2, None, None);
    assert_eq!(
        result(&iso, "second"),
        serde_json::json!({"kind": "refused", "reason": "busy"})
    );
    assert_eq!(result(&iso, "first"), serde_json::Value::Null);

    assert_eq!(
        iso.probe(
            r#"(() => {
              globalThis.__gatherBusyResult = null;
              __api.gather.run({}).then((outcome) => {
                globalThis.__gatherBusyResult = outcome;
              });
              return true;
            })()"#
        )
        .unwrap(),
        true
    );
    tick(&iso, 3, None, None);
    assert_eq!(
        iso.probe("globalThis.__gatherBusyResult").unwrap(),
        serde_json::json!({"kind": "refused", "reason": "busy"})
    );

    let first_stop = iso.probe("__api.combat.stop()").unwrap();
    let second_stop = iso.probe("__api.combat.stop()").unwrap();
    assert_eq!(first_stop, serde_json::json!({"ok": true, "value": null}));
    assert_eq!(second_stop, first_stop, "repeated Stop is idempotent");
    forward_queued_controls(&iso);
    let stop_rows: Vec<_> = iso
        .drain_interacts()
        .into_iter()
        .filter_map(|request| match request {
            InteractReq::CombatStop { request_id } => Some(request_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        stop_rows,
        vec![token],
        "idempotent Stop emits only one host row"
    );

    let empty = clear_page();
    let end = CombatEnd::Stopped { token };
    tick(&iso, 4, Some(&empty), Some(&end));
    assert_eq!(
        result(&iso, "first"),
        serde_json::json!({"kind": "done", "value": {"end": "stopped", "token": token}})
    );
    assert_eq!(settles(&iso, "first"), 1);

    let (later_token, _) = begin_and_take_fight(&iso, "third", &valid);
    assert_ne!(later_token, token);
    iso.join();
}

#[test]
fn host_terminals_map_to_their_json() {
    let iso = isolate();
    let valid = serde_json::json!({"target": {"npc": "cow"}});
    let empty = clear_page();
    let mut next_tick = 2;

    let (interrupted_token, _) = begin_and_take_fight(&iso, "interrupted", &valid);
    let interrupted = CombatEnd::Interrupted {
        token: interrupted_token,
        cause: InterruptCause::Pause,
    };
    tick(&iso, next_tick, Some(&empty), Some(&interrupted));
    next_tick += 1;
    assert_eq!(
        result(&iso, "interrupted"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "interrupted", "token": interrupted_token, "cause": "pause"}
        })
    );

    let (refused_token, _) = begin_and_take_fight(&iso, "refused", &valid);
    let refused = CombatEnd::Refused {
        token: refused_token,
        reason: Arc::from("invalid-setting:target:unknown-npc"),
    };
    tick(&iso, next_tick, Some(&empty), Some(&refused));
    next_tick += 1;
    assert_eq!(
        result(&iso, "refused"),
        serde_json::json!({
            "kind": "done",
            "value": {
                "end": "refused",
                "token": refused_token,
                "reason": "invalid-setting:target:unknown-npc"
            }
        })
    );

    let (failed_token, _) = begin_and_take_fight(&iso, "failed", &valid);
    let failed = CombatEnd::Failed {
        token: failed_token,
        reason: Arc::from("failed:x"),
    };
    tick(&iso, next_tick, Some(&empty), Some(&failed));
    next_tick += 1;
    assert_eq!(
        result(&iso, "failed"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "failed", "token": failed_token, "reason": "failed:x"}
        })
    );

    let (aborted_token, _) = begin_and_take_fight(&iso, "aborted", &valid);
    let aborted = CombatEnd::Fought {
        token: aborted_token,
        report: CombatSummary {
            end: ReportEnd::Aborted,
            reason: Some(Arc::from("no-food")),
            ..killed_report()
        },
    };
    tick(&iso, next_tick, Some(&empty), Some(&aborted));
    assert_eq!(
        result(&iso, "aborted"),
        serde_json::json!({
            "kind": "done",
            "value": {
                "end": "fought",
                "token": aborted_token,
                "report": {
                    "end": "aborted",
                    "reason": "no-food",
                    "npcType": 81,
                    "ticks": 40,
                    "swings": 9,
                    "casts": 0,
                    "damageTaken": 3,
                    "food": 1,
                    "prayerDoses": 0,
                    "boostDoses": 0,
                    "antifireDoses": 0,
                    "protectSwitches": 0
                }
            }
        })
    );
    iso.join();
}
