use super::*;
use crate::ctx::test_support::NullDriver;

struct Noop;

impl Script for Noop {
    fn name(&self) -> &str {
        "noop"
    }
    fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
}

#[cfg(feature = "load")]
fn wait_state(slot: &mut SlotScript, want: RunState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while slot.state() != want && Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(slot.state(), want, "last_error={:?}", slot.last_error());
}

#[cfg(feature = "load")]
fn allow_onstop_completion(slot: &SlotScript) {
    slot.load
        .as_ref()
        .expect("load isolate")
        .set_onstop_timeout_for_test(Duration::from_secs(1));
}

#[cfg(feature = "load")]
#[test]
fn active_tick_error_clears_on_success_not_user_log_text() {
    let source = r#"
export function tick() {
    const n = globalThis.__ticks || 0;
    globalThis.__ticks = n + 1;
    if (n === 0) throw new Error('first tick failure');
    globalThis.__rs2b0t_host.log = ['tick completed normally'];
}
"#;
    let mut slot = SlotScript::new();
    slot.start_load_with_loadouts(source.into(), LoadShape::NativeTick, vec![], &[])
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while slot.state() != RunState::Running {
        slot.observe_lifecycle();
        assert!(
            Instant::now() < deadline,
            "load isolate did not become ready: {:?}",
            slot.last_error()
        );
        std::thread::yield_now();
    }

    slot.load.as_ref().expect("load isolate").on_game_tick(1);
    assert_eq!(
        slot.load
            .as_ref()
            .expect("load isolate")
            .probe("globalThis.__ticks")
            .unwrap(),
        serde_json::json!(1)
    );
    let first_logs = slot.drain_logs();
    assert!(
        first_logs
            .iter()
            .any(|line| line.contains("first tick failure")),
        "{first_logs:?}"
    );
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("first tick failure")),
        "{:?}",
        slot.last_error()
    );

    slot.load.as_ref().expect("load isolate").on_game_tick(2);
    assert_eq!(
        slot.load
            .as_ref()
            .expect("load isolate")
            .probe("globalThis.__ticks")
            .unwrap(),
        serde_json::json!(2)
    );
    let second_logs = slot.drain_logs();
    assert!(
        second_logs
            .iter()
            .any(|line| line == "tick completed normally"),
        "{second_logs:?}"
    );
    assert_eq!(
        slot.last_error(),
        None,
        "a successful same-generation tick clears the active error; a user log does not replace it"
    );
    assert!(
        slot.take_pending_logs()
            .iter()
            .any(|line| line.contains("first tick failure")),
        "the recovered error remains in the slot log"
    );
    slot.stop();
}

