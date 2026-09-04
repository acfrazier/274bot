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
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                hits.insert(format!("{name}: siblings {e}"));
                continue;
            }
        };
        let bag = script::merge_bag(&card.settings_schema, &serde_json::Map::new(), None);
        match LoadIsolate::spawn(card.js.clone(), card.shape, siblings) {
            Err(e) => {
                hits.insert(format!("{name}: load {e}"));
            }
            Ok(iso) => {
                if !bag.is_empty() {
                    iso.post_settings_bag(&bag);
                }
                iso.post_snapshot(script::isolate_fb::encode_snapshot(&empty_snap()));
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
