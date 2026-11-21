//! Shared fixtures of the binding, allocation and thinness tests: a real
//! Luau run from the product factory and a lent snapshot frame.

use std::sync::Arc;

use api::identity::ExecutionId;
use api::selected::RunKey;
use script::native::recovery_hints::RecoveryHintsCell;
use script::native::snapshot_source::{NativeFactsInput, ReachViewInput, SnapshotInput};
use script::native::{SettingsBag, WalkCancelReason};
use script::surface::{
    self, LentOutcome, StopCompletion, SurfaceFactory, SurfaceRun, SurfaceStart, SurfaceWake,
};
use script_luau::{LuauStartCard, LuauSurfaceFactory};

pub fn start(source: &str, settings: Option<SettingsBag>) -> SurfaceStart {
    SurfaceStart {
        card: Arc::new(LuauStartCard::new("binding-test", source)),
        settings_bag: settings.map(Arc::new),
        game_data: None,
        named_banks: Arc::default(),
        wilderness_rules: surface::no_wilderness(),
        loadouts: Arc::default(),
    }
}

pub fn spawn_with(
    source: &str,
    settings: Option<SettingsBag>,
) -> Result<Box<dyn SurfaceRun>, surface::SurfaceError> {
    LuauSurfaceFactory.spawn(
        &start(source, settings),
        ExecutionId::first(RunKey {
            slot: 1,
            run: 1,
            session: 1,
        }),
        Arc::new(RecoveryHintsCell::default()),
    )
}

pub fn spawn(source: &str) -> Box<dyn SurfaceRun> {
    spawn_with(source, None).expect("valid Luau run")
}

pub fn snapshot(tick: u64) -> SnapshotInput<'static> {
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

pub fn tick(run: &mut dyn SurfaceRun, input: &SnapshotInput<'_>) -> LentOutcome {
    tick_with(run, input, NativeFactsInput::default())
}

pub fn tick_with(
    run: &mut dyn SurfaceRun,
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
) -> LentOutcome {
    run.dispatch_lent(
        SurfaceWake::Tick { held: false },
        input.tick,
        input.tick,
        input,
        native,
    )
}

pub fn stop(run: Box<dyn SurfaceRun>) -> StopCompletion {
    let (done, completion) = std::sync::mpsc::channel();
    run.stop(done);
    completion.recv().expect("stop completion")
}