#[test]
fn run_manager_override_is_session_scoped_and_last_call_replaces_snapshot() {
    let source = r#"
import { RunManager } from '../../runtime/RunManager.js';
export default class T extends LoopingBot {
    loop() {
        RunManager.override({ runAuto: false, energyMin: 80 });
        RunManager.override({ energyMin: 55 });
    }
}
"#;
    let mut slot = SlotScript::new();
    slot.start_load(source.into(), LoadShape::CompatClass, vec![])
        .unwrap();
    wait_state(&mut slot, RunState::Running);
    slot.load.as_ref().unwrap().on_game_tick(1);
    slot.probe("1").expect("tick settles before probe");
    assert_eq!(
        slot.run_policy_override(),
        Some(api::run_policy::RunPolicyOverride {
            run_auto: None,
            energy_min: Some(api::run_policy::RunEnergyMin::Floor(55)),
        }),
        "the second call replaces the whole snapshot"
    );

    slot.stop();
    assert_eq!(
        slot.run_policy_override(),
        None,
        "Stop returns auto-run to the host defaults"
    );
    wait_state(&mut slot, RunState::Idle);

    slot.run_policy_override
        .set(Some(api::run_policy::RunPolicyOverride {
            run_auto: Some(false),
            energy_min: Some(api::run_policy::RunEnergyMin::Floor(99)),
        }));
    slot.start_load(
        "export default class T extends LoopingBot { loop() {} }".into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    assert_eq!(
        slot.run_policy_override(),
        None,
        "the next Start clears a leftover harness or previous-run override"
    );
    slot.stop();
}
#[cfg(feature = "load")]
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
    let mut slot = SlotScript::new();
    slot.start_load(source.into(), LoadShape::CompatClass, vec![])
        .unwrap();
    wait_state(&mut slot, RunState::Running);

    use api::run_policy::{RunEnergyMin, RunPolicyOverride};
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
        slot.load.as_ref().unwrap().on_game_tick(index as u64 + 1);
        slot.probe("true")
            .expect("override tick settles before probe");
        assert_eq!(slot.run_policy_override(), expected, "case {index}");
        if index == 0 {
            assert_eq!(
                slot.probe("__override_return === undefined").unwrap(),
                serde_json::json!(true),
                "RunManager.override returns the frozen undefined value"
            );
        }
    }
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[cfg(feature = "load")]
#[test]
fn on_stop_override_is_cleared_before_idle_or_queued_start() {
    let old_source = r#"
import { RunManager } from '../../runtime/RunManager.js';
export default class Old extends LoopingBot {
    loop() {}
    onStop() {
        RunManager.override({ runAuto: false });
        this.log('onstop-policy-set');
    }
}
"#;
    let mut slot = SlotScript::new();
    let cell = slot.run_policy_override_cell();
    slot.start_load(old_source.into(), LoadShape::CompatClass, vec![])
        .unwrap();
    wait_state(&mut slot, RunState::Running);
    allow_onstop_completion(&slot);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    assert!(
        slot.take_pending_logs()
            .iter()
            .any(|line| line.contains("onstop-policy-set")),
        "the Stop teardown hook actually writes the overlay"
    );
    assert_eq!(cell.get(), None, "Stop → Idle clears the onStop write");

    slot.start_load(old_source.into(), LoadShape::CompatClass, vec![])
        .unwrap();
    wait_state(&mut slot, RunState::Running);
    allow_onstop_completion(&slot);
    slot.stop();
    slot.start_load(
        "export default class New extends LoopingBot { loop() {} }".into(),
        LoadShape::CompatClass,
        vec![],
    )
    .expect("Start queues behind the old isolate reap");
    wait_state(&mut slot, RunState::Running);
    assert_eq!(
        cell.get(),
        None,
        "Stop → immediate Start does not carry the previous onStop write"
    );
    assert!(
        slot.take_pending_logs()
            .iter()
            .any(|line| line.contains("onstop-policy-set")),
        "the old isolate's onStop ran before the new run reached Running"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[cfg(feature = "load")]
#[test]
fn script_requested_stop_cleans_slot_work_and_allows_fresh_restart() {
    let source = r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() {
        (globalThis.__rs2b0t_host.interact ||= []).push({op: 'set-camera-yaw', yaw: 123});
        ScriptRunner.stop('finished');
    }
}
"#;
    let mut slot = SlotScript::new();
    slot.start_load_with_loadouts(source.to_string(), LoadShape::CompatClass, vec![], &[])
        .unwrap();
    let input = crate::isolate_fb::tests::empty_input(1);
    slot.encode_snapshot_delta(&input, false);
    slot.store_last_world_id(Some(123));
    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
    let epoch = slot.work_epoch();
    slot.load.as_ref().unwrap().on_game_tick(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut logs = Vec::new();
    while matches!(
        slot.state(),
        RunState::Starting | RunState::Running | RunState::Stopping
    ) && Instant::now() < deadline
    {
        logs.extend(slot.drain_logs());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(slot.state(), RunState::Idle, "{logs:?}");
    assert!(logs
        .iter()
        .any(|line| line.contains("script requested stop")));
    assert!(!slot.has_instance());
    assert!(slot.pending_withdraw_x().is_none());
    assert_ne!(slot.work_epoch(), epoch);
    assert!(!slot.has_snapshot_fingerprint());
    assert_eq!(slot.last_world_id(), None);
    assert!(slot.drain_interacts().is_empty());
    assert!(slot.last_error().unwrap().contains("script requested stop"));
    assert_eq!(
        slot.lifecycle_receipt(),
        Some(ScriptLifecycleReceipt {
            runtime_generation: 1,
            state: ScriptTerminalState::Stopped,
            tick: 1,
            reason: "finished".into(),
        })
    );
    assert_eq!(slot.take_pending_logs(), logs);
    slot.on_is_up(true);
    assert_eq!(
        slot.state(),
        RunState::Idle,
        "login cannot restart a stopped card"
    );
    slot.start_load_with_loadouts(
        "export default class T extends LoopingBot { loop() { this.n = (this.n || 0) + 1; } }"
            .into(),
        LoadShape::CompatClass,
        vec![],
        &[],
    )
    .unwrap();
    slot.load.as_ref().unwrap().on_game_tick(2);
    wait_state(&mut slot, RunState::Running);
    assert_eq!(slot.state(), RunState::Running);
    assert!(slot.last_error().is_none());
    assert_eq!(
        slot.lifecycle_receipt(),
        None,
        "fresh Start clears the receipt"
    );
    slot.stop();
    assert_eq!(
        slot.lifecycle_receipt(),
        None,
        "operator Stop is not a script-requested Stopped receipt"
    );
}

#[cfg(feature = "load")]
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
    let mut slot = SlotScript::new();
    slot.start_load_with_loadouts(source, LoadShape::CompatClass, vec![], &[])
        .unwrap();
    let input = crate::isolate_fb::tests::empty_input(1);
    slot.encode_snapshot_delta(&input, false);
    slot.load.as_ref().unwrap().on_game_tick(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut logs = Vec::new();
    while matches!(
        slot.state(),
        RunState::Starting | RunState::Running | RunState::Stopping
    ) && Instant::now() < deadline
    {
        logs.extend(slot.drain_logs());
        std::thread::sleep(Duration::from_millis(5));
    }
    let receipt = slot.lifecycle_receipt().unwrap_or_else(|| {
        panic!(
            "script Stop receipt; state={:?} logs={logs:?}",
            slot.state()
        )
    });
    assert_eq!(receipt.state, ScriptTerminalState::Stopped);
    assert!(receipt.reason.len() <= 256);
    assert!(receipt.reason.chars().all(|ch| ch == '🙂'));
}

#[cfg(feature = "load")]
#[test]
fn stop_releases_snapshot_storage_and_restart_emits_keyframe() {
    let mut slot = SlotScript::new();
    slot.start_compiled(Box::new(Noop), None).unwrap();
    let text = "x".repeat(1024 * 1024);
    let mut input = crate::isolate_fb::tests::empty_input(1);
    input.chat_text = Some(&text);
    let first = slot.encode_snapshot_delta(&input, false);
    assert!(first.len() > text.len());
    slot.store_last_world_id(Some(123));
    slot.last_error = Some("retained diagnostic".into());
    slot.pending_logs.push("retained log".into());
    slot.pause();
    assert!(slot.has_snapshot_fingerprint());
    assert_eq!(slot.last_world_id(), Some(123));
    slot.resume();
    let delta = slot.encode_snapshot_delta(&input, false);
    assert!(delta.len() < 1024);
    slot.stop();
    assert!(!slot.has_snapshot_fingerprint());
    assert_eq!(slot.last_world_id(), None);
    assert_eq!(std::mem::take(&mut slot.ipc).into_backing_capacity(), 0);
    assert_eq!(slot.last_error.as_deref(), Some("retained diagnostic"));
    assert_eq!(slot.pending_logs, ["retained log"]);
    slot.start_compiled(Box::new(Noop), None).unwrap();
    assert_eq!(slot.encode_snapshot_delta(&input, false), first);
    // The earlier owned packet remains intact after reuse and Stop.
    assert!(crate::isolate_fb::SnapshotReader::from_bytes(&first).is_ok());
}

/// F14 M12: a post the wedged isolate refused never reached it, so the
/// delta base must not advance: the next encode is a keyframe.
#[cfg(feature = "load")]
#[test]
fn refused_snapshot_post_forces_a_keyframe() {
    let mut slot = SlotScript::new();
    slot.start_load_with_loadouts(
        "export function tick(api) { const t = Date.now(); while (Date.now() - t < 400) {} }"
            .into(),
        LoadShape::NativeTick,
        vec![],
        &[],
    )
    .unwrap();
    let text = "x".repeat(64 * 1024);
    let mut input = crate::isolate_fb::tests::empty_input(1);
    input.chat_text = Some(&text);
    let keyframe = slot.encode_snapshot_delta(&input, false);
    assert!(keyframe.len() > text.len());
    assert!(slot.post_snapshot(keyframe));
    slot.load.as_ref().unwrap().on_game_tick(1);
    let mut refused = false;
    for _ in 0..200 {
        let delta = slot.encode_snapshot_delta(&input, false);
        assert!(delta.len() < 1024, "unchanged fields stay out of a delta");
        if !slot.post_snapshot(delta) {
            refused = true;
            break;
        }
    }
    assert!(refused, "the busy isolate's backlog is bounded");
    assert!(
        slot.encode_snapshot_delta(&input, false).len() > text.len(),
        "the post after a refusal carries every field again"
    );
    slot.stop();
}

#[test]
fn on_random_defaults_to_host_and_override_claims_handle() {
    use api::random::{DetectedRandom, RandomClaim, RandomKind};

    struct ClaimHandle;
    impl Script for ClaimHandle {
        fn name(&self) -> &str {
            "claim-handle"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
        fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
            RandomClaim::Handle
        }
    }

    let ev = DetectedRandom {
        kind: RandomKind::Dialog,
        name: "genie".to_string(),
        ours: true,
        npc_index: Some(0),
    };

    // Default: Host.
    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Noop), None).unwrap();
    assert_eq!(s.on_random(&ev), RandomClaim::Host);

    // Override: Handle.
    s.stop();
    s.start_compiled(Box::new(ClaimHandle), None).unwrap();
    assert_eq!(s.on_random(&ev), RandomClaim::Handle);

    // Paused: Host — the knock only fires while Running.
    s.pause();
    assert_eq!(s.on_random(&ev), RandomClaim::Host);

    // Idle (stopped): Host.
    s.stop();
    assert_eq!(s.on_random(&ev), RandomClaim::Host);
}

#[test]
fn ticks_counts_dispatched_ticks_since_start() {
    let mut s = SlotScript::new();
    s.start_compiled(Box::new(Noop), None).unwrap();
    let mut d = NullDriver::default();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 2,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    assert_eq!(s.ticks, 2);

    // Paused ticks do not count.
    s.pause();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 3,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    assert_eq!(s.ticks, 2);

    // A fresh Start resets the counter.
    s.stop();
    s.start_compiled(Box::new(Noop), None).unwrap();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 4,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    assert_eq!(s.ticks, 1);
}

#[test]
fn pending_withdraw_x_pause_freezes_while_stop_and_reconnect_abort() {
    let mut slot = SlotScript::new();
    slot.start_compiled(Box::new(Noop), None).unwrap();
    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
    assert_eq!(
        slot.pending_withdraw_x().unwrap().remaining,
        Duration::from_millis(3000)
    );

    slot.pause();
    let paused = slot
        .pending_withdraw_x()
        .expect("Pause retains pending work");
    assert!(paused.deadline.is_none(), "Pause freezes monotonic time");
    slot.resume();
    assert!(
        slot.pending_withdraw_x().unwrap().deadline.is_some(),
        "Resume restores the remaining deadline"
    );

    slot.stop();
    assert!(
        slot.pending_withdraw_x().is_none(),
        "Stop aborts pending work"
    );

    slot.start_compiled(Box::new(Noop), None).unwrap();
    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
    let before_reset = slot.withdraw_x_result();
    slot.reset_session_work();
    assert!(
        slot.pending_withdraw_x().is_none(),
        "reconnect/session reset aborts pending work"
    );
    assert_eq!(
        slot.withdraw_x_result(),
        (before_reset.0.wrapping_add(1), false),
        "session reset publishes an explicit abort even if a later bank reuses the generation"
    );

    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_load_dialog(
        2, 20, 1, 0, 20, 3,
    )));
    let before_load_reset = slot.withdraw_load_result();
    slot.reset_session_work();
    assert_eq!(
        slot.withdraw_load_result(),
        (before_load_reset.0.wrapping_add(1), false),
        "withdrawLoad gets the same explicit abort on generation-reusing session reset"
    );

    let settlement = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3).waiting_settlement();
    assert_eq!(settlement.remaining, Duration::from_millis(4000));

    let mut expired = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3);
    expired.deadline = Some(Instant::now() - Duration::from_millis(1));
    assert!(expired.expired(), "expiry uses monotonic wall time");
}

