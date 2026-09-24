//! The checked-in hunt examples themselves, run in a real isolate against a
//! toy world that walks one tile a tick toward the last walk it was sent.
//! Their requests go into the act ledger as host-play records them, and
//! their paint and stop into the watch. Deleting the awaited run from an
//! example must fail its cell.

use host_play::catalog_core::{parse_hunt_receipt_line, LineOfSightTile, ScriptAct};
use script::isolate_fb::{encode_snapshot, ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

use super::{ht, HuntRun};

const LEAVE_EXAMPLE: &str = include_str!("../../../script/examples/leave_lair_v2.ts");
const ENTER_EXAMPLE: &str = include_str!("../../../script/examples/enter_lair_v2.ts");

fn snapshot(tick: u64, here: LineOfSightTile) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: Some(TileInput {
            x: here.x,
            z: here.z,
            level: here.level,
        }),
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
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

/// A world walk as host-play records it once queued.
fn recorded_walk(req: &InteractReq) -> Option<ScriptAct> {
    let (x, z, level, radius, exact, teleports, wilderness, bank_fetch, request_id) = match *req {
        InteractReq::Walk {
            x,
            z,
            level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
        } => (
            x,
            z,
            level,
            0,
            true,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
        ),
        InteractReq::WalkNear {
            x,
            z,
            level,
            radius,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
        } => (
            x,
            z,
            level,
            radius,
            false,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
        ),
        _ => return None,
    };
    Some(ScriptAct::Walk {
        dest: LineOfSightTile { x, z, level },
        radius,
        exact,
        allow_teleports: teleports,
        allow_wilderness: wilderness,
        allow_bank_fetch: bank_fetch,
        request_id,
    })
}

/// Transpile `source` as a File card is, run it from `from` for up to 40
/// ticks, and report whether `case` qualifies.
fn example_through_the_gate(case: &str, source: &str, from: LineOfSightTile) -> bool {
    let mut run = HuntRun::start(case, from);
    let js = script::transpile_ts(source).expect("the example transpiles");
    let iso = LoadIsolate::spawn(js, LoadShape::NativeTick, vec![]).unwrap();
    let mut here = from;
    let mut walk: Option<(LineOfSightTile, i32)> = None;
    for tick in 1..=40 {
        iso.post_snapshot(encode_snapshot(&snapshot(tick, here)));
        iso.on_game_tick(tick);
        // Answered after the tick, so the drain below sees its requests.
        let _ = iso.probe("1");
        for act in iso.drain_interacts().iter().filter_map(recorded_walk) {
            if let ScriptAct::Walk { dest, radius, .. } = act {
                walk = Some((dest, radius));
            }
            run.ledger.record(act);
        }
        run.obs.tile = Some((here.x, here.z, here.level));
        run.obs.hunt.receipt = iso.paint().and_then(|paint| {
            paint
                .lines
                .iter()
                .find_map(|line| parse_hunt_receipt_line(run.cell, line))
        });
        if let Some(stop) = iso.script_stop_receipt() {
            run.obs.script_lifecycle = Some(script::ScriptLifecycleReceipt {
                runtime_generation: 1,
                state: script::ScriptTerminalState::Stopped,
                tick: stop.tick,
                reason: stop.reason,
            });
        }
        run.frame();
        if iso.stopped() {
            break;
        }
        if let Some((dest, radius)) = walk {
            if (here.x - dest.x).abs().max((here.z - dest.z).abs()) > radius {
                here.x += (dest.x - here.x).signum();
                here.z += (dest.z - here.z).signum();
            }
        }
    }
    iso.join();
    run.watch.qualify().is_ok()
}

/// The example with its awaited run replaced by a settled `done(true)`.
fn without_the_run(source: &str, run_call: &str) -> String {
    assert!(source.contains(run_call), "the example awaits {run_call}");
    source.replace(run_call, "({ kind: 'done', value: true } as const)")
}

#[test]
fn the_leave_example_passes_and_fails_without_its_run() {
    let from = ht(3222, 3218);
    assert!(
        example_through_the_gate("leave_lair_v2_ts", LEAVE_EXAMPLE, from),
        "the checked-in leave example qualifies"
    );
    let no_run = without_the_run(LEAVE_EXAMPLE, "await api.leaveRun(site, hooks)");
    assert!(
        !example_through_the_gate("leave_lair_v2_ts", &no_run, from),
        "deleting its leaveRun fails the cell"
    );
}

#[test]
fn the_enter_example_passes_and_fails_without_its_run() {
    let from = ht(3222, 3218);
    assert!(
        example_through_the_gate("enter_lair_v2_ts", ENTER_EXAMPLE, from),
        "the checked-in enter example qualifies"
    );
    let no_run = without_the_run(ENTER_EXAMPLE, "await api.enterRun({ token }, hooks)");
    assert!(
        !example_through_the_gate("enter_lair_v2_ts", &no_run, from),
        "deleting its enterRun fails the cell"
    );
}
