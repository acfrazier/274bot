#[cfg(feature = "test-hooks")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(feature = "test-hooks")]
use std::sync::{mpsc, Mutex};
#[cfg(feature = "test-hooks")]
use std::time::{Duration, Instant};

use api::identity::ExecutionId;
#[cfg(feature = "test-hooks")]
use api::identity::RequestId;
use api::selected::RunKey;
#[cfg(feature = "test-hooks")]
use script::api_progress::ProgressPage;
#[cfg(feature = "test-hooks")]
use script::ctx::test_support::NullDriver;
#[cfg(feature = "test-hooks")]
use script::interact::InteractReq;
use script::native::recovery_hints::RecoveryHintsCell;
#[cfg(feature = "test-hooks")]
use script::native::seat::{SeatResult, SeatTerminal};
#[cfg(feature = "test-hooks")]
use script::native::seat_rows::SeatPageInput;
use script::native::snapshot_source::{NativeFactsInput, ReachViewInput, SnapshotInput};
#[cfg(feature = "test-hooks")]
use script::slot::ScriptTerminalState;
use script::surface::{
    LentOutcome, SurfaceFactory, SurfaceRun, SurfaceStart, SurfaceWake, TickOutcome,
};
use script::{native::WalkCancelReason, surface};
#[cfg(feature = "test-hooks")]
use script::{CompiledTick, ScriptCtx, SlotScript, StartOutcome, StartPoll};
use script_luau::{LuauStartCard, LuauSurfaceFactory};

fn start(source: &str) -> SurfaceStart {
    SurfaceStart {
        card: Arc::new(LuauStartCard {
            card_name: Arc::from("runtime-test"),
            source: Arc::from(source),
        }),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        wilderness_rules: surface::no_wilderness(),
        loadouts: Arc::default(),
    }
}

fn spawn(source: &str) -> Result<Box<dyn SurfaceRun>, surface::SurfaceError> {
    let start = start(source);
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
        walk_outcome_cancel_reason: WalkCancelReason::default(),
    }
}

fn dispatch(run: &mut dyn SurfaceRun, wake: SurfaceWake, tick: u64) -> LentOutcome {
    let input = empty_snapshot(tick);
    run.dispatch_lent(wake, tick, tick, &input, NativeFactsInput::default())
}

#[test]
fn load_requires_a_tick_entry() {
    assert!(spawn("return {}").is_err());
}

#[test]
fn tick_wait_resumes_only_on_an_unheld_game_tick() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                api.log("before")
                api.wait()
                api.log("after")
            end,
            onPaint = function(api)
                api.log("paint")
            end,
        }
        "#,
    )
    .expect("valid Luau run");

    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        vec!["before".to_owned(), "paint".to_owned()]
    );
    assert!(run.drain_tick_outcomes().is_empty());

    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: true }, 2),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), vec!["paint".to_owned()]);
    assert!(run.drain_tick_outcomes().is_empty());

    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 3),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        vec!["after".to_owned(), "paint".to_owned()]
    );
    assert_eq!(
        run.drain_tick_outcomes(),
        vec![TickOutcome::Success {
            tick: 3,
            generation: 1,
        }]
    );
}

#[test]
fn host_binding_errors_are_catchable_without_escaping_the_vm() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                api.log(
                    os == nil,
                    debug == nil,
                    coroutine == nil,
                    loadstring == nil,
                    require == nil,
                    string.pack == nil,
                    string.unpack == nil,
                    string.packsize == nil
                )
                local ok, message = pcall(function()
                    api.settings.str(3)
                end)
                api.log(tostring(ok), message)
                api.log("continued")
            end,
        }
        "#,
    )
    .expect("valid Luau run");

    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );
    let logs = run.drain_logs();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0], "true\ttrue\ttrue\ttrue\ttrue\ttrue\ttrue\ttrue");
    assert!(logs[1].contains("api.settings.str expects a string key"));
    assert_eq!(logs[2], "continued");
    assert_eq!(
        run.drain_tick_outcomes(),
        vec![TickOutcome::Success {
            tick: 1,
            generation: 1,
        }]
    );
}

#[cfg(not(feature = "test-hooks"))]
#[test]
fn product_run_has_no_test_request_bindings() {
    let mut run = spawn(
        r#"
        return {
            tick = function()
                api.log(tostring(api.__test == nil))
            end,
        }
        "#,
    )
    .expect("valid Luau run");
    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), vec!["true"]);
}

#[test]
fn caught_memory_refusal_still_trips_the_run() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                pcall(function()
                    string.rep("x", 18 * 1024 * 1024)
                end)
                api.log("after")
            end,
        }
        "#,
    )
    .expect("valid Luau run");

    let LentOutcome::Failed(failure) = dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1)
    else {
        panic!("caught allocator refusal must trip the run");
    };
    assert_eq!(failure.code.as_ref(), "luau-memory");
    assert!(run.drain_logs().is_empty());
}

