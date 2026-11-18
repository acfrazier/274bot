use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::identity::ExecutionId;
use api::selected::RunKey;
use script::native::recovery_hints::RecoveryHintsCell;
use script::native::snapshot_source::{NativeFactsInput, ReachViewInput, SnapshotInput};
use script::surface::{LentOutcome, SurfaceFactory, SurfaceRun, SurfaceStart, SurfaceWake};
use script::{native::WalkCancelReason, surface};
use script_luau::{LuauStartCard, LuauSurfaceFactory};

const CHILD_ENV: &str = "SCRIPT_LUAU_STEP_INSTALL_TEST_CHILD";
const CHILD_WATCHDOG: Duration = Duration::from_secs(5);
const MAX_DISPATCH_TIME: Duration = Duration::from_secs(4);
const STRAIGHT_LINE_STATEMENTS: usize = 1_500;

fn straight_line_source() -> String {
    let mut source = String::with_capacity(STRAIGHT_LINE_STATEMENTS * 32 + 256);
    source.push_str(
        "local s, invocation = nil, 0\n\
         return {\n\
         tick = function(api)\n\
         invocation += 1\n\
         if invocation == 1 then\n\
         s = string.rep('9', 4 * 1024 * 1024)\n\
         api.log('prepared')\n\
         return\n\
         end\n\
         local total = 0\n",
    );
    for _ in 0..STRAIGHT_LINE_STATEMENTS {
        source.push_str("total = total + s + s\n");
    }
    source.push_str("api.log(tostring(total))\nend\n}\n");
    source
}

fn spawn(source: &str) -> Box<dyn SurfaceRun> {
    let start = SurfaceStart {
        card: Arc::new(LuauStartCard {
            card_name: Arc::from("step-install-test"),
            source: Arc::from(source),
        }),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        wilderness_rules: surface::no_wilderness(),
        loadouts: Arc::default(),
    };
    LuauSurfaceFactory
        .spawn(
            &start,
            ExecutionId::first(RunKey {
                slot: 1,
                run: 1,
                session: 1,
            }),
            Arc::new(RecoveryHintsCell::default()),
        )
        .expect("the straight-line Luau run starts")
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

fn dispatch(run: &mut dyn SurfaceRun, tick: u64) -> LentOutcome {
    let input = empty_snapshot(tick);
    run.dispatch_lent(
        SurfaceWake::Tick { held: false },
        tick,
        tick,
        &input,
        NativeFactsInput::default(),
    )
}

fn assert_surface_run_uses_the_step_hook() {
    let source = straight_line_source();
    let mut run = spawn(&source);
    assert_eq!(dispatch(&mut *run, 1), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), vec!["prepared".to_owned()]);

    let started = Instant::now();
    let outcome = dispatch(&mut *run, 2);
    let elapsed = started.elapsed();
    assert!(
        elapsed < MAX_DISPATCH_TIME,
        "straight-line dispatch did not stop at the watchdog deadline: {elapsed:?}"
    );
    let LentOutcome::Failed(failure) = outcome else {
        panic!("straight-line dispatch must fail at the watchdog deadline: {outcome:?}");
    };
    assert_eq!(failure.code.as_ref(), "luau-watchdog");
    assert!(run.drain_logs().is_empty());
}

#[test]
fn surface_run_stops_straight_line_work_at_the_watchdog_deadline() {
    if std::env::var_os(CHILD_ENV).is_some() {
        assert_surface_run_uses_the_step_hook();
        return;
    }

    let executable = std::env::current_exe().expect("test executable path is available");
    let mut child = Command::new(executable)
        .args([
            "--exact",
            "surface_run_stops_straight_line_work_at_the_watchdog_deadline",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .spawn()
        .expect("the bounded product-test child starts");
    let deadline = Instant::now() + CHILD_WATCHDOG;
    loop {
        if let Some(status) = child.try_wait().expect("child status is available") {
            assert!(status.success(), "product-test child failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait().expect("timed-out child is reaped");
            panic!("product-test child exceeded its {CHILD_WATCHDOG:?} hard cap: {status}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
