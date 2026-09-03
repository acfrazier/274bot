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
fn quest_engine_throws_not_v1_on_use() {
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
        msg.as_str().unwrap_or("").contains("not v1"),
        "QuestEngine use must throw not v1, got {msg:?}"
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