#[test]
fn self_stop_ends_the_entry_and_runs_on_stop_once() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                api.log("before")
                api.stop("done")
                api.log("after")
            end,
            onStop = function(reason)
                api.log("stop:" .. reason)
            end,
        }
        "#,
    )
    .expect("valid Luau run");

    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 7),
        LentOutcome::Accepted
    );
    let receipt = run.script_stop_receipt().expect("script stop receipt");
    assert_eq!(receipt.tick, 7);
    assert_eq!(receipt.reason, "done");
    assert!(run.stopped());
    assert_eq!(run.drain_logs(), vec!["before".to_owned()]);
    assert!(run.drain_tick_outcomes().is_empty());

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    run.stop(done_tx);
    let done = done_rx.recv().expect("stop completion");
    assert_eq!(done.hook_failure, None);
    assert_eq!(done.logs, vec!["stop:done".to_owned()]);
}

#[test]
fn self_stop_reason_is_bounded_at_a_utf8_boundary() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                api.stop(string.rep("x", 255) .. "é")
            end,
            onStop = function(reason)
                api.log(#reason)
            end,
        }
        "#,
    )
    .expect("valid Luau run");

    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );
    let receipt = run.script_stop_receipt().expect("script stop receipt");
    assert_eq!(receipt.reason, "x".repeat(255));

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    run.stop(done_tx);
    let done = done_rx.recv().expect("stop completion");
    assert_eq!(done.hook_failure, None);
    assert_eq!(done.logs, vec!["255".to_owned()]);
}

#[cfg(feature = "test-hooks")]
#[test]
fn tick_and_paint_share_one_dispatch_deadline() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                api.log("tick")
                api.__test.advance(550)
            end,
            onPaint = function(api)
                api.__test.advance(100)
            end,
        }
        "#,
    )
    .expect("valid Luau run");

    let LentOutcome::Failed(failure) = dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1)
    else {
        panic!("dispatch should trip its shared 600 ms deadline");
    };
    assert_eq!(failure.code.as_ref(), "luau-watchdog");
    assert_eq!(run.drain_logs(), vec!["tick".to_owned()]);
}

#[cfg(feature = "test-hooks")]
#[test]
fn on_stop_has_a_50ms_deadline() {
    let mut run = spawn(
        r#"
        return {
            tick = function() end,
            onStop = function(reason)
                api.__test.advance(51)
            end,
        }
        "#,
    )
    .expect("valid Luau run");
    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    run.stop(done_tx);
    let done = done_rx.recv().expect("stop completion");
    assert_eq!(
        done.hook_failure
            .as_ref()
            .map(|failure| failure.code.as_ref()),
        Some("luau-watchdog")
    );
    assert_eq!(
        done.hook_failure.unwrap().message.as_ref(),
        "onStop did not finish within 50 ms"
    );
}

#[cfg(feature = "test-hooks")]
#[test]
fn real_clock_test_hook_is_monotonic_and_rejects_arguments() {
    let mut run = spawn(
        r#"
        return {
            tick = function()
                local before = api.__test.realClock()
                local after = api.__test.realClock()
                assert(type(before) == "number" and after >= before)
                local ok, message = pcall(api.__test.realClock, 1)
                assert(not ok and string.find(message, "expects no arguments", 1, true))
                api.log("clock")
            end,
        }
        "#,
    )
    .expect("valid Luau run");
    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["clock"]);
}

#[cfg(feature = "test-hooks")]
fn started_slot(source: &str, arm_seat: bool) -> SlotScript {
    let mut slot = SlotScript::new();
    if arm_seat {
        slot.bind_incarnation(42);
    }
    slot.bind_surface_factory(Arc::new(LuauSurfaceFactory));
    slot.start_surface(start(source)).expect("Luau Start");
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    slot.on_is_up(true);
    slot
}

#[cfg(feature = "test-hooks")]
#[expect(
    clippy::large_types_passed_by_value,
    reason = "SlotScript consumes NativeFactsInput; fixtures move it once instead of copying a large snapshot"
)]
fn slot_wake(
    slot: &mut SlotScript,
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
    tick: bool,
    held: bool,
) -> LentOutcome {
    let mut driver = NullDriver::default();
    let mut ctx = ScriptCtx {
        driver: &mut driver,
        tick: input.tick,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: CompiledTick::default(),
    };
    if tick {
        slot.on_game_tick_lent(&mut ctx, held, input, native)
    } else {
        slot.on_snapshot_change_lent(&mut ctx, input, native)
    }
}

#[cfg(feature = "test-hooks")]
fn slot_tick(slot: &mut SlotScript, tick: u64, held: bool) -> LentOutcome {
    let input = empty_snapshot(tick);
    slot_wake(slot, &input, NativeFactsInput::default(), true, held)
}

