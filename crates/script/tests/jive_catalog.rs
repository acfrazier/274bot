//! Frozen JiveCrafting / JiveEnchanter full-module load and onPaint.
//! Not catalog enablement and not gameplay PASS.

use std::path::{Path, PathBuf};

use script::isolate_fb::{
    encode_snapshot, NearestBoothInput, ReachViewInput, SnapshotInput, StatInput, TileInput,
};
use script::load::first_unloadable_for_card;
use script::{CacheMeta, JsCache, JsLibrary, LoadIsolate, ScriptKind, ScriptSource};

mod common;

fn frozen_root() -> PathBuf {
    let root =
        PathBuf::from(std::env::var_os("RS2B0T").expect("set absolute RS2B0T for catalog checks"));
    assert!(root.is_absolute(), "RS2B0T must be an absolute source path");
    root.canonicalize().expect("RS2B0T source directory")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-jive-catalog-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        name
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(encode_snapshot(input));
}

fn loaded_stats(skill: &'static str, xp: i32, base: i32) -> Vec<StatInput<'static>> {
    common::FRESH_STATS
        .iter()
        .map(|row| {
            if row.name == skill {
                StatInput {
                    index: row.index,
                    name: row.name,
                    xp,
                    base,
                    effective: base,
                }
            } else {
                *row
            }
        })
        .collect()
}

fn jive_snapshot<'a>(
    stats: &'a [StatInput<'a>],
    booth: Option<NearestBoothInput<'a>>,
) -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3269,
            z: 3167,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats,
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
        nearest_booth: booth,
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

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn card_path(root: &Path, folder: &str, file: &str) -> PathBuf {
    root.join("src/bot/scripts").join(folder).join(file)
}

#[test]
#[ignore = "requires an external rs2b0t checkout via absolute RS2B0T"]
fn first_unloadable_jive_crafting_and_enchanter_none() {
    let root = frozen_root();
    let crafting = card_path(&root, "JiveCrafting", "JiveCrafting.ts");
    let origin = std::fs::read_to_string(&crafting).expect("JiveCrafting.ts");
    assert_eq!(
        first_unloadable_for_card(&origin, &crafting),
        None,
        "JiveCrafting leftover after jive remap"
    );

    let enchanter = card_path(&root, "JiveEnchanter", "JiveEnchanter.ts");
    let origin = std::fs::read_to_string(&enchanter).expect("JiveEnchanter.ts");
    assert_eq!(
        first_unloadable_for_card(&origin, &enchanter),
        None,
        "JiveEnchanter leftover after jive remap"
    );
}

/// The four hunt cards load as full modules on the hunting name maps
/// (every named import links) and tick without a load or loop error.
#[test]
#[ignore = "requires an external rs2b0t checkout via absolute RS2B0T"]
fn jive_hunt_cards_full_module_load_and_tick() {
    for name in ["JiveDragons", "JiveDemons", "JiveKBD", "JiveChests"] {
        let (iso, bag) = spawn_frozen_card(name);
        iso.post_settings_bag(&bag);
        let stats = [StatInput {
            index: 6,
            name: "magic",
            xp: 0,
            base: 40,
            effective: 40,
        }];
        post_snapshot_input(&iso, &jive_snapshot(&stats, None));
        tick(&iso, 1);
        tick(&iso, 2);
        let logs = iso.drain_logs();
        let last_error = iso.probe("globalThis.__rs2b0t_host.lastError").ok();
        iso.join();
        assert!(
            !logs
                .iter()
                .any(|l| l.contains("SyntaxError") || l.contains("does not provide an export")),
            "{name}: {logs:?}"
        );
        assert!(
            last_error.as_ref().is_none_or(|e| e.is_null()),
            "{name} loop error: {last_error:?} logs={logs:?}"
        );
    }
}

fn spawn_frozen_card(name: &str) -> (LoadIsolate, serde_json::Map<String, serde_json::Value>) {
    let root = frozen_root();
    let dir = scratch(name);
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("register frozen catalog");
    lib.ensure_js(ScriptSource::Catalog, name)
        .unwrap_or_else(|e| panic!("{name} transpile: {e}"));
    let card = lib
        .get(ScriptSource::Catalog, name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} listed"));
    assert_eq!(
        card.unloadable, None,
        "{name} first_unloadable after register"
    );
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        &JsCache::new(dir.join("sib-cache")),
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
            shape: Some(format!("{:?}", card.shape)),
            api_family: None,
        },
    )
    .unwrap_or_else(|e| panic!("{name} siblings: {e}"));
    let bag = script::merge_bag(&card.settings_schema, &serde_json::Map::new(), None);
    let iso = LoadIsolate::spawn_with_game_data(
        card.js,
        card.shape,
        siblings,
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .unwrap_or_else(|e| panic!("{name} FULL module load: {e}"));
    (iso, bag)
}

