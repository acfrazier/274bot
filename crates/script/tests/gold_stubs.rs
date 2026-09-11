//! Task 10: gold-script stubs, data tables, and sibling logic imports.

use std::path::PathBuf;

use script::load::{JsLibrary, LoadIsolate, LoadShape};
use script::{CacheMeta, JsCache, ScriptKind, ScriptSource};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-gold-stubs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// ChickenKiller-shaped bot: SETTINGS references COMBAT_STYLE_OPTIONS and
/// SPELL_DB keys; TaskBot adds DeathRecovery so Start wiring resolves.
const CHICKEN_SHAPED: &str = r#"
import { TaskBot } from '../../api/bot/Bot.js';
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
import { COMBAT_STYLE_OPTIONS } from '../../api/combat/CombatStyle.js';
import { SPELL_DB } from '../../data/spelldb.js';
import { PERIODIC_BANK_SETTINGS } from '../../api/bank/Banking.js';

export const SETTINGS = {
    meleeStyle: {
        type: 'string',
        default: 'strength',
        options: COMBAT_STYLE_OPTIONS,
        label: 'Melee style',
    },
    spell: {
        type: 'string',
        default: 'Wind Strike',
        options: Object.keys(SPELL_DB),
        label: 'Autocast spell',
    },
    ...PERIODIC_BANK_SETTINGS,
};

export default class ChickenShaped extends TaskBot {
    onStart() {
        this.add(new DeathRecovery(this, { anchor: { x: 3222, z: 3222, level: 0 } }));
    }
    loop() {}
}
"#;

#[test]
fn gold_catalog_import_paths_resolve_for_live_scripts() {
    let probes = [
        (
            "Autocast",
            r#"
import { Autocast } from '../../api/magic/Autocast.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = typeof Autocast.armed; }
}
"#,
        ),
        (
            "food",
            r#"
import { foodHealAmount } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = typeof foodHealAmount; }
}
"#,
        ),
        (
            "itemdb",
            r#"
import { ITEM_DB } from '../../data/itemdb.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = ITEM_DB.length; }
}
"#,
        ),
        (
            "CombatStyleLogic",
            r#"
import { castsAvailable } from '../../api/combat/CombatStyleLogic.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = typeof castsAvailable; }
}
"#,
        ),
    ];
    for (name, src) in probes {
        let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
        iso.on_game_tick(1);
        let value = iso.probe("__probe").unwrap();
        assert!(
            value != serde_json::Value::Null,
            "{name} import probe must resolve, got {value:?}"
        );
        let logs = iso.drain_logs();
        assert!(
            logs.iter()
                .all(|l| !l.contains("404") && !l.contains("Module not found")),
            "{name} import must not 404: {logs:?}"
        );
        iso.join();
    }
}

#[test]
fn file_load_parses_export_settings_without_a_throwaway_runtime() {
    let dir = temp_dir("chicken-shaped");
    let path = dir.join("ChickenShaped.ts");
    std::fs::write(&path, CHICKEN_SHAPED).unwrap();
    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    let card = lib
        .load(&path)
        .expect("ChickenKiller-shaped SETTINGS is origin parse, not a V8 compile");
    assert!(
        card.settings_schema.iter().any(|d| d.id == "meleeStyle"),
        "File Load must parse export const SETTINGS: {:?}",
        card.settings_schema
    );
    assert!(
        card.settings_schema.iter().any(|d| d.id == "spell"),
        "spell field must survive identifier-valued sibling keys: {:?}",
        card.settings_schema
    );
}

#[test]
fn death_recovery_import_does_not_404() {
    let src = r#"
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = typeof DeathRecovery;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(
        value, "function",
        "DeathRecovery must resolve as a loadable class"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .all(|l| !l.contains("404") && !l.contains("Module not found")),
        "DeathRecovery import must not 404: {logs:?}"
    );
    iso.join();
}

#[test]
fn solve_clue_construct_is_idle_not_impl() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        this.add(new SolveClue({}));
        globalThis.__probe = this._tasks[0].clueStatus();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("solveClues".into(), serde_json::json!(false));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    let status = iso.probe("__probe").unwrap();
    assert_eq!(
        status, "idle",
        "new SolveClue() must not throw when clue drops are off: {status:?}"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("not impl")),
        "SolveClue must not fire not impl with solveClues false: {logs:?}"
    );
    iso.join();
}