#[cfg(feature = "test-hooks")]
#[test]
fn real_slot_wait_settles_through_seat_lifecycle_and_self_stop() {
    let mut slot = started_slot(
        r#"
        local phase = 0
        return {
            tick = function()
                if phase == 0 then
                    phase = 1
                    api.wait()
                    api.log("wait settled")
                else
                    api.stop("done")
                end
            end,
        }
        "#,
        true,
    );
    assert_eq!(slot_tick(&mut slot, 7, false), LentOutcome::Accepted);
    let (_, outgoing, _) = slot.drain_host_interacts(None);
    assert!(outgoing.is_empty());
    let enqueued = slot.drain_lifecycle();
    assert!(
        enqueued.contains(&InteractReq::WaitEnqueued),
        "rows={enqueued:?}; pending={:?}",
        slot.take_pending_logs()
    );

    assert_eq!(slot_tick(&mut slot, 8, true), LentOutcome::Accepted);
    assert!(
        slot.drain_logs().is_empty(),
        "held tick cannot resume a wait"
    );
    assert_eq!(slot_tick(&mut slot, 9, false), LentOutcome::Accepted);
    let (_, _, _) = slot.drain_host_interacts(None);
    let settled = slot.drain_lifecycle();
    assert!(settled.contains(&InteractReq::WaitSettled), "{settled:?}");
    assert!(settled.contains(&InteractReq::LoopSettled), "{settled:?}");
    let logs = slot.drain_logs();
    assert!(logs.iter().any(|line| line == "wait settled"), "{logs:?}");
    assert_eq!(slot_tick(&mut slot, 10, false), LentOutcome::Accepted);
    assert!(
        slot.drain_logs().is_empty(),
        "self-stop has no extra tick logs"
    );
    let receipt = slot.lifecycle_receipt().expect("self-stop receipt");
    assert_eq!(receipt.state, ScriptTerminalState::Stopped);
    assert_eq!(receipt.reason, "done");
}

#[cfg(feature = "test-hooks")]
fn queued_start_id(slot: &mut SlotScript) -> RequestId {
    slot.with_surface_run_for_test(|run| {
        let batches = run.drain_seat_batches();
        assert_eq!(batches.len(), 1, "one test request batch");
        assert_eq!(batches[0].batch.starts.len(), 1, "one queued seat start");
        batches[0].batch.starts[0].id
    })
    .expect("running Luau surface")
}

#[cfg(feature = "test-hooks")]
fn queued_walk_id(slot: &mut SlotScript) -> RequestId {
    slot.with_surface_run_for_test(|run| {
        let batches = run.drain_seat_batches();
        assert_eq!(batches.len(), 1, "one test walk batch");
        batches[0]
            .batch
            .reqs
            .iter()
            .find_map(|row| match &row.request {
                InteractReq::Walk { request_id, .. } => RequestId::isolate(*request_id).ok(),
                _ => None,
            })
            .expect("walk queued through SeatClient")
    })
    .expect("running Luau surface")
}

#[cfg(feature = "test-hooks")]
fn seat_result(id: RequestId, seq: u64, outcome: SeatTerminal) -> SeatResult {
    SeatResult {
        id,
        seq,
        outcome,
        prior_parent: None,
        session: None,
        authority_revoked: false,
    }
}

#[cfg(feature = "test-hooks")]
#[test]
fn real_slot_seat_terminal_resumes_after_pause_and_rejects_stale_ids() {
    let mut slot = started_slot(
        r#"
        return {
            tick = function()
                local result = api.__test.awaitSeat()
                api.log(result.kind .. ":" .. result.status .. ":" .. tostring(result.token))
            end,
        }
        "#,
        false,
    );
    assert_eq!(slot_tick(&mut slot, 1, false), LentOutcome::Accepted);
    let id = queued_start_id(&mut slot);
    let input = empty_snapshot(1);
    let matching = [seat_result(id, 8, SeatTerminal::Done)];
    let page = SeatPageInput::new(&[], &matching, &[], 0).expect("matching page");
    slot.pause();
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                seat_page: Some(page),
                ..Default::default()
            },
            false,
            false,
        ),
        LentOutcome::Refused,
        "a paused slot must not acknowledge the terminal"
    );
    slot.resume();

    let stale_id = RequestId::isolate(id.get() + 1).expect("next isolate id");
    let stale = [seat_result(stale_id, 9, SeatTerminal::Done)];
    let stale_page = SeatPageInput::new(&[], &stale, &[], 0).expect("stale page");
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                seat_page: Some(stale_page),
                ..Default::default()
            },
            false,
            false,
        ),
        LentOutcome::Accepted
    );
    assert!(
        slot.drain_logs().is_empty(),
        "stale ID cannot resume the wait"
    );

    assert_eq!(
        slot_wake(&mut slot, &input, NativeFactsInput::default(), true, true),
        LentOutcome::Accepted
    );
    assert!(
        slot.drain_logs().is_empty(),
        "held tick leaves the seat answer owed"
    );
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                seat_page: Some(page),
                ..Default::default()
            },
            false,
            false,
        ),
        LentOutcome::Accepted
    );
    assert_eq!(
        slot.drain_logs(),
        vec![format!("terminal:done:{}", id.get())]
    );
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                seat_page: Some(page),
                ..Default::default()
            },
            false,
            false,
        ),
        LentOutcome::Accepted
    );
    assert!(slot.drain_logs().is_empty(), "one completion resumes once");
}