#[test]
#[ignore = "requires an external rs2b0t checkout via absolute RS2B0T"]
fn jive_crafting_full_module_onpaint_uses_real_settings_and_select() {
    let (iso, bag) = spawn_frozen_card("JiveCrafting");
    assert_eq!(
        bag.get("product").and_then(|v| v.as_str()),
        Some("Sapphire ring"),
        "schema default, not a test-invented product"
    );
    iso.post_settings_bag(&bag);
    let stats = loaded_stats("crafting", 0, 40);
    let booth = NearestBoothInput {
        x: 3269,
        z: 3167,
        level: 0,
        id: 1,
        name: "Bank booth",
        op: "Use-quickly",
    };
    post_snapshot_input(&iso, &jive_snapshot(&stats, Some(booth)));
    tick(&iso, 1);
    tick(&iso, 2);
    let logs = iso.drain_logs();
    let paint = iso.paint().unwrap_or_else(|| {
        panic!("JiveCrafting onPaint must record a frame; logs={logs:?}");
    });
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("Runtime") && l.contains("Made")),
        "default Statistics/Overview: {lines:?} logs={logs:?}",
        lines = paint.lines
    );
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("Craft") || l.contains("no experience yet")),
        "paintLevels from progress(): {:?}",
        paint.lines
    );

    iso.paint_select("strip:jive:JiveCrafting", "Options");
    tick(&iso, 3);
    let options = iso.paint().expect("options");
    assert!(
        options
            .lines
            .iter()
            .any(|l| l.contains("Product: Sapphire ring")),
        "Options uses resolved settings jewel: {:?}",
        options.lines
    );
    assert!(!options.lines.iter().any(|l| l.contains("Made:")));
    assert!(
        options.rail.is_none(),
        "Options skips rail: {:?}",
        options.rail
    );

    iso.paint_select("strip:jive:JiveCrafting", "Statistics");
    iso.paint_select("rail:jive:JiveCrafting", "Supplies");
    tick(&iso, 4);
    let supplies = iso.paint().expect("supplies");
    assert!(
        supplies.lines.iter().any(|l| l.contains("Bars:")),
        "Supplies bars: {:?}",
        supplies.lines
    );
    assert!(
        supplies.lines.iter().any(|l| l.contains("Mould:")),
        "Supplies mould: {:?}",
        supplies.lines
    );
    assert!(!supplies.lines.iter().any(|l| l.contains("Made:")));
    iso.join();
}

#[test]
#[ignore = "requires an external rs2b0t checkout via absolute RS2B0T"]
fn jive_enchanter_full_module_onpaint_uses_real_settings_and_select() {
    let (iso, bag) = spawn_frozen_card("JiveEnchanter");
    assert_eq!(
        bag.get("jewel").and_then(|v| v.as_str()),
        Some("Sapphire ring"),
        "schema default, not a test-invented jewel"
    );
    iso.post_settings_bag(&bag);
    let stats = loaded_stats("magic", 0, 40);
    let booth = NearestBoothInput {
        x: 3269,
        z: 3167,
        level: 0,
        id: 1,
        name: "Al Kharid",
        op: "Use-quickly",
    };
    post_snapshot_input(&iso, &jive_snapshot(&stats, Some(booth)));
    tick(&iso, 1);
    tick(&iso, 2);
    let logs = iso.drain_logs();
    let paint = iso.paint().unwrap_or_else(|| {
        panic!("JiveEnchanter onPaint must record a frame; logs={logs:?}");
    });
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("Runtime") && l.contains("Enchanted")),
        "default Statistics/Overview: {lines:?} logs={logs:?}",
        lines = paint.lines
    );

    iso.paint_select("strip:jive:JiveEnchanter", "Options");
    tick(&iso, 3);
    let options = iso.paint().expect("options");
    assert!(
        options
            .lines
            .iter()
            .any(|l| l.contains("Jewel: Sapphire ring")),
        "Options uses resolved settings jewel: {:?}",
        options.lines
    );
    assert!(!options.lines.iter().any(|l| l.contains("Enchanted:")));
    assert!(options.rail.is_none());

    iso.paint_select("strip:jive:JiveEnchanter", "Statistics");
    iso.paint_select("rail:jive:JiveEnchanter", "Supplies");
    tick(&iso, 4);
    let supplies = iso.paint().expect("supplies");
    assert!(
        supplies
            .lines
            .iter()
            .any(|l| l.contains("Product: Ring of recoil")),
        "Supplies product from sibling jewel table: {:?}",
        supplies.lines
    );
    assert!(!supplies.lines.iter().any(|l| l.contains("Enchanted:")));
    iso.join();
}
