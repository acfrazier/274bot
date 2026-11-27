#![cfg(feature = "scenario-luau")]

use std::sync::{mpsc, Arc};

use api::identity::ExecutionId;
use api::selected::RunKey;
use script::native::recovery_hints::RecoveryHintsCell;
use script::native::snapshot_source::{NativeFactsInput, ReachViewInput, SnapshotInput};
use script::surface::extension::{
    ExtensionConstant, ExtensionDock, ExtensionEntry, ExtensionFaultCode, ExtensionFunction,
    ExtensionMode, ExtensionNamespace, ExtensionPayload, ExtensionValue, RunBinding,
};
use script::surface::{
    LentOutcome, SurfaceFactory, SurfaceRun, SurfaceStart, SurfaceWake, TickOutcome,
};
use script_luau::{
    scenario_source, ExtensionRegistry, LuauScenarioExtension, LuauStartCard, LuauSurfaceFactory,
};

fn binding(run_key: u64) -> RunBinding {
    RunBinding::new(1, run_key, 17 + run_key)
}

fn extension(
    dock: Arc<ExtensionDock>,
    binding: RunBinding,
    functions: Vec<ExtensionFunction>,
) -> LuauScenarioExtension {
    let registry = ExtensionRegistry::new(
        functions,
        vec![ExtensionNamespace::new(
            "values",
            vec![ExtensionConstant::new("north", ExtensionValue::Integer(1))],
        )],
    )
    .expect("static extension registry is valid");
    LuauScenarioExtension::new(dock, binding, Arc::new(registry), "fixture-probe", "alice")
}

fn start(source: &str, extension: LuauScenarioExtension) -> SurfaceStart {
    let body = scenario_source(Arc::from(source))
        .expect("scenario test source has a valid header")
        .erased;
    SurfaceStart {
        card: Arc::new(LuauStartCard::new_scenario(
            "scenario-test",
            body,
            extension,
        )),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        wilderness_rules: script::surface::no_wilderness(),
        loadouts: Arc::default(),
    }
}

fn spawn(
    source: &str,
    extension: LuauScenarioExtension,
) -> Result<Box<dyn SurfaceRun>, script::surface::SurfaceError> {
    let start = start(source, extension);
    LuauSurfaceFactory.spawn(
        &start,
        ExecutionId::first(RunKey {
            slot: 1,
            run: 1,
            session: 1,
        }),
        Arc::new(RecoveryHintsCell::default()),
    )
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
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn dispatch(run: &mut dyn SurfaceRun, tick: u64, held: bool) -> LentOutcome {
    let input = empty_snapshot(tick);
    run.dispatch_lent(
        SurfaceWake::Tick { held },
        tick,
        tick,
        &input,
        NativeFactsInput::default(),
    )
}

fn function(name: &'static str, mode: ExtensionMode) -> ExtensionFunction {
    ExtensionFunction::new(name, mode)
}

#[test]
fn ordinary_start_has_no_scenario_global() {
    let start = SurfaceStart {
        card: Arc::new(LuauStartCard::new(
            "ordinary-test",
            "return { tick = function() assert(scenario == nil) end }",
        )),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        wilderness_rules: script::surface::no_wilderness(),
        loadouts: Arc::default(),
    };
    let mut run = LuauSurfaceFactory
        .spawn(
            &start,
            ExecutionId::first(RunKey {
                slot: 1,
                run: 1,
                session: 1,
            }),
            Arc::new(RecoveryHintsCell::default()),
        )
        .expect("ordinary card starts without the extension");
    assert_eq!(dispatch(&mut *run, 1, false), LentOutcome::Accepted);
}

#[test]
fn scenario_constants_are_read_only_and_await_resumes_once_with_boolean_payload() {
    let binding = binding(1);
    let dock = Arc::new(ExtensionDock::new());
    let ext = extension(
        Arc::clone(&dock),
        binding,
        vec![function("probe.answer", ExtensionMode::Await)],
    );
    let mut run = spawn(
        r#"
        scenario_header { name = "probe" }
        return { tick = function()
            assert(scenario.case == "fixture-probe")
            assert(scenario.role == "alice")
            assert(scenario.values.north == 1)
            assert(not pcall(function() scenario.case = "changed" end))
            local answer = scenario.probe.answer({ key = "value" })
            assert(answer == true)
            api.log("resumed")
        end }
        "#,
        ext,
    )
    .expect("scenario card starts");

    assert_eq!(dispatch(&mut *run, 1, false), LentOutcome::Accepted);
    let call = dock
        .take(binding)
        .expect("host reads dock")
        .expect("await call is staged");
    assert_eq!(call.name, "probe.answer");
    assert_eq!(call.mode, ExtensionMode::Await);
    assert_eq!(call.entry, ExtensionEntry::Tick);
    assert!(matches!(
        call.arguments,
        ExtensionValue::List(values) if matches!(values.as_slice(), [ExtensionValue::Named(_)])
    ));
    dock.complete(binding, call.sequence, ExtensionPayload::Boolean(true))
        .expect("host completes matching await");

    assert_eq!(dispatch(&mut *run, 2, true), LentOutcome::Accepted);
    assert!(
        run.drain_logs().is_empty(),
        "held tick does not resume the await"
    );
    assert_eq!(dispatch(&mut *run, 3, false), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), vec!["resumed".to_owned()]);
    assert!(run
        .drain_tick_outcomes()
        .iter()
        .all(|outcome| matches!(outcome, TickOutcome::Success { .. })));
}

