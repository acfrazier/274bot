use api::identity::SessionKey;
use api::run_policy::{RunEnergyMin, RunPolicyOverride};
use api::selected::{ClientRevision, RunKey};
use script::ctx::test_support::NullDriver;
use script::ctx::{CompiledTick, ScriptCtx};
use script::native::snapshot_source::{NativeFactsInput, SnapshotInput, TileInput};
use script::slot::{RunState, SlotScript};
use script::surface::{SurfaceCard, SurfaceStart};
use script_js::surface::{JsStartCard, JsSurfaceFactory, JsSurfaceRun};
use script_js::{resolve_api_family, LoadShape};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn js_slot(incarnation: u64) -> SlotScript {
    let mut slot = SlotScript::new();
    if incarnation != 0 {
        slot.bind_incarnation(incarnation);
    }
    slot.bind_surface_factory(Arc::new(JsSurfaceFactory::default()));
    slot
}

fn surface_start(
    source: impl Into<Arc<str>>,
    shape: LoadShape,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
) -> SurfaceStart {
    let source = source.into();
    let family = resolve_api_family(&source).ok().map(|(_, family)| family);
    let card: Arc<dyn SurfaceCard> = Arc::new(JsStartCard {
        source,
        shape,
        siblings: Arc::from(Vec::<(String, String)>::new()),
        family,
    });
    SurfaceStart {
        card,
        settings_bag: None,
        game_data,
        named_banks: Arc::new(api::named_banks::NamedBankFacts::empty()),
        loadouts: Arc::from(Vec::<api::loadout::Loadout>::new()),
    }
}

fn start_js(
    slot: &mut SlotScript,
    source: impl Into<Arc<str>>,
    shape: LoadShape,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
) {
    slot.start_surface(surface_start(source, shape, game_data))
        .expect("start JavaScript surface");
}

fn start_js_running(
    slot: &mut SlotScript,
    source: impl Into<Arc<str>>,
    shape: LoadShape,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
) {
    start_js(slot, source, shape, game_data);
    wait_state(slot, RunState::Running);
}

fn wait_state(slot: &mut SlotScript, want: RunState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while slot.state() != want && Instant::now() < deadline {
        let _ = slot.drain_logs();
        std::thread::yield_now();
    }
}

fn wait_for_receipt(slot: &mut SlotScript) -> Option<script::slot::ScriptLifecycleReceipt> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while slot.lifecycle_receipt().is_none() && Instant::now() < deadline {
        let _ = slot.drain_logs();
        std::thread::yield_now();
    }
    slot.lifecycle_receipt()
}

fn probe(slot: &mut SlotScript, expr: &str) -> serde_json::Value {
    slot.with_surface_run_for_test(|run| run.probe(expr).expect("JavaScript probe"))
        .expect("active JavaScript surface")
}

fn surface_tick(slot: &mut SlotScript, tick: u64) {
    slot.with_surface_run_for_test(|run| run.on_game_tick_at(tick, 0))
        .expect("active JavaScript surface");
    let _ = probe(slot, "true");
}

fn slot_tick(slot: &mut SlotScript, tick: u64, here: Option<(i32, i32, i32)>) {
    let mut driver = NullDriver::default();
    let mut ctx = ScriptCtx {
        driver: &mut driver,
        tick,
        here,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: CompiledTick::default(),
    };
    slot.on_game_tick(&mut ctx);
    let _ = probe(slot, "true");
}

fn allow_onstop_completion(slot: &mut SlotScript) {
    slot.with_surface_run_for_test(|run| {
        run.as_any_mut()
            .downcast_mut::<JsSurfaceRun>()
            .expect("JavaScript surface run")
            .set_onstop_timeout_for_test(Duration::from_secs(1));
    })
    .expect("active JavaScript surface");
}