#[test]
fn pending_bank_op_pause_freezes_while_stop_and_reconnect_abort() {
    let mut slot = SlotScript::new();
    slot.start_compiled(Box::new(Noop), None).unwrap();
    slot.set_pending_bank_op(Some(PendingBankOp::new(
        PendingBankOpKind::Deposit,
        1,
        3,
        0,
        7,
    )));
    assert_eq!(
        slot.pending_bank_op().unwrap().remaining,
        Duration::from_millis(2000)
    );

    slot.pause();
    assert!(
        slot.pending_bank_op().unwrap().deadline.is_none(),
        "Pause freezes the ordinary bank deadline"
    );
    slot.resume();
    assert!(slot.pending_bank_op().unwrap().deadline.is_some());

    slot.stop();
    assert!(slot.pending_bank_op().is_none(), "Stop drops old-slot work");

    slot.start_compiled(Box::new(Noop), None).unwrap();
    slot.set_pending_bank_op(Some(PendingBankOp::new(
        PendingBankOpKind::Withdraw,
        1,
        20,
        0,
        7,
    )));
    assert_eq!(
        slot.pending_bank_op().unwrap().remaining,
        Duration::from_millis(4000)
    );
    let before = slot.bank_op_result();
    slot.reset_session_work();
    assert!(slot.pending_bank_op().is_none());
    assert_eq!(slot.bank_op_result(), (before.0.wrapping_add(1), false));
}