#[test]
fn caught_unknown_function_refusal_latches_a_durable_binding_first_fault() {
    let binding = binding(1);
    let dock = Arc::new(ExtensionDock::new());
    let ext = extension(
        Arc::clone(&dock),
        binding,
        vec![function("expect.tile", ExtensionMode::Request)],
    );
    let mut run = spawn(
        r#"
        scenario_header { name = "probe" }
        return { tick = function()
            assert(not pcall(function() scenario.expect.missing({}) end))
        end }
        "#,
        ext,
    )
    .expect("scenario card starts");

    assert!(matches!(
        dispatch(&mut *run, 1, false),
        LentOutcome::Failed(_)
    ));
    let fault = dock.fault().expect("caught fault remains latched");
    assert_eq!(fault.binding, binding);
    assert_eq!(fault.entry, ExtensionEntry::Tick);
}

/// LD2 V08: an uncaught Tick error in a scenario VM must latch a durable
/// fault, discard the staged calls of that dispatch and stay latched, instead
/// of letting the next tick restart the body.
#[test]
fn uncaught_tick_error_latches_a_durable_scenario_fault() {
    let binding = binding(1);
    let dock = Arc::new(ExtensionDock::new());
    let ext = extension(
        Arc::clone(&dock),
        binding,
        vec![function("step", ExtensionMode::Request)],
    );
    let mut run = spawn(
        r#"
        scenario_header { name = "probe" }
        return { tick = function()
            scenario.step("before", "staged before the error")
            error("deliberate uncaught failure")
        end }
        "#,
        ext,
    )
    .expect("scenario card starts");

    let _ = dispatch(&mut *run, 1, false);
    let fault = dock.fault().expect("uncaught error is latched");
    assert_eq!(fault.code, ExtensionFaultCode::UncaughtError);
    assert_eq!(fault.entry, ExtensionEntry::Tick);
    assert!(
        !matches!(dock.take(binding), Ok(Some(_))),
        "the failed dispatch's staged call is discarded"
    );
    let _ = dispatch(&mut *run, 2, false);
    assert_eq!(
        dock.fault().expect("first fault stays latched").code,
        ExtensionFaultCode::UncaughtError
    );
}

#[test]
fn caught_load_call_is_a_load_failure_and_stays_on_the_dock() {
    let binding = binding(1);
    let dock = Arc::new(ExtensionDock::new());
    let ext = extension(
        Arc::clone(&dock),
        binding,
        vec![function("expect.tile", ExtensionMode::Request)],
    );
    let result = spawn(
        r#"
        scenario_header { name = "probe" }
        pcall(function() scenario.expect.tile({}) end)
        return { tick = function() end }
        "#,
        ext,
    );
    assert!(
        result.is_err(),
        "a caught Load refusal cannot make Load valid"
    );
    assert_eq!(
        dock.fault().expect("Load refusal remains latched").entry,
        ExtensionEntry::Load
    );
}

#[test]
fn finish_seals_and_suspends_without_running_following_or_later_tick_code() {
    let binding = binding(1);
    let dock = Arc::new(ExtensionDock::new());
    let ext = extension(
        Arc::clone(&dock),
        binding,
        vec![function("finish", ExtensionMode::Finish)],
    );
    let mut run = spawn(
        r#"
        scenario_header { name = "probe" }
        return { tick = function()
            api.log("before-finish")
            scenario.finish()
            api.log("after-finish")
        end }
        "#,
        ext,
    )
    .expect("scenario card starts");

    assert_eq!(dispatch(&mut *run, 1, false), LentOutcome::Accepted);
    assert!(dock.is_fenced());
    assert_eq!(run.drain_logs(), vec!["before-finish".to_owned()]);
    let call = dock
        .take(binding)
        .expect("host reads dock")
        .expect("finish request");
    assert_eq!(call.name, "finish");
    assert_eq!(call.mode, ExtensionMode::Finish);

    assert_eq!(dispatch(&mut *run, 2, false), LentOutcome::Accepted);
    assert!(run.drain_logs().is_empty());
}

#[test]
fn caught_stop_hook_refusal_is_distinct_from_hook_failure_and_blocks_clean_end() {
    let binding = binding(1);
    let dock = Arc::new(ExtensionDock::new());
    let ext = extension(
        Arc::clone(&dock),
        binding,
        vec![function("note", ExtensionMode::Request)],
    );
    let run = spawn(
        r#"
        scenario_header { name = "probe" }
        return {
            tick = function() end,
            onStop = function()
                pcall(function() scenario.note("x", "y") end)
            end,
        }
        "#,
        ext,
    )
    .expect("scenario card starts");
    let (tx, rx) = mpsc::channel();
    run.stop(tx);
    let completion = rx.recv().expect("Stop completion");
    assert!(completion.hook_failure.is_none());

    let end = dock.end().expect("typed extension end is retained");
    assert_eq!(end.binding, binding);
    assert!(end.vm_dropped);
    assert!(end.hook_failure.is_none());
    assert_eq!(
        end.first_fault
            .as_ref()
            .expect("caught refusal survives Stop")
            .entry,
        ExtensionEntry::Hook
    );
    assert!(!end.is_clean_end(binding, end.cause));
}