fn empty_snapshot(tick: u64) -> SnapshotInput<'static> {
    SnapshotInput {
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
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
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
        reach: script::native::snapshot_source::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn run_policy_tick(slot: &mut SlotScript, tick: u64, session_epoch: u64) {
    let key = slot.slot_key_for_test().expect("bound test slot");
    slot.align_seat_session(Some(SessionKey::new(key, session_epoch)));
    let input = empty_snapshot(tick);
    let _ = slot.post_snapshot_delta(&input, NativeFactsInput::default(), false);
    slot_tick(slot, tick, None);
}

fn arm_test_seat(slot: &mut SlotScript, session_epoch: u64) {
    let key = slot.slot_key_for_test().expect("bound test slot");
    slot.arm_seat_for_test(RunKey {
        slot: key.incarnation().get(),
        run: slot.runtime_generation(),
        session: session_epoch,
    });
    slot.align_seat_session(Some(SessionKey::new(key, session_epoch)));
}

fn drain_run_policy_update(slot: &mut SlotScript) -> Option<RunPolicyOverride> {
    let (policy, requests, _) = slot.drain_host_interacts();
    assert!(
        requests
            .iter()
            .all(|row| matches!(&row.req, script::interact::InteractReq::LoopSettled)),
        "unexpected run-policy sibling: {requests:?}"
    );
    policy.unwrap_or_else(|| {
        let logs = slot.drain_logs();
        panic!(
            "run-policy replacement did not cross the JavaScript wire batch; state={:?}, logs={logs:?}",
            slot.state()
        )
    })
}

fn finish_until_idle(slot: &mut SlotScript) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut logs = Vec::new();
    while slot.state() != RunState::Idle && Instant::now() < deadline {
        logs.extend(slot.drain_logs());
        std::thread::yield_now();
    }
    logs.extend(slot.drain_logs());
    logs
}

