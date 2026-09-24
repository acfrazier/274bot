// Bright catalog Start hammer: spawn every loadable card, dump every
// `not impl` / load error in one failure. Skip when `$RS2B0T` is absent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use script::load::{JsLibrary, LoadIsolate, LoadShape};
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-catalog-start-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn locked_unloadable(spec: &str) -> bool {
    spec.contains("WalkExecutor.js")
        || spec.contains("event/webwalk/Navigator.js")
        || spec.contains("ToolAcquire.js")
        || spec.contains("/defs/")
        || spec.contains("barcrawl/")
}

/// A fresh account's stat rows (client stat index, name): an in-game
/// client always posts them, and a card's first `onPaint` — logged on its
/// own tick — reads them.
const SKILLS: [(i32, &str); 19] = [
    (0, "attack"),
    (1, "defence"),
    (2, "strength"),
    (3, "hitpoints"),
    (4, "ranged"),
    (5, "prayer"),
    (6, "magic"),
    (7, "cooking"),
    (8, "woodcutting"),
    (9, "fletching"),
    (10, "fishing"),
    (11, "firemaking"),
    (12, "crafting"),
    (13, "smithing"),
    (14, "mining"),
    (15, "herblore"),
    (16, "agility"),
    (17, "thieving"),
    (20, "runecraft"),
];

fn fresh_stats() -> Vec<script::isolate_fb::StatInput<'static>> {
    SKILLS
        .iter()
        .map(|&(index, name)| {
            let (xp, level) = if name == "hitpoints" {
                (1154, 10)
            } else {
                (0, 1)
            };
            script::isolate_fb::StatInput {
                index,
                name,
                xp,
                base: level,
                effective: level,
            }
        })
        .collect()
}

/// Full special energy: the host posts every nonzero varp.
const VARPS: [script::isolate_fb::VarpInput; 1] = [script::isolate_fb::VarpInput {
    index: 300,
    value: 1000,
}];

fn empty_snap() -> script::isolate_fb::SnapshotInput<'static> {
    script::isolate_fb::SnapshotInput {
        tick: 1,
        here: Some(script::isolate_fb::TileInput {
            x: 3222,
            z: 3222,
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
        side_tab: -1,
        varps: &VARPS,
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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

fn throw_shaped(line: &str) -> bool {
    line.contains("not impl")
        || line.contains("Error")
        || (line.starts_with("tick ") && line.contains(':'))
}

#[test]
fn bright_catalog_cards_start_without_not_impl() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let dir = scratch("gold");
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");

    let names: Vec<String> = lib
        .cards()
        .iter()
        .filter(|c| {
            c.source == ScriptSource::Catalog
                && !script::is_catalog_dim(&c.name)
                && c.unloadable
                    .as_deref()
                    .is_none_or(|u| !locked_unloadable(u))
                && c.unloadable.is_none()
        })
        .map(|c| c.name.clone())
        .collect();

    let mut hits: BTreeSet<String> = BTreeSet::new();
    let cache = JsCache::new(dir.join("sib-cache"));
    let stats = fresh_stats();
    let mut snap = empty_snap();
    snap.stats = &stats;
    let snap = script::isolate_fb::encode_snapshot(&snap);
    for name in &names {
        if let Err(e) = lib.ensure_js(ScriptSource::Catalog, name) {
            hits.insert(format!("{name}: transpile {e}"));
            continue;
        }
        let Some(card) = lib.get(ScriptSource::Catalog, name).cloned() else {
            hits.insert(format!("{name}: missing after ensure_js"));
            continue;
        };
        if card.shape == LoadShape::Reject {
            hits.insert(format!("{name}: reject shape"));
            continue;
        }
        let siblings = match script::resolve_sibling_modules(
            &card.path,
            &card.origin,
            &cache,
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
                shape: Some(format!("{:?}", card.shape)),
                api_family: None,
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                hits.insert(format!("{name}: siblings {e}"));
                continue;
            }
        };
        let bag = script::merge_bag(&card.settings_schema, &serde_json::Map::new(), None);
        match LoadIsolate::spawn_with_game_data(
            card.js.clone(),
            card.shape,
            siblings,
            api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
        ) {
            Err(e) => {
                hits.insert(format!("{name}: load {e}"));
            }
            Ok(iso) => {
                if !bag.is_empty() {
                    iso.post_settings_bag(&bag);
                }
                iso.post_snapshot(snap.clone());
                iso.on_game_tick(1);
                let _ = iso.probe("__rs_bot");
                for line in iso.drain_logs() {
                    if throw_shaped(&line) {
                        hits.insert(format!("{name}: {line}"));
                    }
                }
                iso.join();
            }
        }
    }

    assert!(
        !names.is_empty(),
        "catalog must yield bright cards when $RS2B0T is set"
    );
    assert!(
        hits.is_empty(),
        "bright catalog Start threw ({} cards, {} hits):\n{}",
        names.len(),
        hits.len(),
        hits.iter().cloned().collect::<Vec<_>>().join("\n")
    );
}
