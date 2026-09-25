//! G5: `Supervisor.noteProgress` forwards to `Execution.noteProgress` / host
//! `note-progress` lifecycle op. RangingGuild import graph after the shim.

use script::isolate_fb::{ReachViewInput, SnapshotInput, TileInput};
use script::load::{JsLibrary, LoadShape};
use script::shim::InteractReq;
use script::{LoadIsolate, ScriptSource};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2655,
            z: 3298,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
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

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn tick(iso: &LoadIsolate, n: u64) {
    let mut snap = base_snapshot();
    snap.tick = n;
    post_snapshot_input(iso, &snap);
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

#[test]
fn supervisor_note_progress_import_is_remapped() {
    let src = r#"
import { Supervisor } from '../../runtime/Supervisor.js';
export default class T extends LoopingBot {
    loop() { Supervisor.noteProgress(); }
}
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn supervisor_note_progress_queues_host_lifecycle_op() {
    let src = r#"
import { Supervisor } from '../../runtime/Supervisor.js';
export default class T extends LoopingBot {
    loop() { Supervisor.noteProgress(); }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "note-progress is not a game op"
    );
    let life = iso.drain_lifecycle();
    assert!(
        life.iter().any(|r| matches!(r, InteractReq::NoteProgress)),
        "Supervisor.noteProgress must reach host note-progress: {life:?}"
    );
    iso.join();
}

#[test]
fn ranging_guild_card_has_no_import_blockers_after_supervisor_shim() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let card_path = root.join("src/bot/scripts/RangingGuild/RangingGuild.ts");
    if !card_path.is_file() {
        return;
    }
    let origin = std::fs::read_to_string(&card_path).expect("RangingGuild.ts");
    assert_eq!(
        script::load::first_unloadable_for_card(&origin, &card_path),
        None,
        "RangingGuild must not remain import-blocked on Supervisor"
    );
}

#[test]
fn ranging_guild_catalog_row_not_supervisor_unloadable() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let card_path = root.join("src/bot/scripts/RangingGuild/RangingGuild.ts");
    if !card_path.is_file() {
        eprintln!("skip: pinned catalog lacks RangingGuild");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "274bot-ranging-supervisor-{}-{}",
        std::process::id(),
        "catalog"
    ));
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    let card = lib
        .get(ScriptSource::Catalog, "RangingGuild")
        .cloned()
        .expect("RangingGuild catalog card");
    assert_eq!(
        card.unloadable, None,
        "G5 removes ../../runtime/Supervisor.js from RangingGuild unloadable stamp"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