#[test]
fn js_ticks_emit_errors_and_user_logs() {
    let source = r#"
export function tick() {
    const n = globalThis.__ticks || 0;
    globalThis.__ticks = n + 1;
    if (n < 2) throw new Error(n === 0 ? 'first tick failure' : 'second tick failure');
    globalThis.__rs2b0t_host.log = ['tick completed normally'];
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::NativeTick, None);

    surface_tick(&mut slot, 1);
    assert_eq!(probe(&mut slot, "globalThis.__ticks"), serde_json::json!(1));
    let first_logs = slot.drain_logs();
    assert!(
        first_logs
            .iter()
            .any(|line| line.contains("first tick failure")),
        "{first_logs:?}"
    );

    surface_tick(&mut slot, 2);
    assert_eq!(probe(&mut slot, "globalThis.__ticks"), serde_json::json!(2));
    let second_logs = slot.drain_logs();
    assert!(
        second_logs
            .iter()
            .any(|line| line.contains("second tick failure")),
        "{second_logs:?}"
    );

    surface_tick(&mut slot, 3);
    assert_eq!(probe(&mut slot, "globalThis.__ticks"), serde_json::json!(3));
    let third_logs = slot.drain_logs();
    assert!(
        third_logs
            .iter()
            .any(|line| line == "tick completed normally"),
        "{third_logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_reconnect_runs_a_clean_loop() {
    let source = r#"
export function tick() {
    globalThis.__ticks = (globalThis.__ticks || 0) + 1;
    if (globalThis.__ticks === 1) throw new Error('session-one failure');
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::NativeTick, None);
    surface_tick(&mut slot, 1);
    let _ = probe(&mut slot, "true");
    let _ = slot.drain_logs();

    slot.reset_session_work();
    slot.on_is_up(true);
    surface_tick(&mut slot, 2);
    assert_eq!(probe(&mut slot, "globalThis.__ticks"), serde_json::json!(2));
    let _ = slot.drain_logs();
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn parked_async_iteration_remains_parked() {
    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__loops = (globalThis.__loops || 0) + 1;
        await Execution.delayTicks(1);
        throw new Error('async iteration failure');
    }
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);

    surface_tick(&mut slot, 1);
    let _ = probe(&mut slot, "true");
    let _ = slot.drain_logs();
    surface_tick(&mut slot, 2);
    let _ = probe(&mut slot, "true");
    let _ = slot.drain_logs();

    surface_tick(&mut slot, 3);
    let _ = probe(&mut slot, "true");
    let _ = slot.drain_logs();
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn completed_slow_js_tick_emits_slow_tick_log() {
    let source = r#"
export function tick() {
    globalThis.__ticks = (globalThis.__ticks || 0) + 1;
    if (globalThis.__ticks === 1) throw new Error('transient failure');
    const started = Date.now();
    while (Date.now() - started < 700) {}
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::NativeTick, None);
    surface_tick(&mut slot, 1);
    let _ = probe(&mut slot, "true");
    let _ = slot.drain_logs();

    surface_tick(&mut slot, 2);
    let _ = probe(&mut slot, "true");
    let logs = slot.drain_logs();
    assert!(
        logs.iter().any(|line| line.starts_with("slow tick 2:")),
        "{logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn run_manager_override_last_call_replaces_wire_snapshot() {
    let source = r#"
import { RunManager } from '../../runtime/RunManager.js';
export default class T extends LoopingBot {
    loop() {
        RunManager.override({ runAuto: false, energyMin: 80 });
        RunManager.override({ energyMin: 55 });
    }
}
"#;
    let mut slot = js_slot(1201);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    arm_test_seat(&mut slot, 0);
    run_policy_tick(&mut slot, 1, 0);
    assert_eq!(
        drain_run_policy_update(&mut slot),
        Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::Floor(55)),
        }),
        "the second call replaces the whole wire snapshot"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn run_manager_override_uses_frozen_javascript_coercions() {
    let source = r#"
import { RunManager } from '../../runtime/RunManager.js';
export default class T extends LoopingBot {
    loop() {
        const step = this.step || 0;
        switch (step) {
            case 0:
                globalThis.__override_return = RunManager.override({ energyMin: Infinity });
                break;
            case 1:
                RunManager.override({ energyMin: -Infinity });
                break;
            case 2:
                RunManager.override({ energyMin: NaN });
                break;
            case 3:
                RunManager.override({ energyMin: '50' });
                break;
            case 4:
                RunManager.override({ runAuto: 0, energyMin: '50' });
                break;
            case 5:
                RunManager.override({ runAuto: '' });
                break;
            case 6:
                RunManager.override({ runAuto: {} });
                break;
            case 7:
                RunManager.override({ runAuto: null, energyMin: null });
                break;
        }
        this.step = step + 1;
    }
}
"#;
    let mut slot = js_slot(1202);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    arm_test_seat(&mut slot, 0);

    let expected = [
        Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::Floor(100)),
        }),
        Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::Floor(0)),
        }),
        Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::NotANumber),
        }),
        Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::Floor(50)),
        }),
        Some(RunPolicyOverride {
            run_auto: Some(false),
            energy_min: Some(RunEnergyMin::Floor(50)),
        }),
        Some(RunPolicyOverride {
            run_auto: Some(false),
            energy_min: None,
        }),
        Some(RunPolicyOverride {
            run_auto: Some(true),
            energy_min: None,
        }),
        None,
    ];
    for (index, expected) in expected.into_iter().enumerate() {
        run_policy_tick(&mut slot, index as u64 + 1, 0);
        assert_eq!(drain_run_policy_update(&mut slot), expected, "case {index}");
        if index == 0 {
            assert_eq!(
                probe(&mut slot, "__override_return === undefined"),
                serde_json::json!(true),
                "RunManager.override returns the frozen undefined value"
            );
        }
    }
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_run_manager_override_republishes_after_session_reset() {
    let source = r#"
import { RunManager } from '../../runtime/RunManager.js';
export default class T extends LoopingBot {
    loop() {
        if (this.overridden) return;
        this.overridden = true;
        RunManager.override({ runAuto: false });
    }
}
"#;
    let mut slot = js_slot(1203);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    arm_test_seat(&mut slot, 0);
    run_policy_tick(&mut slot, 1, 0);

    slot.reset_session_work();
    let _ = slot.drain_host_interacts();
    slot.on_is_up(true);
    run_policy_tick(&mut slot, 2, 1);
    assert_eq!(
        drain_run_policy_update(&mut slot),
        Some(RunPolicyOverride {
            run_auto: Some(false),
            energy_min: None,
        }),
        "the JavaScript run re-publishes the run-scoped override in the new generation"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn script_requested_stop_emits_stopped_receipt() {
    let source = r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() {
        (globalThis.__rs2b0t_host.interact ||= []).push({op: 'set-camera-yaw', yaw: 123});
        ScriptRunner.stop('finished');
    }
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    surface_tick(&mut slot, 1);
    let _ = slot.drain_logs();
    assert!(slot
        .last_error()
        .is_some_and(|error| error.contains("script requested stop")));
    let receipt = wait_for_receipt(&mut slot).expect("script Stop receipt");
    assert_eq!(receipt.runtime_generation, 1);
    assert_eq!(receipt.state, script::slot::ScriptTerminalState::Stopped);
    assert_eq!(receipt.tick, 1);
    assert_eq!(receipt.reason, "finished");
}

#[test]
fn script_stop_receipt_bounds_utf8_reason() {
    let reason = "🙂".repeat(100);
    let source = format!(
        r#"
import {{ ScriptRunner }} from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {{
  loop() {{ ScriptRunner.stop({reason:?}); }}
}}
"#
    );
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    surface_tick(&mut slot, 1);
    let receipt = wait_for_receipt(&mut slot).expect("script Stop receipt");
    assert_eq!(receipt.state, script::slot::ScriptTerminalState::Stopped);
    assert!(receipt.reason.len() <= 256);
    assert!(receipt.reason.chars().all(|ch| ch == '🙂'));
}

#[test]
fn request_finish_emits_clean_stopped_receipt() {
    let source = r#"
export default class T extends LoopingBot {
    loop() {
        this.requestFinish("BankFletcher: bank is out of 'Willow logs'");
    }
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    surface_tick(&mut slot, 1);
    let receipt = wait_for_receipt(&mut slot);
    assert_eq!(
        receipt,
        Some(script::slot::ScriptLifecycleReceipt {
            runtime_generation: 1,
            state: script::slot::ScriptTerminalState::Stopped,
            tick: 1,
            reason: "BankFletcher: bank is out of 'Willow logs'".into(),
        }),
        "a finish is a clean Stopped receipt, never Failed"
    );
}

#[test]
fn bound_finish_sink_does_not_stop_the_js_script() {
    let source = r#"
export default class T extends LoopingBot {
    onStart() {
        this.bindFinish((reason) => this.log(`finish-sink:${reason}:delegated=${this.delegated}`));
    }
    loop() {
        if (!this._fired) {
            this._fired = true;
            this.requestFinish('child done');
        }
    }
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut logs: Vec<String> = Vec::new();
    let mut tick = 0;
    while !logs
        .iter()
        .any(|line| line.contains("finish-sink:child done:delegated=true"))
        && Instant::now() < deadline
    {
        tick += 1;
        surface_tick(&mut slot, tick);
        logs.extend(slot.drain_logs());
        std::thread::yield_now();
    }
    assert!(
        logs.iter()
            .any(|line| line.contains("finish-sink:child done:delegated=true")),
        "the bound sink heard the reason: {logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn banking_module_links_open_bank_access() {
    let source = r#"
import { openBankAccess } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    loop() {
        this.log(`openBankAccess:${typeof openBankAccess}`);
        this.requestFinish('linked');
    }
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    surface_tick(&mut slot, 1);
    let logs = finish_until_idle(&mut slot);
    assert!(
        logs.iter()
            .any(|line| line.contains("openBankAccess:function")),
        "the Banking module provides openBankAccess: {logs:?}"
    );
    assert_eq!(
        slot.lifecycle_receipt().map(|receipt| receipt.reason),
        Some("linked".into()),
        "{logs:?}"
    );
}

#[test]
fn javascript_onstop_emits_final_log_once() {
    let mut slot = js_slot(0);
    start_js_running(
        &mut slot,
        "export default class T extends LoopingBot {\n            loop() {}\n            onStop() { this.log('stopped-ok'); }\n        }",
        LoadShape::CompatClass,
        None,
    );
    surface_tick(&mut slot, 1);
    let _ = probe(&mut slot, "1");
    allow_onstop_completion(&mut slot);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    let logs = slot.take_pending_logs();
    let hits = logs
        .iter()
        .filter(|line| line.contains("stopped-ok"))
        .count();
    assert_eq!(hits, 1, "exactly one onStop log after take: {logs:?}");
}

#[test]
fn javascript_self_stop_runs_onstop_once() {
    let source = r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() { ScriptRunner.stop('done'); }
    onStop() { this.log('stopped-ok'); }
}
"#;
    let mut slot = js_slot(0);
    start_js_running(&mut slot, source, LoadShape::CompatClass, None);
    allow_onstop_completion(&mut slot);
    surface_tick(&mut slot, 1);
    let mut logs = finish_until_idle(&mut slot);
    logs.extend(slot.take_pending_logs());
    let hits = logs
        .iter()
        .filter(|line| line.contains("stopped-ok"))
        .count();
    assert_eq!(hits, 1, "self-stop onStop once: {logs:?}");
    slot.stop();
}

#[test]
fn watchdog_restart_starts_new_js_isolate() {
    let mut slot = js_slot(0);
    start_js_running(
        &mut slot,
        "export default class T extends LoopingBot {\n            onStart() { globalThis.__gen = (globalThis.__gen || 0) + 1; }\n            loop() { globalThis.__n = (globalThis.__n || 0) + 1; }\n            onStop() { this.log('stopped-ok'); }\n        }",
        LoadShape::CompatClass,
        None,
    );
    surface_tick(&mut slot, 1);
    let _ = probe(&mut slot, "1");
    allow_onstop_completion(&mut slot);
    slot.restart_from_identity(Instant::now())
        .expect("restart from identity");
    wait_state(&mut slot, RunState::Running);
    surface_tick(&mut slot, 1);
    assert_eq!(
        probe(&mut slot, "__gen"),
        serde_json::json!(1),
        "new isolate onStart runs"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_surface_start_runs_after_v8_setup() {
    let mut slot = js_slot(0);
    start_js(
        &mut slot,
        "export function tick(api) { globalThis.__n = (globalThis.__n || 0) + 1; }",
        LoadShape::NativeTick,
        None,
    );
    wait_state(&mut slot, RunState::Running);
    surface_tick(&mut slot, 1);
    assert_eq!(probe(&mut slot, "globalThis.__n"), serde_json::json!(1));
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_surface_setup_failure_preserves_v8_diagnostic() {
    let mut slot = js_slot(0);
    start_js(
        &mut slot,
        "throw new Error('zz-setup-proof');\nexport function tick(api) {}",
        LoadShape::NativeTick,
        None,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while slot.last_error().is_none() && Instant::now() < deadline {
        let _ = slot.drain_logs();
        std::thread::yield_now();
    }
    let error = slot.last_error().unwrap_or("").to_string();
    assert!(error.contains("zz-setup-proof"), "{error}");
}

#[test]
fn js_onstop_completes_before_queued_start() {
    let src = "export default class T extends LoopingBot {\n            loop() {}\n            onStop() { this.log('stopped-ok'); }\n        }";
    let mut slot = js_slot(0);
    start_js_running(&mut slot, src, LoadShape::CompatClass, None);
    allow_onstop_completion(&mut slot);
    slot.stop();
    start_js(&mut slot, src, LoadShape::CompatClass, None);
    wait_state(&mut slot, RunState::Running);
    let logs = slot.take_pending_logs();
    assert!(
        logs.iter().any(|line| line.contains("stopped-ok")),
        "reaped onStop before the queued Start: {logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_surface_start_all_over_n_members_does_not_block() {
    let src = "export function tick(api) { globalThis.__n = (globalThis.__n || 0) + 1; }";
    let mut slots: Vec<SlotScript> = (0..4).map(|_| js_slot(0)).collect();
    for slot in &mut slots {
        start_js(slot, src, LoadShape::NativeTick, None);
    }
    for slot in &mut slots {
        wait_state(slot, RunState::Running);
        surface_tick(slot, 1);
        assert_eq!(probe(slot, "globalThis.__n"), serde_json::json!(1));
        slot.stop();
        wait_state(slot, RunState::Idle);
    }
}

fn start_sherlock(incarnation: u64, data: Arc<api::game_data::SelectedGameData>) -> SlotScript {
    script_js::install_cards().expect("install canonical JavaScript card slice");
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_compiled(
        "account",
        script::CompiledId("Sherlock"),
        Arc::new(script::native::SettingsBag::new()),
        Arc::clone(&data),
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    )
    .expect("start registered Sherlock card");
    wait_state(&mut slot, RunState::Running);
    slot
}

#[test]
fn the_pump_freezes_and_aborts_the_compiled_clue_machine() {
    let data = api::game_data::for_revision(ClientRevision::R274).expect("selected data");
    let held_id = data
        .trails()
        .expect("trails")
        .rows
        .iter()
        .find(|row| row.role == "clue")
        .expect("a selected clue row")
        .id;
    let mut slot = start_sherlock(102_419, Arc::clone(&data));
    let begin = script_js::clue::dispatch(
        Some(&data),
        &serde_json::json!({ "op": "begin", "generation": 0, "held": [[held_id, 1]] }),
    );
    assert_eq!(begin["kind"], "token", "{begin}");
    let token = begin["token"].as_u64().expect("token");
    let next = || {
        serde_json::json!({
            "op": "next",
            "token": token,
            "generation": 0,
            "held": [[held_id, 1]],
        })
    };

    slot.sync_compiled_clue(false);
    assert_eq!(
        script_js::clue::dispatch(Some(&data), &next())["kind"],
        "callback.enabled"
    );

    slot.pause();
    slot.sync_compiled_clue(false);
    assert_eq!(
        script_js::clue::dispatch(Some(&data), &next())["kind"],
        "wait"
    );
    slot.resume();
    slot.sync_compiled_clue(false);
    assert_eq!(
        script_js::clue::dispatch(Some(&data), &next())["kind"],
        "callback.enabled",
        "a thaw resumes the same session"
    );

    slot.sync_compiled_clue(true);
    assert_eq!(
        script_js::clue::dispatch(Some(&data), &next())["kind"],
        "wait"
    );
    slot.sync_compiled_clue(false);

    slot.stop();
    slot.sync_compiled_clue(false);
    let after = script_js::clue::dispatch(Some(&data), &next());
    assert_eq!(after["kind"], "aborted", "{after}");
    let again = script_js::clue::dispatch(
        Some(&data),
        &serde_json::json!({ "op": "begin", "generation": 0, "held": [[held_id, 1]] }),
    );
    assert_eq!(again["kind"], "token", "{again}");
    assert_ne!(
        again["token"].as_u64(),
        Some(token),
        "the aborted session's token is not reused"
    );
}

#[test]
fn a_session_reset_keeps_the_clue_strip_list_and_a_stop_clears_it() {
    let data = api::game_data::for_revision(ClientRevision::R274).expect("selected data");
    const ENTRANA: i32 = 3579;
    const HELM: i32 = 1163;
    let mut slot = start_sherlock(102_420, Arc::clone(&data));
    let begin = script_js::clue::dispatch(
        Some(&data),
        &serde_json::json!({ "op": "begin", "generation": 0, "held": [[ENTRANA, 1]] }),
    );
    assert_eq!(begin["kind"], "token", "{begin}");
    let token = begin["token"].as_u64().expect("token");
    let next = |extra: serde_json::Value| {
        let mut call = serde_json::json!({
            "op": "next",
            "token": token,
            "generation": 0,
            "held": [[ENTRANA, 1]],
        });
        for (key, value) in extra.as_object().expect("extra") {
            call[key] = value.clone();
        }
        script_js::clue::dispatch(Some(&data), &call)
    };
    assert_eq!(next(serde_json::json!({}))["kind"], "callback.enabled");
    assert_eq!(
        next(serde_json::json!({ "resume": true }))["kind"],
        "callback.log"
    );
    assert_eq!(next(serde_json::json!({}))["kind"], "callback.setStatus");
    let strip = next(serde_json::json!({
        "equipment": [{ "id": HELM, "name": "Rune full helm", "count": 1, "slot": 0 }],
    }));
    assert_eq!(strip["kind"], "unequip", "{strip}");
    let owns = |data: &Arc<api::game_data::SelectedGameData>| {
        script_js::clue::dispatch(Some(data), &serde_json::json!({ "op": "ownsEquipment" }))["owns"]
            == true
    };
    assert!(owns(&data), "the strip listed the name");

    slot.reset_session_work();
    slot.sync_compiled_clue(false);
    let dead = script_js::clue::dispatch(
        Some(&data),
        &serde_json::json!({
            "op": "next",
            "token": token,
            "generation": 0,
            "held": [[ENTRANA, 1]],
        }),
    );
    assert_eq!(dead["kind"], "aborted", "the boundary kills the step");
    assert!(
        owns(&data),
        "and keeps the list the reclaim still owes after a relog"
    );

    slot.stop();
    slot.sync_compiled_clue(false);
    assert!(!owns(&data), "Stop clears the strip list with the step");
}

const HELD_WALK: &str = r#"
import { Traversal } from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        globalThis.__ok = null;
        globalThis.__ok = await Traversal.walkResilient(
            { x: 2820, z: 3556, level: 0 },
            { radius: 1, timeoutMs: 300000 },
        );
    }
}
"#;

fn held_walk_tick(slot: &mut SlotScript, tick: u64, session_epoch: u64) {
    let key = slot.slot_key_for_test().expect("bound traversal slot");
    slot.align_seat_session(Some(SessionKey::new(key, session_epoch)));
    let here = (2823, 3555, 0);
    let mut input = empty_snapshot(tick);
    input.here = Some(TileInput {
        x: here.0,
        z: here.1,
        level: here.2,
    });
    let _ = slot.post_snapshot_delta(&input, NativeFactsInput::default(), false);
    slot_tick(slot, tick, Some(here));
    let _ = slot.drain_host_interacts();
}

#[test]
fn js_traversal_walk_remains_parked_across_reconnect() {
    let data = api::game_data::for_revision(ClientRevision::R289).expect("selected data");
    let mut slot = js_slot(1204);
    start_js_running(
        &mut slot,
        HELD_WALK,
        LoadShape::CompatClass,
        Some(Arc::clone(&data)),
    );
    arm_test_seat(&mut slot, 0);
    held_walk_tick(&mut slot, 1, 0);

    let _ = slot.reconnect_session_work();
    slot.on_is_up(true);
    held_walk_tick(&mut slot, 2, 1);
    assert_eq!(
        probe(&mut slot, "globalThis.__ok"),
        serde_json::Value::Null,
        "the Traversal promise remains pending across reconnect"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

fn run_surface_walk_tick(slot: &mut SlotScript, tick: u64, session_epoch: u64) {
    let key = slot.slot_key_for_test().expect("bound traversal slot");
    slot.align_seat_session(Some(SessionKey::new(key, session_epoch)));
    let here = (2823, 3555, 0);
    let mut input = empty_snapshot(tick);
    input.here = Some(TileInput {
        x: here.0,
        z: here.1,
        level: here.2,
    });
    let _ = slot.post_snapshot_delta(&input, NativeFactsInput::default(), false);
    surface_tick(slot, tick);
}

fn walk_near_rows(
    batches: &[script::native::seat_rows::QueuedSeatBatch],
) -> Vec<script::interact::InteractReq> {
    batches
        .iter()
        .flat_map(|queued| queued.batch.reqs.iter())
        .filter(|row| {
            matches!(
                &row.request,
                script::interact::InteractReq::WalkNear {
                    x: 2820,
                    z: 3556,
                    ..
                }
            )
        })
        .map(|row| row.request.clone())
        .collect()
}

fn drain_surface_seat_batches(
    slot: &mut SlotScript,
) -> Vec<script::native::seat_rows::QueuedSeatBatch> {
    slot.with_surface_run_for_test(|run| run.drain_seat_batches())
        .expect("active JavaScript surface")
}

fn consume_surface_batches(
    slot: &mut SlotScript,
    batches: Vec<script::native::seat_rows::QueuedSeatBatch>,
) {
    for queued in batches {
        slot.consume_seat_batch_for_test(
            queued.batch,
            queued.observed_walk_outcome_seq,
            &queued.machine_events,
        );
    }
}

#[test]
fn js_traversal_emits_walk_batch_for_slot_custody() {
    let data = api::game_data::for_revision(ClientRevision::R289).expect("selected data");
    let mut slot = js_slot(1207);
    start_js_running(
        &mut slot,
        HELD_WALK,
        LoadShape::CompatClass,
        Some(Arc::clone(&data)),
    );
    arm_test_seat(&mut slot, 0);

    run_surface_walk_tick(&mut slot, 1, 0);
    let batches = drain_surface_seat_batches(&mut slot);
    assert!(
        !walk_near_rows(&batches).is_empty(),
        "the first JavaScript tick emits the WalkNear batch: {batches:?}"
    );
    consume_surface_batches(&mut slot, batches);

    let _ = slot.reconnect_session_work();
    slot.on_is_up(true);
    let mut fresh = Vec::new();
    for tick in 2..=4 {
        run_surface_walk_tick(&mut slot, tick, 1);
        let batches = drain_surface_seat_batches(&mut slot);
        fresh.extend(walk_near_rows(&batches));
        consume_surface_batches(&mut slot, batches);
    }
    assert_eq!(
        fresh.len(),
        1,
        "the resumed wait emits exactly one fresh WalkNear batch: {fresh:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_stop_ends_traversal_held_script_after_reconnect() {
    let data = api::game_data::for_revision(ClientRevision::R289).expect("selected data");
    let mut slot = js_slot(1205);
    start_js_running(
        &mut slot,
        HELD_WALK,
        LoadShape::CompatClass,
        Some(Arc::clone(&data)),
    );
    arm_test_seat(&mut slot, 0);
    held_walk_tick(&mut slot, 1, 0);
    slot.reconnect_session_work();
    slot.on_is_up(true);
    held_walk_tick(&mut slot, 2, 1);
    assert_eq!(
        probe(&mut slot, "globalThis.__ok"),
        serde_json::Value::Null,
        "the Traversal walk remains pending after relog"
    );

    slot.reconnect_session_work();
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn js_ignored_randoms_publishes_maze_and_mime() {
    let source = r#"
export default class T extends LoopingBot {
    loop() { globalThis.__ticks = (globalThis.__ticks || 0) + 1; }
    ignoredRandoms() { return ["Maze", "Mime"]; }
}
"#;
    let mut slot = js_slot(1);
    let data = api::game_data::for_revision(ClientRevision::R289).expect("selected data");
    start_js_running(
        &mut slot,
        source,
        LoadShape::CompatClass,
        Some(Arc::clone(&data)),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut tick = 1;
    while !["Maze", "Mime"]
        .iter()
        .all(|name| slot.ignored_randoms().iter().any(|ignored| ignored == name))
        && Instant::now() < deadline
    {
        slot_tick(&mut slot, tick, None);
        tick += 1;
        let _ = slot.drain_host_interacts();
        std::thread::yield_now();
    }
    assert_eq!(slot.ignored_randoms(), ["Maze", "Mime"]);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}
