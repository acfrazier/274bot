//! Shared helpers for `script` integration tests.
//!
//! Intentional fixture variants (`base_snapshot`, seeds, `scene_state`, …) stay
//! in each test file; only byte-identical scaffolding is centralized here.

use script::isolate_fb::{ReachViewInput, SnapshotInput, StatInput, TileInput};
use script::LoadIsolate;

#[allow(dead_code)]
pub fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

const fn stat(index: i32, name: &'static str) -> StatInput<'static> {
    StatInput {
        index,
        name,
        xp: 0,
        base: 1,
        effective: 1,
    }
}

/// A fresh account's stat rows (client stat slot, name): every used stat
/// is loaded, so a compat card's `onPaint` gate (stats ready) holds. The
/// live host posts these once the login's stat packets arrive.
#[allow(dead_code)]
pub const FRESH_STATS: [StatInput<'static>; 19] = [
    stat(0, "attack"),
    stat(1, "defence"),
    stat(2, "strength"),
    StatInput {
        index: 3,
        name: "hitpoints",
        xp: 1154,
        base: 10,
        effective: 10,
    },
    stat(4, "ranged"),
    stat(5, "prayer"),
    stat(6, "magic"),
    stat(7, "cooking"),
    stat(8, "woodcutting"),
    stat(9, "fletching"),
    stat(10, "fishing"),
    stat(11, "firemaking"),
    stat(12, "crafting"),
    stat(13, "smithing"),
    stat(14, "mining"),
    stat(15, "herblore"),
    stat(16, "agility"),
    stat(17, "thieving"),
    stat(20, "runecraft"),
];

/// A logged-in client on a loaded scene: a tile, scene state 2 and the
/// fresh stat rows, every other table empty.
#[allow(dead_code)]
pub fn ingame_snapshot() -> SnapshotInput<'static> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3222,
            z: 3222,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &FRESH_STATS,
        booths: &[],
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
        side_tab: 0,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("bot"),
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