#[test]
fn quest_engine_throws_not_impl_on_use() {
    let src = r#"
import { QuestEngine } from '../../api/ai/quests/engine/QuestEngine.js';
export default class T extends LoopingBot {
    loop() {
        try {
            QuestEngine.start();
        } catch (e) {
            globalThis.__probe = String(e.message || e);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let msg = iso.probe("__probe").unwrap();
    assert!(
        msg.as_str().unwrap_or("").contains("not impl"),
        "QuestEngine use must throw not impl, got {msg:?}"
    );
    iso.join();
}

const ALCHER_MAIN: &str = r#"
import { ALCH_OPTIONS } from './AlcherLogic.js';
export default class Alcher extends LoopingBot {
    loop() { globalThis.__alch_opts = ALCH_OPTIONS.length; }
}
"#;

const ALCHER_LOGIC_TS: &str = r#"
export const ALCH_OPTIONS = ['Maple longbow', 'Yew longbow'];
"#;

#[test]
fn alcher_logic_sibling_is_sha_cached_at_start() {
    let dir = temp_dir("alcher-logic");
    let cache_root = dir.join("js-cache");
    let cache = JsCache::new(cache_root.clone());
    let card_dir = dir.join("Alcher");
    std::fs::create_dir_all(&card_dir).unwrap();
    let main_path = card_dir.join("Alcher.ts");
    std::fs::write(&main_path, ALCHER_MAIN).unwrap();
    std::fs::write(card_dir.join("AlcherLogic.ts"), ALCHER_LOGIC_TS).unwrap();

    let siblings = script::resolve_sibling_modules(
        &main_path,
        ALCHER_MAIN,
        &cache,
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
            shape: Some("CompatClass".into()),
        },
    )
    .expect("AlcherLogic sibling resolves");

    assert_eq!(siblings.len(), 1);
    assert_eq!(siblings[0].0, "/rs2b0t/bot/scripts/bot/AlcherLogic.js");
    assert!(
        siblings[0].1.contains("ALCH_OPTIONS"),
        "cached AlcherLogic.js must export ALCH_OPTIONS"
    );

    let iso = LoadIsolate::spawn(
        std::fs::read_to_string(&main_path).unwrap(),
        LoadShape::CompatClass,
        siblings,
    )
    .unwrap();
    iso.on_game_tick(1);
    let n = iso.probe("__alch_opts").unwrap();
    assert_eq!(n, 2, "AlcherLogic sibling must load at Start");
    iso.join();
}

/// Thiever onStart reads `scriptFood` + `autoFoodBanking` after the
/// ingame gate; the steal loop then uses `canStealNow` / `countFood`.
/// A one-tick catalog Start parks on `delayUntil(..., 0)` and misses these.
#[test]
fn thiever_start_helpers_read_posted_bag_without_not_impl() {
    let src = r#"
import { scriptFood } from '../../api/loadout/loadoutPlan.js';
import {
    autoFoodBanking,
    canStealNow,
    countFood,
    foodMatches,
    safeToSteal,
    shouldRestockFood,
} from '../../api/thieving/stealRules.js';
export default class T extends LoopingBot {
    onStart() {
        this.food = scriptFood(this.settings, '').toLowerCase();
        this.autoBank = autoFoodBanking(this.settings.str('banking', 'None'));
    }
    loop() {
        const n = countFood([{ name: 'Lobster', count: 10 }], this.food);
        globalThis.__probe = {
            food: this.food,
            auto: this.autoBank,
            steal: canStealNow(0, 50, 5, false),
            match: foodMatches('Lobster', this.food),
            restock: shouldRestockFood(this.autoBank, n, 0, false),
            safe: safeToSteal(1, 0.4, n),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("banking".into(), serde_json::json!("None"));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("not impl")),
        "Thiever Start helpers must not throw not impl: {logs:?}"
    );
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["food"], "", "no loadout → scriptFood fallback");
    assert_eq!(probe["auto"], false, "banking None is not Auto");
    assert_eq!(probe["steal"], true, "HP above minEatHp can steal");
    assert_eq!(probe["match"], false, "empty food keyword matches nothing");
    assert_eq!(probe["restock"], false, "banking off does not restock");
    assert_eq!(probe["safe"], true, "full HP is safe to steal");
    iso.join();
}

#[test]
fn item_db_reads_host_content_alcher_gold_row() {
    let src = r#"
import { ITEM_DB } from '../../data/itemdb.js';
export default class T extends LoopingBot {
    loop() {
        const row = ITEM_DB.find((r) => r && r.obj === 'rune_chainbody');
        globalThis.__probe = row ? { id: row.id, name: row.name, cost: row.cost } : null;
    }
}
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .unwrap();
    iso.on_game_tick(1);
    let row = iso.probe("__probe").unwrap();
    assert_eq!(row["id"], 1113, "ITEM_DB rune_chainbody id");
    assert_eq!(row["name"], "Rune chainbody");
    iso.join();
}

#[test]
fn selected_game_data_is_published_before_modules_and_offline_stays_empty() {
    let source = r#"
import { ITEM_DB } from '../../data/itemdb.js';
import { foodHealAmount } from '../../api/combat/food.js';
import { requiredThieving } from '../../api/thieving/targets.js';
export default class T extends LoopingBot {
    loop() {
        let unknownFood = null;
        let ambiguousFood = null;
        try { foodHealAmount('Not a food'); } catch (e) { unknownFood = String(e.message || e); }
        try { foodHealAmount('Cabbage'); } catch (e) { ambiguousFood = String(e.message || e); }
        globalThis.__probe = {
            plate: ITEM_DB.find(r => r.obj === 'rune_platebody') || null,
            castlewars: ITEM_DB.find(r => r.obj === 'castlewars_armour_body') || null,
            dragonhide: ITEM_DB.filter(r => r.name === 'Dragonhide').map(r => [r.obj, r.id]),
            bread: foodHealAmount('Bread'),
            anchovies: foodHealAmount('Anchovies'),
            guard: requiredThieving('Guard'),
            unknownFood,
            ambiguousFood,
        };
    }
}
"#;

    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(source.into(), LoadShape::CompatClass, vec![], data)
            .unwrap();
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["plate"]["name"], "Rune platebody");
    assert_eq!(probe["castlewars"]["name"], "Decorative armour");
    assert!(probe["dragonhide"].as_array().unwrap().len() >= 8);
    assert_eq!(probe["bread"], 4);
    assert_eq!(probe["anchovies"], 3);
    assert_eq!(probe["guard"], 40);
    assert!(probe["unknownFood"].as_str().unwrap().contains("not impl"));
    assert!(probe["ambiguousFood"]
        .as_str()
        .unwrap()
        .contains("not impl"));
    iso.join();

    let offline = LoadIsolate::spawn(
        r#"
import { ITEM_DB } from '../../data/itemdb.js';
import { foodHealAmount } from '../../api/combat/food.js';
import { requiredThieving } from '../../api/thieving/targets.js';
export default class T extends LoopingBot {
    loop() {
        let food = null;
        let target = null;
        try { foodHealAmount('Lobster'); } catch (e) { food = String(e.message || e); }
        try { requiredThieving('Guard'); } catch (e) { target = String(e.message || e); }
        globalThis.__probe = { items: ITEM_DB.length, food, target };
    }
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    offline.on_game_tick(1);
    let probe = offline.probe("__probe").unwrap();
    assert_eq!(probe["items"], 0);
    assert!(probe["food"].as_str().unwrap().contains("not impl"));
    assert!(probe["target"].as_str().unwrap().contains("not impl"));
    offline.join();
}

#[test]
#[ignore = "requires RS2B0T to name a frozen catalog root"]
fn selected_game_data_composes_with_frozen_alcher_logic() {
    let root = PathBuf::from(std::env::var("RS2B0T").expect("RS2B0T frozen root"))
        .canonicalize()
        .expect("canonical frozen root");
    let alcher_dir = root.join("src/bot/scripts/Alcher");
    let probe_path = alcher_dir.join("SelectedDataProbe.ts");
    let source = r#"
import { ALCH_ITEMS, ALCH_OPTIONS, customAlchItem, selectedAlchItems } from './AlcherLogic.js';
export default class T extends LoopingBot {
    loop() {
        const selected = selectedAlchItems(['rune_platebody', 'rune_chainbody']);
        const bodies = ALCH_ITEMS.filter(i => i.key.endsWith('dragonhide_body'));
        globalThis.__probe = {
            options: ALCH_OPTIONS.length,
            allItems: ALCH_ITEMS.length,
            selected: selected.map(i => i.key),
            customAlias: customAlchItem('adamant_scimitar'),
            customName: customAlchItem('Adamant scimitar'),
            bodyIds: bodies.map(i => i.id),
            unknown: customAlchItem('not_a_real_selected_item'),
        };
    }
}
"#;
    let cache = JsCache::new(temp_dir("selected-alcher").join("js-cache"));
    let siblings = script::resolve_sibling_modules(
        &probe_path,
        source,
        &cache,
        CacheMeta {
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
            shape: Some("CompatClass".into()),
        },
    )
    .expect("frozen AlcherLogic resolves");
    for revision in [
        client::io::ClientRevision::R274,
        client::io::ClientRevision::R289,
    ] {
        let data = api::game_data::for_revision(revision).unwrap();
        let iso = LoadIsolate::spawn_with_game_data(
            source.into(),
            LoadShape::CompatClass,
            siblings.clone(),
            data,
        )
        .unwrap();
        iso.on_game_tick(1);
        let probe = iso.probe("__probe").unwrap();
        assert_eq!(probe["options"], 37, "custom plus every enabled fodder");
        assert_eq!(probe["allItems"], 36);
        assert_eq!(
            probe["selected"],
            serde_json::json!(["rune_platebody", "rune_chainbody"])
        );
        assert_eq!(probe["customAlias"]["id"], 1331);
        assert_eq!(probe["customName"]["id"], 1331);
        let body_ids = probe["bodyIds"].as_array().unwrap();
        assert_eq!(body_ids.len(), 4);
        assert_eq!(
            body_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
        assert!(probe["unknown"].is_null());
        iso.join();
    }
}

#[test]
fn thiever_resolves_food_from_host_loadout_and_queues_eat() {
    let src = r#"
import { scriptFood } from '../../api/loadout/loadoutPlan.js';
import { foodHealAmount, shouldEatFood } from '../../api/combat/food.js';
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    loop() {
        const food = scriptFood(this.settings, '');
        globalThis.__food = [food, foodHealAmount(food)];
        if (shouldEatFood(food, {hp: 30, maxHp: 50, foodCount: 1})) Inventory.items()[0].interact('Eat');
    }
}
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::CompatClass,
        vec![],
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .unwrap();
    iso.post_loadouts(&[script::Loadout::new("Food")
        .with_carry("Coins", 1)
        .with_carry("Lobster", 1)]);
    iso.probe("globalThis.__rs2b0t_host.snapshot = {inv: [{name:'Lobster',count:1}]}; true")
        .unwrap();
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__food").unwrap(),
        serde_json::json!(["Lobster", 12])
    );
    assert!(iso.drain_interacts().iter().any(|r| matches!(r,script::shim::InteractReq::Held{name,action} if name=="Lobster" && action=="Eat")));
    iso.join();
}

#[test]
fn bank_access_already_adjacent_does_not_rearm_navigation() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() { await Bank.openNearestAccess({name:'Bank booth',op:'Use-quickly'}); }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.probe("globalThis.__rs2b0t_host.snapshot={here:{x:10,z:10,level:0},nearest_booth:{x:11,z:10,level:0,id:2213},bank_open:false,bank_loaded:false};true").unwrap();
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    let requests = iso.drain_interacts();
    assert_eq!(requests.len(), 1);
    assert!(matches!(
        requests[0],
        script::shim::InteractReq::OpenBooth { .. }
    ));
    iso.join();
}

#[test]
fn withdraw_x_waits_for_inventory_publication() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() { globalThis.__withdrawResult = await Bank.withdrawX('Lobster', 19); }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.probe("globalThis.__rs2b0t_host.snapshot={bank:[{id:377,name:'Lobster',count:2000,ops:['Withdraw X']}],bank_open:true,bank_loaded:true,bank_generation:1,count_dialog_open:false,inv_size:28,inv:[{name:'Lobster',count:3}]};true").unwrap();
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    iso.probe("globalThis.__rs2b0t_host.snapshot.count_dialog_open=true;true")
        .unwrap();
    for tick in 2..=3 {
        iso.on_game_tick(tick);
        iso.probe("true").unwrap();
    }
    assert_eq!(
        iso.probe("typeof globalThis.__withdrawResult").unwrap(),
        "undefined",
        "a sent count is not a completed withdrawal"
    );
    iso.probe("globalThis.__rs2b0t_host.snapshot.inv=[{name:'Lobster',count:22}];globalThis.__rs2b0t_host.snapshot.withdraw_x_result_seq=1;globalThis.__rs2b0t_host.snapshot.withdraw_x_result=true;true")
        .unwrap();
    iso.on_game_tick(4);
    assert_eq!(iso.probe("globalThis.__withdrawResult").unwrap(), true);
    iso.join();
}