#[cfg(feature = "test-hooks")]
#[test]
fn real_slot_walk_completion_copies_actual_request_after_held_tick() {
    let mut slot = started_slot(
        r#"
        return {
            tick = function()
                local result = api.__test.awaitWalk()
                api.log(result.kind .. ":" .. result.status .. ":" ..
                    tostring(result.token) .. ":" .. tostring(result.sequence) .. ":" ..
                    result.reason)
            end,
        }
        "#,
        false,
    );
    let mut input = empty_snapshot(1);
    input.user_move_intent_seq = 17;
    assert_eq!(
        slot_wake(&mut slot, &input, NativeFactsInput::default(), true, false),
        LentOutcome::Accepted
    );
    let id = queued_walk_id(&mut slot);
    slot.pause();
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                walk_outcome_seq: 5,
                walk_outcome_request_id: id.get(),
                local_player_moving: Some(false),
                ..Default::default()
            },
            false,
            false,
        ),
        LentOutcome::Refused,
        "paused slot must not accept a walk end"
    );
    slot.resume();

    let mut stale_input = empty_snapshot(2);
    stale_input.user_move_intent_seq = 17;
    assert_eq!(
        slot_wake(
            &mut slot,
            &stale_input,
            NativeFactsInput {
                walk_outcome_seq: 6,
                walk_outcome_request_id: id.get() + 1,
                local_player_moving: Some(false),
                ..Default::default()
            },
            true,
            true,
        ),
        LentOutcome::Accepted
    );
    assert!(
        slot.drain_logs().is_empty(),
        "wrong walk token cannot settle"
    );

    let mut completion_input = empty_snapshot(3);
    completion_input.user_move_intent_seq = 17;
    assert_eq!(
        slot_wake(
            &mut slot,
            &completion_input,
            NativeFactsInput {
                walk_outcome_seq: 7,
                walk_outcome_request_id: id.get(),
                local_player_moving: Some(false),
                ..Default::default()
            },
            true,
            true,
        ),
        LentOutcome::Accepted
    );
    assert!(
        slot.drain_logs().is_empty(),
        "held tick copies but does not resume"
    );

    assert_eq!(slot_tick(&mut slot, 4, false), LentOutcome::Accepted);
    assert_eq!(
        slot.drain_logs(),
        vec![format!("walk:done:{}:7:none", id.get())],
        "the copied walk end survives the borrowed input frame"
    );
}

#[cfg(feature = "test-hooks")]
#[test]
fn real_slot_progress_family_completion_survives_page_clear_before_resume() {
    let mut slot = started_slot(
        r#"
        return {
            tick = function()
                local result = api.__test.awaitProgress()
                api.log(result.kind .. ":" .. result.status .. ":" ..
                    result.reason .. ":" .. tostring(result.token))
            end,
        }
        "#,
        false,
    );
    assert_eq!(slot_tick(&mut slot, 1, false), LentOutcome::Accepted);
    let id = queued_start_id(&mut slot);
    slot.set_progress_page_for_test(Some(ProgressPage::Refused {
        token: id.get(),
        reason: Arc::from("busy"),
    }));
    let result = [seat_result(id, 12, SeatTerminal::Failed)];
    let page = SeatPageInput::new(&[], &result, &[], 0).expect("progress terminal");
    let input = empty_snapshot(2);
    slot.pause();
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                seat_page: Some(page),
                ..Default::default()
            },
            false,
            false,
        ),
        LentOutcome::Refused,
        "paused slot must not accept the family terminal"
    );
    slot.resume();
    assert_eq!(
        slot_wake(
            &mut slot,
            &input,
            NativeFactsInput {
                seat_page: Some(page),
                ..Default::default()
            },
            true,
            true,
        ),
        LentOutcome::Accepted
    );
    assert!(
        slot.drain_logs().is_empty(),
        "held tick does not run the waiting script"
    );
    slot.set_progress_page_for_test(None);

    assert_eq!(slot_tick(&mut slot, 3, false), LentOutcome::Accepted);
    assert_eq!(
        slot.drain_logs(),
        vec![format!("progress:refused:busy:{}", id.get())],
        "the typed family page, not the bare seat end, must be copied before its source clears"
    );
}

// Ignored lab cell: these calls run through the production Luau factory,
// installed prelude, single-step hook, allocator limit and tick dispatch.
// Preparing inputs on the first tick keeps input construction out of the
// measured tick without charging the 50 ms load budget for the fixture.
const C8_REF_HEAD: &str = r#"
local function get(t, k) return t[k] end
local rep, find, match, gsub = get(string, "rep"), get(string, "find"), get(string, "match"), get(string, "gsub")
local split, lower, format, char = get(string, "split"), get(string, "lower"), get(string, "format"), get(string, "char")
local concat, sort, unpack, create = get(table, "concat"), get(table, "sort"), get(table, "unpack"), get(table, "create")
local move, tfind = get(table, "move"), get(table, "find")
local ulen, uchar = get(utf8, "len"), get(utf8, "char")
local tonum = tonumber
"#;

fn c8_ref_source(body: &str) -> String {
    format!(
        "{C8_REF_HEAD}\nlocal fn\nreturn {{ tick = function(api)\n\
         if fn == nil then\nfn = (function()\n{body}\nend)()\napi.log('ready')\n\
         else\nlocal ok, value = pcall(fn)\n\
         api.log(ok and 'ok' or 'error:' .. tostring(value))\nend\nend }}"
    )
}

fn c8_ref_start(body: &str) -> Box<dyn SurfaceRun> {
    let mut run = spawn(&c8_ref_source(body)).expect("C8 product run starts");
    assert_eq!(
        dispatch(&mut *run, SurfaceWake::Tick { held: false }, 1),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        ["ready"],
        "fixture prepared on first tick"
    );
    run
}