#[cfg(feature = "load")]
#[test]
fn fenced_settings_reject_stale_identity_generation_and_unchanged_bag() {
    let mut slot = SlotScript::new();
    slot.start_compiled(Box::new(Noop), None).unwrap();
    slot.attach_source_identity("catalog:ChickenKiller");
    let gen = slot.runtime_generation();
    let mut bag = serde_json::Map::new();
    bag.insert("x".into(), serde_json::json!(1));
    assert!(slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen));
    assert!(
        !slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen),
        "unchanged bag is not reposted"
    );
    assert!(!slot.post_settings_bag_fenced(&bag, "catalog:Other", gen));
    assert!(!slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen.wrapping_add(1)));
    slot.stop();
    assert!(slot.source_identity().is_none());
    assert_ne!(slot.runtime_generation(), gen);
    assert!(!slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen));
}

#[cfg(feature = "load")]
#[test]
fn settings_update_reaches_start_queued_behind_reap() {
    let mut slot = SlotScript::new();
    let source = "export function tick() {}".to_string();
    slot.start_load_with_loadouts(source.clone(), LoadShape::NativeTick, vec![], &[])
        .unwrap();
    slot.stop();
    slot.start_load_with_loadouts(source, LoadShape::NativeTick, vec![], &[])
        .unwrap();
    slot.attach_source_identity("catalog:Example");
    let generation = slot.runtime_generation();
    let bag = serde_json::Map::from_iter([("amount".to_string(), serde_json::json!(9))]);
    assert!(slot.post_settings_bag_fenced(&bag, "catalog:Example", generation));
    assert_eq!(
        slot.load_identity
            .as_ref()
            .and_then(|identity| identity.settings_bag.as_deref()),
        Some(&bag)
    );
}