#[test]
#[rustfmt::skip]
#[ignore = "REF-only measured product-path cap cell; prints per-row timing, not a CI timing gate"]
fn c8_product_cap_timing_on_reference_guest() {
    // The worst-content native fixtures from LD1 r7, at the product guard's
    // accepted boundary. The allocator-limited 8 MiB concat result is omitted:
    // it can fail for heap size although its N_c guard admits the input.
    let caps = [
        ("find-plain-254417-1024", r#"local s = rep("a", 254417) local p = rep("a", 1023) .. "b" return function() return find(s, p, 1, true) end"#),
        ("find-plain-32K-8K", r#"local s = rep("a", 32768) local p = rep("a", 8191) .. "b" return function() return find(s, p, 1, true) end"#),
        ("find-implicit-199088-16", r#"local s = rep("a", 199088) local p = rep("a", 15) .. "b" return function() return find(s, p) end"#),
        ("split-254417-1024", r#"local s = rep("a", 254417) local p = rep("a", 1023) .. "b" return function() return #split(s, p) end"#),
        ("split-allsep-254417", r#"local s = rep(",", 254417) return function() return #split(s, ",") end"#),
        ("split-emptysep-254417", r#"local s = rep("a", 254417) return function() return #split(s, "") end"#),
        ("pat-bracket-199088-16", r#"local s = rep("z", 199088) local p = "^[" .. rep("a", 10) .. "z]*X" assert(#p == 16) return function() return find(s, p) end"#),
        ("pat-bracket-49772-64", r#"local s = rep("z", 49772) local p = "^[" .. rep("a", 58) .. "z]*X" assert(#p == 64) return function() return find(s, p) end"#),
        ("pat-bracket-12443-256", r#"local s = rep("z", 12443) local p = "^[" .. rep("a", 250) .. "z]*X" assert(#p == 256) return function() return find(s, p) end"#),
        ("pat-dot-254417", r#"local s = rep("z", 254417) local p = "^.*X" return function() return find(s, p) end"#),
        ("pat-class-254417", r#"local s = rep("z", 254417) local p = "^%a*X" return function() return find(s, p) end"#),
        ("pat-balance-254417", r#"local s = rep("(", 254417) local p = "^%b()" return function() return find(s, p) end"#),
        ("pat-items-256", r#"local s = rep("a", 12443) local p = "^" .. rep("a", 254) .. "b" return function() return match(s, p) end"#),
        ("gsub-repl-4096", r#"local s = rep("x", 64) local r = rep("%1", 2048) return function() local _, n = gsub(s, "(y*)", r) return n end"#),
        ("format-d-1024", r#"local fmt = rep("%d", 512) local t = create(512, 9007199254740992) return function() return #format(fmt, unpack(t)) end"#),
        ("format-f99-1020", r#"local fmt = rep("%.99f", 204) local t = create(204, -1e308) return function() return #format(fmt, unpack(t)) end"#),
        ("format-q-256K-quote", r#"local s = rep("\"", 262144) return function() return #format("%q", s) end"#),
        ("format-q-256K-nul", r#"local s = rep("\0", 262144) return function() return #format("%q", s) end"#),
        ("char-8000", r#"local t = create(8000, 65) return function() return #char(unpack(t)) end"#),
        ("utf8char-8000", r#"local t = create(8000, 1114111) return function() return #uchar(unpack(t)) end"#),
        ("math-min-8000", r#"local t = create(8000, 1) return function() return math.min(unpack(t)) end"#),
        ("math-max-8000", r#"local t = create(8000, 1) return function() return math.max(unpack(t)) end"#),
        ("bit32-band-8000", r#"local t = create(8000, 1) return function() return bit32.band(unpack(t)) end"#),
        ("bit32-bor-8000", r#"local t = create(8000, 1) return function() return bit32.bor(unpack(t)) end"#),
        ("bit32-bxor-8000", r#"local t = create(8000, 1) return function() return bit32.bxor(unpack(t)) end"#),
        ("bit32-btest-8000", r#"local t = create(8000, 1) return function() return bit32.btest(unpack(t)) end"#),
        ("concat-16384-floats", r#"local t = create(16384, 0) for i = 1, 16384 do t[i] = i + 0.1 end return function() return #concat(t) end"#),
        ("concat-16384-ints", r#"local t = create(16384, 0) for i = 1, 16384 do t[i] = i end return function() return #concat(t) end"#),
        ("concat-16384-str-sep", r#"local t = create(16384, "xxxxxxxxxxxxxx") return function() return #concat(t, ",") end"#),
        ("move-empty-range-65536", r#"local a, b = {}, {} return function() move(a, 1, 65536, 2, b) end"#),
        ("move-hash-65536", r#"local a = {} for i = 1, 65536 do a[i + 1048576] = i end return function() local b = {} move(a, 1048577, 1048576 + 65536, 2, b) end"#),
        ("move-dense-65536", r#"local a, b = create(65536, 1), create(65536, 0) return function() move(a, 1, 65536, 1, b) end"#),
        ("sort-wrapped-comparator", r#"local t = create(256, 1) return function() sort(t, function(a, b) return a < b end) end"#),
        ("find-luau-loop-table", r#"local e, v = {}, {} local mt = { __eq = function() return false end } setmetatable(e, mt) setmetatable(v, mt) local t = create(4096, e) return function() return tfind(t, v) end"#),
        ("tonumber-decimal-12M", r#"local s = rep("0", 12582912) return function() return tonum(s) end"#),
        ("tonumber-hex-12M", r#"local s = rep("f", 12582912) return function() return tonum(s, 16) end"#),
        ("utf8len-12M", r#"local s = rep("\244\143\191\191", 3145728) return function() return ulen(s) end"#),
        ("lower-6M", r#"local s = rep("A", 6291456) return function() return #lower(s) end"#),
        ("rep-12M", r#"local s = rep("a", 1048576) return function() return #rep(s, 12) end"#),
    ];
    for (name, body) in caps {
        let mut timings = [0.0; 7];
        for ms in &mut timings {
            // Each sample owns a new VM: large output buffers can otherwise
            // accumulate until the independent 16 MiB heap latch trips.
            let mut run = c8_ref_start(body);
            let tick = 2;
            let start = std::time::Instant::now();
            let outcome = dispatch(&mut *run, SurfaceWake::Tick { held: false }, tick);
            *ms = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(outcome, LentOutcome::Accepted, "{name}: dispatch {tick}");
            assert_eq!(run.drain_logs(), ["ok"], "{name}: at cap {tick}");
        }
        timings.sort_by(f64::total_cmp);
        println!("C8_REF_CAP {name} min_ms={:.4} median_ms={:.4} max_ms={:.4} target_ms=10", timings[0], timings[3], timings[6]);
    }

    // Numeric caps refuse the first input beyond their boundary, including
    // the 8,001-argument case after Luau has expanded the arguments.
    let over = [
        ("find-subject-N_s-plus-1", r#"local s = rep("a", 254418) return function() return find(s, "a", 1, true) end"#, "N_s"),
        ("find-pattern-N_p-plus-1", r#"local p = rep("a", 257) return function() return find("a", p) end"#, "N_p"),
        ("find-work-above-W_p", r#"local s = rep("a", 199089) local p = rep("a", 16) return function() return find(s, p) end"#, "W_p"),
        ("find-work-above-W_n", r#"local s = rep("a", 254417) local p = rep("a", 1056) return function() return find(s, p, 1, true) end"#, "W_n"),
        ("split-work-above-W_n", r#"local s = rep("a", 254417) local p = rep("a", 1056) return function() return split(s, p) end"#, "W_n"),
        ("gsub-replacement-N_r-plus-1", r#"local r = rep("x", 4097) return function() return gsub("x", "x", r) end"#, "N_r"),
        ("format-N_f-plus-1", r#"local fmt = rep("x", 1025) return function() return format(fmt) end"#, "N_f"),
        ("format-N_a-plus-1", r#"local s = rep("a", 262145) return function() return format("%s", s) end"#, "N_a"),
        ("concat-N_c-plus-1", r#"local t = create(16385, "x") return function() return concat(t) end"#, "N_c"),
        ("move-N_m-plus-1", r#"local a, b = {}, {} return function() return move(a, 1, 65537, 2, b) end"#, "N_m"),
        ("char-N_v-plus-1", r#"local t = create(8001, 65) return function() return char(unpack(t, 1, 8001)) end"#, "N_v"),
    ];
    for (name, body, cap) in over {
        let mut run = c8_ref_start(body);
        assert_eq!(
            dispatch(&mut *run, SurfaceWake::Tick { held: false }, 2),
            LentOutcome::Accepted,
            "{name}: refused dispatch"
        );
        let logs = run.drain_logs();
        assert_eq!(logs.len(), 1, "{name}: one refusal");
        assert!(logs[0].starts_with("error:"), "{name}: {logs:?}");
        assert!(logs[0].contains(cap), "{name}: {logs:?}");
        println!("C8_REF_OVER {name} refused={cap}");
    }
}

#[test]
#[rustfmt::skip]
#[ignore = "REF-only proportional C8 pattern-cap recheck; prints timing, not a CI timing gate"]
fn c8_adjusted_pattern_caps_on_reference_guest() {
    // L2b's first REF run exceeded 10 ms for these three rows. Recheck only
    // their reduced boundaries, each in a fresh product VM.
    let caps = [
        ("pat-bracket-199088-16", r#"local s = rep("z", 199088) local p = "^[" .. rep("a", 10) .. "z]*X" assert(#p == 16) return function() return find(s, p) end"#),
        ("pat-dot-254417", r#"local s = rep("z", 254417) local p = "^.*X" return function() return find(s, p) end"#),
        ("pat-class-254417", r#"local s = rep("z", 254417) local p = "^%a*X" return function() return find(s, p) end"#),
    ];
    for (name, body) in caps {
        let mut timings = [0.0; 7];
        for ms in &mut timings {
            let mut run = c8_ref_start(body);
            let start = std::time::Instant::now();
            assert_eq!(dispatch(&mut *run, SurfaceWake::Tick { held: false }, 2), LentOutcome::Accepted, "{name}: at cap");
            *ms = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(run.drain_logs(), ["ok"], "{name}: at cap");
        }
        timings.sort_by(f64::total_cmp);
        println!("C8_REF_ADJUSTED {name} min_ms={:.4} median_ms={:.4} max_ms={:.4} target_ms=10", timings[0], timings[3], timings[6]);
    }
}

#[test]
#[rustfmt::skip]
#[ignore = "REF-only 10-sample final bracket cap recheck; prints each sample, not a CI timing gate"]
fn c8_bracket_final_cap_on_reference_guest() {
    // The bracket row is controlled by W_p: floor(W_p / 16) = 199088.
    // Each sample owns a new product VM to exclude retained heap artifacts.
    let body = r#"local s = rep("z", 199088) local p = "^[" .. rep("a", 10) .. "z]*X" assert(#p == 16) return function() return find(s, p) end"#;
    let mut samples = [0.0_f64; 10];
    for (index, ms) in samples.iter_mut().enumerate() {
        let mut run = c8_ref_start(body);
        let start = std::time::Instant::now();
        assert_eq!(dispatch(&mut *run, SurfaceWake::Tick { held: false }, 2), LentOutcome::Accepted, "sample {index}");
        *ms = start.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(run.drain_logs(), ["ok"], "sample {index}");
        println!("C8_REF_BRACKET_SAMPLE index={index} ms={ms:.4}");
    }
    samples.sort_by(f64::total_cmp);
    println!("C8_REF_BRACKET_FINAL n=10 median_ms={:.4} max_ms={:.4} target_ms=10", (samples[4] + samples[5]) / 2.0, samples[9]);
}

#[cfg(feature = "test-hooks")]
const Q8_REFERENCE: &str = r#"
    local retained = buffer.create(262144)
    return {
        tick = function()
            local n = 0
            for i = 1, 64 do n = n + i end
            if n < 0 then api.log(buffer.len(retained)) end
        end,
        onStop = function()
            api.log("stopped")
        end,
    }
"#;

#[cfg(feature = "test-hooks")]
const Q8_LEGAL_LOOP: &str = r#"
    local retained = buffer.create(15 * 1024 * 1024)
    local function spin(seconds)
        local finish = api.__test.realClock() + seconds
        local n = 0
        while api.__test.realClock() < finish do
            for i = 1, 512 do n = n + i end
        end
        if n < 0 then api.log(buffer.len(retained)) end
    end
    return {
        tick = function() spin(0.590) end,
        onStop = function() spin(0.045) end,
    }
"#;

#[cfg(feature = "test-hooks")]
const Q8_NATIVE_GAP: &str = r#"
    local subject = string.rep("z", 199088)
    local pattern = "^[" .. string.rep("a", 10) .. "z]*X"
    return {
        tick = function() string.find(subject, pattern) end,
        onStop = function() string.find(subject, pattern) end,
    }
"#;

#[cfg(feature = "test-hooks")]
#[derive(Clone, Copy)]
struct Q8Sample {
    lock_wait: Duration,
    stop_work_upper: Duration,
    whole: Duration,
    heap_bytes: u64,
    tick_failures: usize,
    hook_failures: usize,
    missing_stop_logs: usize,
    dispatch_ceiling_hits: usize,
}

#[cfg(feature = "test-hooks")]
fn q8_next_random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[cfg(feature = "test-hooks")]
fn q8_real_slot_sample(
    source: &str,
    roles: usize,
    pause: bool,
    phase_us: u64,
    dispatch_ceiling: usize,
    seed: &mut u64,
    expect_stop_log: bool,
) -> Q8Sample {
    let slots: Vec<_> = (0..roles)
        .map(|_| Arc::new(Mutex::new(started_slot(source, false))))
        .collect();
    let flags: Vec<_> = (0..roles)
        .map(|_| Arc::new(AtomicBool::new(false)))
        .collect();
    let heap_bytes = slots[0]
        .lock()
        .expect("Q8 slot mutex")
        .memory_metrics()
        .and_then(|metrics| metrics["used_bytes"].as_u64())
        .expect("Luau VM used bytes");
    let phase = Duration::from_micros(q8_next_random(seed) % phase_us);
    let (lock_wait, stop_work_upper, whole, tick_failures, dispatch_ceiling_hits) =
        std::thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let mut workers = Vec::with_capacity(roles);
            for (slot, flag) in slots.iter().zip(&flags) {
                let slot = Arc::clone(slot);
                let flag = Arc::clone(flag);
                let tx = started_tx.clone();
                workers.push(scope.spawn(move || {
                    for tick in 1..=dispatch_ceiling {
                        let mut guard = slot.lock().expect("Q8 slot mutex");
                        if flag.load(Ordering::Acquire) {
                            return (0, false);
                        }
                        if tick == 1 {
                            tx.send(()).expect("Q8 control still receiving starts");
                        }
                        let outcome = slot_tick(&mut guard, tick as u64, false);
                        // HostPlay drains each seat batch before its next
                        // observed tick; otherwise repeated LoopSettled rows
                        // fill SeatClient's queue and manufacture an internal
                        // failure unrelated to Stop latency.
                        guard
                            .with_surface_run_for_test(|run| run.drain_seat_batches())
                            .expect("running Luau surface");
                        if !matches!(outcome, LentOutcome::Accepted) {
                            return (1, false);
                        }
                    }
                    (0, true)
                }));
            }
            drop(started_tx);
            for _ in 0..roles {
                started_rx
                    .recv()
                    .expect("every role entered its first tick");
            }
            // The control thread arrives at a deterministic random phase of
            // real dispatch work, as in the L2p proxy. The flag prevents an
            // unfair second dispatch after Stop is already requested.
            std::thread::park_timeout(phase);
            let all_start = Instant::now();
            let mut wait = Duration::ZERO;
            let mut work = Duration::ZERO;
            for (slot, flag) in slots.iter().zip(&flags) {
                flag.store(true, Ordering::Release);
                let start = Instant::now();
                let mut guard = slot.lock().expect("Q8 slot mutex");
                wait += start.elapsed();
                let after_lock = Instant::now();
                if pause {
                    guard.pause();
                } else {
                    guard.stop();
                }
                work += after_lock.elapsed();
            }
            let whole = all_start.elapsed();
            let mut failures = 0;
            let mut ceiling_hits = 0;
            for worker in workers {
                let (failed, reached_ceiling) = worker.join().expect("Q8 dispatch thread");
                failures += failed;
                ceiling_hits += usize::from(reached_ceiling);
            }
            (wait, work, whole, failures, ceiling_hits)
        });
    let mut hook_failures = 0;
    let mut missing_stop_logs = 0;
    for (role, slot) in slots.iter().enumerate() {
        let mut guard = slot.lock().expect("Q8 slot mutex");
        if pause {
            guard.stop();
        }
        let logs = guard.take_pending_logs();
        if expect_stop_log {
            let count = logs
                .iter()
                .filter(|line| line.as_str() == "stopped")
                .count();
            if count != 1 {
                missing_stop_logs += 1;
                eprintln!(
                    "Q8_REF_LOG_ANOMALY role={role} pause={pause} count={count} logs={logs:?} last_error={:?} state={:?}",
                    guard.last_error(),
                    guard.state()
                );
            }
        }
        hook_failures += usize::from(guard.last_error().is_some());
    }
    Q8Sample {
        lock_wait,
        stop_work_upper,
        whole,
        heap_bytes,
        tick_failures,
        hook_failures,
        missing_stop_logs,
        dispatch_ceiling_hits,
    }
}

#[cfg(feature = "test-hooks")]
fn q8_max_p99(values: &mut [Duration]) -> (f64, f64) {
    values.sort_unstable();
    let p99 = (values.len() * 99).div_ceil(100) - 1;
    (
        values[values.len() - 1].as_secs_f64() * 1_000.0,
        values[p99].as_secs_f64() * 1_000.0,
    )
}

#[cfg(feature = "test-hooks")]
#[test]
#[ignore = "REF-only real SlotScript control latency lab cell; 1,000 real-clock samples per class and configuration"]
fn q8_real_slot_stop_all_on_reference_guest() {
    let samples = std::env::var("LUAU_Q8_SAMPLES")
        .map_or(1000, |value| value.parse::<usize>().expect("sample count"));
    assert!((1..=1000).contains(&samples), "bounded Q8 sample count");
    let mut seed = 0x274b_0a11_0301_u64;
    for (class, source, phase_us, dispatch_ceiling) in [
        ("reference-shape", Q8_REFERENCE, 1000, 1024),
        ("legal-loop", Q8_LEGAL_LOOP, 590_000, 16),
        ("native-gap", Q8_NATIVE_GAP, 10_000, 128),
    ] {
        for (roles, pause) in [(1, false), (10, false), (1, true)] {
            let mut rows = Vec::with_capacity(samples);
            for _ in 0..samples {
                rows.push(q8_real_slot_sample(
                    source,
                    roles,
                    pause,
                    phase_us,
                    dispatch_ceiling,
                    &mut seed,
                    class == "reference-shape",
                ));
            }
            let mut waits: Vec<_> = rows.iter().map(|row| row.lock_wait).collect();
            let mut works: Vec<_> = rows.iter().map(|row| row.stop_work_upper).collect();
            let mut wholes: Vec<_> = rows.iter().map(|row| row.whole).collect();
            let (w1_max, w1_p99) = q8_max_p99(&mut waits);
            let (w24_max, w24_p99) = q8_max_p99(&mut works);
            let (whole_max, whole_p99) = q8_max_p99(&mut wholes);
            let tick_failures: usize = rows.iter().map(|row| row.tick_failures).sum();
            let hook_failures: usize = rows.iter().map(|row| row.hook_failures).sum();
            let missing_stop_logs: usize = rows.iter().map(|row| row.missing_stop_logs).sum();
            let ceiling_hits: usize = rows.iter().map(|row| row.dispatch_ceiling_hits).sum();
            let action = if pause { "pause" } else { "stop" };
            println!(
                "Q8_REF class={class} roles={roles} action={action} samples={samples} heap_bytes={} w1_max_ms={w1_max:.4} w1_p99_ms={w1_p99:.4} w2_plus_w4_upper_max_ms={w24_max:.4} w2_plus_w4_upper_p99_ms={w24_p99:.4} whole_max_ms={whole_max:.4} whole_p99_ms={whole_p99:.4} tick_failures={tick_failures} hook_failures={hook_failures} missing_stop_logs={missing_stop_logs} dispatch_ceiling_hits={ceiling_hits}",
                rows[0].heap_bytes
            );
            if class == "reference-shape" {
                assert_eq!(tick_failures, 0, "reference ticks must not trip");
                assert_eq!(hook_failures, 0, "reference hook must not trip");
                assert_eq!(missing_stop_logs, 0, "reference onStop logs once");
                let limit_ms = if roles == 10 { 200.0 } else { 20.0 };
                if !pause {
                    println!(
                        "Q8_REF_BOUND class=reference-shape roles={roles} target_ms={limit_ms} max_within_target={}",
                        whole_max <= limit_ms
                    );
                }
            }
        }
    }
}