#[test]
fn stop_clears_identity_and_bumps_runtime_generation() {
    let mut slot = SlotScript::new();
    slot.start_compiled(Box::new(Noop), None).unwrap();
    slot.attach_source_identity("file:shared.ts");
    let gen = slot.runtime_generation();
    slot.stop();
    assert!(slot.source_identity().is_none());
    assert_eq!(slot.state(), RunState::Idle);
    assert_ne!(
        slot.runtime_generation(),
        gen,
        "Stop must invalidate the previous execution generation"
    );
}

#[cfg(feature = "load")]
#[test]
fn slot_stop_delivers_final_logs_exactly_once() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }"
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    slot.load.as_ref().unwrap().on_game_tick(1);
    let _ = slot.probe("1");
    allow_onstop_completion(&slot);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    let logs = slot.take_pending_logs();
    let hits = logs.iter().filter(|l| l.contains("stopped-ok")).count();
    assert_eq!(hits, 1, "exactly one onStop log after take: {logs:?}");
    slot.stop();
    let again = slot.take_pending_logs();
    assert!(
        again.iter().all(|l| !l.contains("stopped-ok")),
        "second stop must not rerun the hook: {again:?}"
    );
}

#[cfg(feature = "load")]
#[test]
fn self_stop_runs_hook_once_then_slot_stop_does_not() {
    let mut slot = SlotScript::new();
    slot.start_load(
        r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() { ScriptRunner.stop('done'); }
    onStop() { this.log('stopped-ok'); }
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    allow_onstop_completion(&slot);
    slot.load.as_ref().unwrap().on_game_tick(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(
        slot.state(),
        RunState::Starting | RunState::Running | RunState::Stopping
    ) && Instant::now() < deadline
    {
        let _ = slot.drain_logs();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(slot.state(), RunState::Idle);
    let logs = slot.take_pending_logs();
    let hits = logs.iter().filter(|l| l.contains("stopped-ok")).count();
    assert_eq!(hits, 1, "self-stop onStop once: {logs:?}");
    assert!(logs.iter().any(|l| l.contains("script requested stop")));
    slot.stop();
    let again = slot.take_pending_logs();
    assert!(
        again.iter().all(|l| !l.contains("stopped-ok")),
        "join after self-stop then Drop must not rerun: {again:?}"
    );
}

#[cfg(feature = "load")]
#[test]
fn watchdog_restart_folds_onstop_logs_into_pending() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export default class T extends LoopingBot {
            onStart() { globalThis.__gen = (globalThis.__gen || 0) + 1; }
            loop() { globalThis.__n = (globalThis.__n || 0) + 1; }
            onStop() { this.log('stopped-ok'); }
        }"
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    slot.load.as_ref().unwrap().on_game_tick(1);
    let _ = slot.probe("1");
    allow_onstop_completion(&slot);
    slot.restart_load_from_identity(Instant::now())
        .expect("restart from identity");
    wait_state(&mut slot, RunState::Starting);
    wait_state(&mut slot, RunState::Running);
    let logs = slot.take_pending_logs();
    assert!(
        logs.iter().any(|l| l.contains("stopped-ok")),
        "dying isolate onStop must land in pending_logs: {logs:?}"
    );
    slot.load.as_ref().unwrap().on_game_tick(1);
    assert_eq!(slot.probe("__gen").unwrap(), 1, "new isolate onStart runs");
    slot.stop();
}

#[cfg(feature = "load")]
#[test]
fn restart_still_refuses_pause() {
    let mut slot = SlotScript::new();
    slot.start_load(
            "export default class T extends LoopingBot { loop() {} onStop() { this.log('stopped-ok'); } }"
                .into(),
            LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
    slot.pause();
    let err = slot.restart_load_from_identity(Instant::now()).unwrap_err();
    assert!(
        err.contains("not running") || err.contains("pause") || err.contains("frozen"),
        "{err}"
    );
    slot.stop();
}

#[cfg(feature = "load")]
#[test]
fn start_load_returns_before_setup_and_becomes_running() {
    let mut slot = SlotScript::new();
    let t0 = Instant::now();
    slot.start_load(
        "export function tick(api) { globalThis.__n = (globalThis.__n || 0) + 1; }".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "Start must not wait on V8: {:?}",
        t0.elapsed()
    );
    assert_eq!(slot.state(), RunState::Starting);
    wait_state(&mut slot, RunState::Running);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[cfg(feature = "load")]
#[test]
fn start_load_setup_failure_returns_to_idle_with_the_diagnostic() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "throw new Error('zz-setup-proof');\nexport function tick(api) {}".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .expect("Start returns before setup");
    slot.attach_source_identity("file:bad.ts");
    assert_eq!(
        slot.poll_start(),
        StartPoll::Pending,
        "setup has not settled"
    );
    wait_state(&mut slot, RunState::Idle);
    let err = slot.last_error().unwrap_or("").to_string();
    assert!(err.contains("zz-setup-proof"), "{err}");
    assert_eq!(
        slot.poll_start(),
        StartPoll::Settled(StartOutcome::Failed(err))
    );
    assert_eq!(slot.source_identity(), None);
    assert!(slot.load_identity.is_none());
    assert_eq!(
        slot.runtime_generation(),
        0,
        "a Start that never reached Ready is not a Start"
    );
}

#[cfg(feature = "load")]
#[test]
fn stop_during_starting_ends_idle_and_cancels_the_start() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export function tick(api) {}".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    assert_eq!(slot.state(), RunState::Starting);
    slot.stop();
    assert_eq!(slot.state(), RunState::Stopping);
    wait_state(&mut slot, RunState::Idle);
    assert_eq!(
        slot.poll_start(),
        StartPoll::Settled(StartOutcome::Cancelled)
    );
    assert_eq!(slot.last_error(), None);
}

#[cfg(feature = "load")]
#[test]
fn watchdog_restart_during_stop_reap_never_revives_the_slot() {
    let mut slot = SlotScript::new();
    slot.start_load(
        "export function tick(api) {}".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    wait_state(&mut slot, RunState::Running);
    slot.stop();
    assert_eq!(slot.state(), RunState::Stopping);
    assert!(slot.restart_load_from_identity(Instant::now()).is_err());
    wait_state(&mut slot, RunState::Idle);
    for _ in 0..20 {
        slot.observe_lifecycle();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(slot.state(), RunState::Idle);
    assert!(!slot.load_active());
}

#[cfg(feature = "load")]
#[test]
fn stop_returns_immediately_and_second_start_waits_for_reap() {
    let src = "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }";
    let mut slot = SlotScript::new();
    slot.start_load(src.into(), LoadShape::CompatClass, vec![])
        .unwrap();
    wait_state(&mut slot, RunState::Running);
    allow_onstop_completion(&slot);
    let t0 = Instant::now();
    slot.stop();
    assert!(
        t0.elapsed() < Duration::from_millis(200),
        "Stop blocked: {:?}",
        t0.elapsed()
    );
    assert_eq!(slot.state(), RunState::Stopping);
    slot.start_load(src.into(), LoadShape::CompatClass, vec![])
        .expect("Start while Stopping is queued");
    let generation = slot.runtime_generation();
    assert!(
        slot.start_load(src.into(), LoadShape::CompatClass, vec![])
            .is_err(),
        "a second queued Start is refused"
    );
    wait_state(&mut slot, RunState::Starting);
    wait_state(&mut slot, RunState::Running);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    assert_eq!(
        slot.runtime_generation(),
        generation,
        "the generation read after Start is the one that runs"
    );
    let logs = slot.take_pending_logs();
    assert!(
        logs.iter().any(|l| l.contains("stopped-ok")),
        "reaped onStop before the queued Start: {logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[cfg(feature = "load")]
#[test]
fn start_all_over_n_members_does_not_block_per_member() {
    let src = "export function tick(api) {}".to_string();
    let mut slots: Vec<SlotScript> = (0..4).map(|_| SlotScript::new()).collect();
    let t0 = Instant::now();
    for slot in &mut slots {
        slot.start_load(src.clone(), LoadShape::NativeTick, vec![])
            .unwrap();
        assert_eq!(slot.state(), RunState::Starting);
    }
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "Start all blocked per member: {:?}",
        t0.elapsed()
    );
    for slot in &mut slots {
        wait_state(slot, RunState::Running);
        slot.stop();
        wait_state(slot, RunState::Idle);
    }
}

/// A compiled card that queues one walk per tick onto the ctx's sink —
/// the queue the slot parks there — and nothing else.
#[cfg(feature = "load")]
#[derive(Default)]
struct Walker;

#[cfg(feature = "load")]
impl Script for Walker {
    fn name(&self) -> &str {
        "Walker"
    }

    fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        if let Some(sink) = ctx.compiled.interacts.as_mut() {
            sink.push(crate::shim::InteractReq::Walk {
                x: 3,
                z: 4,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
            });
        }
    }
}

/// The walk `Walker` queues, for comparing whole requests.
#[cfg(feature = "load")]
fn walker_walk() -> crate::shim::InteractReq {
    crate::shim::InteractReq::Walk {
        x: 3,
        z: 4,
        level: 0,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id: 0,
    }
}

/// A ctx with no views wired and nothing parked: a compiled tick over it
/// is the slot's own queue, installed by the slot.
#[cfg(feature = "load")]
fn compiled_ctx<'a>(
    driver: &'a mut dyn api::interact::Driver,
    selected: Option<&'a api::game_data::SelectedGameData>,
) -> ScriptCtx<'a> {
    ScriptCtx {
        driver,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::CompiledTick {
            selected,
            hold: false,
            interacts: None,
        },
    }
}

#[cfg(feature = "load")]
#[test]
fn a_compiled_tick_queues_onto_the_slot_and_one_drain_takes_it() {
    let mut slot = SlotScript::new();
    slot.start_compiled(Box::new(Walker), None).unwrap();
    let mut d = NullDriver::default();
    slot.on_game_tick(&mut compiled_ctx(&mut d, None));
    assert_eq!(
        slot.drain_interacts(),
        vec![walker_walk()],
        "the compiled card's verbs ride the slot's own drain"
    );
    assert!(
        slot.drain_interacts().is_empty(),
        "the drain takes the queue, it never replays it"
    );

    // A paused card is not ticked, so it queues nothing.
    slot.pause();
    slot.on_game_tick(&mut compiled_ctx(&mut d, None));
    assert!(slot.drain_interacts().is_empty());

    // Stop drops the instance and the queue it had not spent.
    slot.resume();
    slot.on_game_tick(&mut compiled_ctx(&mut d, None));
    slot.stop();
    assert!(
        slot.drain_interacts().is_empty(),
        "Stop must not leak a dead card's requests into the next Start"
    );
}

#[cfg(feature = "load")]
#[test]
fn a_compiled_start_pins_the_selected_facts_and_stop_clears_them() {
    let data =
        api::game_data::for_revision(client::io::ClientRevision::R274).expect("selected data");
    let mut slot = SlotScript::new();
    assert!(slot.compiled_game_data().is_none());
    slot.start_compiled(Box::new(Noop), Some(Arc::clone(&data)))
        .unwrap();
    assert!(
        slot.compiled_game_data().is_some(),
        "the Start pin rides out to the ctx"
    );
    slot.stop();
    assert!(
        slot.compiled_game_data().is_none(),
        "a stopped card keeps no pin"
    );
    slot.start_compiled(Box::new(Noop), None).unwrap();
    assert!(
        slot.compiled_game_data().is_none(),
        "a Start with no pin is what a compiled identify fails closed on"
    );
}

#[cfg(feature = "load")]
#[test]
fn the_pump_freezes_and_aborts_the_compiled_clue_machine() {
    let data =
        api::game_data::for_revision(client::io::ClientRevision::R274).expect("selected data");
    // The machine's own identify decides what is held: a selected
    // membership row with a positive count.
    let held_id = data
        .trails()
        .expect("trails")
        .rows
        .iter()
        .find(|row| row.role == "clue")
        .expect("a selected clue row")
        .id;
    let mut slot = SlotScript::new();
    slot.start_compiled(
        Box::new(crate::sherlock::Sherlock::default()),
        Some(Arc::clone(&data)),
    )
    .unwrap();
    let begin = crate::clue::dispatch(
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

    // Running and unfrozen: the machine posts its own gate question.
    slot.sync_compiled_clue(false);
    assert_eq!(
        crate::clue::dispatch(Some(&data), &next())["kind"],
        "callback.enabled"
    );

    // Operator Pause, on a frame that dispatches no tick at all: the live
    // token waits instead, so the pause cannot burn the session's clock.
    slot.pause();
    slot.sync_compiled_clue(false);
    assert_eq!(crate::clue::dispatch(Some(&data), &next())["kind"], "wait");
    slot.resume();
    slot.sync_compiled_clue(false);
    assert_eq!(
        crate::clue::dispatch(Some(&data), &next())["kind"],
        "callback.enabled",
        "a thaw resumes the same session"
    );

    // The guardian's hold freezes the same live session the same way.
    slot.sync_compiled_clue(true);
    assert_eq!(crate::clue::dispatch(Some(&data), &next())["kind"], "wait");
    slot.sync_compiled_clue(false);

    // Stop is marked wherever it ran and applied on this thread: the live
    // token is gone, and the next Start begins a fresh session.
    slot.stop();
    slot.sync_compiled_clue(false);
    let after = crate::clue::dispatch(Some(&data), &next());
    assert_eq!(after["kind"], "aborted", "{after}");
    let again = crate::clue::dispatch(
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

/// The two resets are not each other. A connection boundary
/// (`reset_session_work`) aborts the compiled clue machine's live step and
/// its token and keeps what the session still owes — the Entrana strip list
/// the reclaim reads — while operator Stop is a fresh task instance and
/// clears it with the step.
#[cfg(feature = "load")]
#[test]
fn a_session_reset_keeps_the_clue_strip_list_and_a_stop_clears_it() {
    let data =
        api::game_data::for_revision(client::io::ClientRevision::R274).expect("selected data");
    // The Entrana-box proof row, and a worn name the frozen matcher folds.
    const ENTRANA: i32 = 3579;
    const HELM: i32 = 1163;
    let mut slot = SlotScript::new();
    slot.start_compiled(
        Box::new(crate::sherlock::Sherlock::default()),
        Some(Arc::clone(&data)),
    )
    .unwrap();
    let begin = crate::clue::dispatch(
        Some(&data),
        &serde_json::json!({ "op": "begin", "generation": 0, "held": [[ENTRANA, 1]] }),
    );
    assert_eq!(begin["kind"], "token", "{begin}");
    let token = begin["token"].as_u64().expect("token");
    // The landed gate first: enabled, the report, the row's own status.
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
        crate::clue::dispatch(Some(&data), &call)
    };
    assert_eq!(next(serde_json::json!({}))["kind"], "callback.enabled");
    assert_eq!(
        next(serde_json::json!({ "resume": true }))["kind"],
        "callback.log"
    );
    assert_eq!(next(serde_json::json!({}))["kind"], "callback.setStatus");
    // One strip step: the worn restricted row goes off, and the name is
    // listed for the reclaim.
    let strip = next(serde_json::json!({
        "equipment": [{ "id": HELM, "name": "Rune full helm", "count": 1, "slot": 0 }],
    }));
    assert_eq!(strip["kind"], "unequip", "{strip}");
    let owns = |data: &Arc<api::game_data::SelectedGameData>| {
        crate::clue::dispatch(Some(data), &serde_json::json!({ "op": "ownsEquipment" }))["owns"]
            == true
    };
    assert!(owns(&data), "the strip listed the name");

    // The connection boundary, applied the way the pump applies it.
    slot.reset_session_work();
    slot.sync_compiled_clue(false);
    let dead = crate::clue::dispatch(
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

    // Operator Stop: the fresh instance starts the session over.
    slot.stop();
    slot.sync_compiled_clue(false);
    assert!(!owns(&data), "Stop clears the strip list with the step");
}
