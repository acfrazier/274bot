//! Unloadable catalog cards: import specifiers that do not remap to a shim.

#[test]
fn webwalk_something_is_unloadable() {
    let src = "import x from '../../event/webwalk/Something.js'; export default class T extends LoopingBot { loop() {} }";
    assert_eq!(
        script::first_unloadable_specifier(src).as_deref(),
        Some("../../event/webwalk/Something.js")
    );
}

#[test]
fn boneburier_shaped_imports_are_loadable() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn chicken_killer_import_lines_are_loadable() {
    let src = r#"
import { TaskBot, type Task } from '../../api/bot/Bot.js';
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
import Tile from '../../geometry/Tile.js';
import { ContinueDialog } from '../../api/tasks/ContinueDialog.js';
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
import { PERIODIC_BANK_SETTINGS, depositAllExcept, parseBankStrategy, type BankDestination } from '../../api/bank/Banking.js';
import { Autocast } from '../../api/magic/Autocast.js';
import { castsAvailable, runeWithdrawList } from '../../api/combat/CombatStyleLogic.js';
import { SPELL_DB } from '../../data/spelldb.js';
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
import { GroundItems } from '../../api/grounditems/GroundItems.js';
import { Npcs, type Npc } from '../../api/npcs/Npcs.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Equipment } from '../../api/equipment/Equipment.js';
import { Bank } from '../../api/bank/Bank.js';
import { Skills } from '../../api/skills/Skills.js';
import { Paint } from '../../paint/Paint.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { CANT_REACH, GameMessages } from '../../api/chatbox/gameMessages.js';
import { RecoveryHints } from '../../runtime/RecoveryHints.js';
import type { SettingsSchema } from '../../runtime/Settings.js';
import { fmtDuration } from '../../paint/paintLogic.js';
import { Reach } from '../../api/walking/Reach.js';
import { CombatStyle } from '../../api/combat/CombatStyle.js';
"#;
    assert_eq!(
        script::first_unloadable_specifier(src),
        None,
        "ChickenKiller gold imports must stay loadable"
    );
}

#[test]
fn catalog_dim_names_are_locked() {
    for name in [
        "AIOQuester",
        "ClueSolver",
        "Woodcutter",
        "Miner",
        "Fisher",
        "ArravSupplier",
        "Barcrawl",
        "RoguesPurse",
        "MarketMaker",
    ] {
        assert!(script::is_catalog_dim(name), "{name}");
    }
    assert!(!script::is_catalog_dim("CookBot"));
    assert!(!script::is_catalog_dim("ChickenKiller"));
    assert!(!script::is_catalog_dim("WalkTo"));
    assert!(script::is_reserved("WalkTo"));
}

#[test]
fn catalog_dim_register_stamps_unloadable_even_when_imports_remap() {
    use script::load::JsLibrary;
    use script::ScriptSource;

    let dir = std::env::temp_dir().join(format!(
        "274bot-catalog-dim-{}-{}",
        std::process::id(),
        "stamp"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts/Woodcutter");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        root.join("src/bot/scripts/index.ts"),
        r#"
import Woodcutter from './Woodcutter/Woodcutter.js';
import CookBot from './CookBot/CookBot.js';
import BankSorter from './BankSorter/BankSorter.js';
ScriptRegistry.register({ name: 'Woodcutter', create: () => new Woodcutter() });
ScriptRegistry.register({ name: 'CookBot', create: () => new CookBot() });
ScriptRegistry.register({ name: 'BankSorter', create: () => new BankSorter() });
"#,
    )
    .unwrap();
    let body = "import { Game } from '../../api/game/Game.js'; export default class T extends LoopingBot { loop() {} }";
    std::fs::write(scripts.join("Woodcutter.ts"), body).unwrap();
    std::fs::create_dir_all(root.join("src/bot/scripts/CookBot")).unwrap();
    std::fs::write(root.join("src/bot/scripts/CookBot/CookBot.ts"), body).unwrap();
    std::fs::create_dir_all(root.join("src/bot/scripts/BankSorter")).unwrap();
    std::fs::write(root.join("src/bot/scripts/BankSorter/BankSorter.ts"), body).unwrap();

    let mut lib = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("registry fills");
    let wood = lib
        .get(ScriptSource::Catalog, "Woodcutter")
        .expect("Woodcutter listed");
    assert_eq!(wood.unloadable.as_deref(), Some("dim: Woodcutter"));
    let cook = lib
        .get(ScriptSource::Catalog, "CookBot")
        .expect("CookBot listed");
    assert_eq!(cook.unloadable, None, "CookBot is not name-locked");
    let sorter = lib
        .get(ScriptSource::Catalog, "BankSorter")
        .expect("BankSorter remains listed with its availability reason");
    assert_eq!(
        sorter.unloadable.as_deref(),
        Some("dim: BankSorter is unavailable until native bank sorting is implemented")
    );
}

#[test]
fn catalog_flour_collector_links_murder_data_and_dims_ess_miner() {
    use script::load::JsLibrary;
    use script::ScriptSource;

    let dir = std::env::temp_dir().join(format!(
        "274bot-catalog-loads-{}-{}",
        std::process::id(),
        "murder"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("FlourCollector")).unwrap();
    std::fs::create_dir_all(scripts.join("EssMiner")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import FlourCollector from './FlourCollector/FlourCollector.js';
import EssMiner from './EssMiner/EssMiner.js';
ScriptRegistry.register({ name: 'FlourCollector', create: () => new FlourCollector() });
ScriptRegistry.register({ name: 'EssMiner', create: () => new EssMiner() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("FlourCollector/FlourCollector.ts"),
        r#"
import { MURDER_LOC, MURDER_NAME, MURDER_OBJ, MURDER_TILE } from '../../api/ai/quests/defs/murder/areas.js';
export default class FlourCollector extends LoopingBot {
    loop() {
        globalThis.__murder = [MURDER_NAME, MURDER_OBJ.POT, MURDER_LOC.FLOUR_BARREL, MURDER_TILE.BANK.x];
    }
}
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("EssMiner/EssMiner.ts"),
        r#"
import { ToolAcquire } from '../../api/acquisition/ToolAcquire.js';
export default class EssMiner extends LoopingBot { loop() { ToolAcquire; } }
"#,
    )
    .unwrap();

    let mut library = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    library
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("fixture catalog registers");
    let flour = library
        .get(ScriptSource::Catalog, "FlourCollector")
        .expect("FlourCollector listed");
    assert_eq!(flour.unloadable, None);
    let prepared = library
        .prepare_card(ScriptSource::Catalog, "FlourCollector")
        .expect("FlourCollector transpiles and instantiates");
    assert_eq!(prepared.card.unloadable, None);

    let ess = library
        .get(ScriptSource::Catalog, "EssMiner")
        .expect("EssMiner listed");
    assert_eq!(
        ess.unloadable.as_deref(),
        Some("dim: EssMiner is unavailable until the native Gatherer replaces it")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pinned_flour_collector_transpiles_and_instantiates() {
    use script::load::JsLibrary;
    use script::ScriptSource;

    let Some(root) = script::rs2b0t_root() else {
        eprintln!("skip pinned FlourCollector proof: RS2B0T is not configured");
        return;
    };
    let dir = std::env::temp_dir().join(format!("274bot-pinned-flour-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut library = JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    library
        .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("pinned catalog registers");
    let card = library
        .get(ScriptSource::Catalog, "FlourCollector")
        .expect("pinned FlourCollector listed");
    assert_eq!(card.unloadable, None);
    library
        .prepare_card(ScriptSource::Catalog, "FlourCollector")
        .expect("pinned FlourCollector transpiles and instantiates");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reach_loc_op_fails_closed_through_proxy() {
    use script::{LoadIsolate, LoadShape};

    let src = r#"
import { Reach } from '../../api/walking/Reach.js';
export default class T extends LoopingBot {
    loop() {
        try {
            Reach.locOp({});
            globalThis.__probe = 'unexpected success';
        } catch (error) {
            globalThis.__probe = String(error);
        }
    }
}
"#;
    let isolate = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![])
        .expect("Reach.locOp probe loads");
    isolate.on_game_tick(1);
    assert_eq!(
        isolate.probe("__probe").expect("Reach.locOp probe"),
        "Error: not impl: Reach.locOp"
    );
    isolate.join();
}
#[test]
fn event_webwalk_direct_navigator_remaps() {
    let src = "import { DirectNavigator } from '../../event/webwalk/DirectNavigator.js'; export default class T extends LoopingBot { loop() {} }";
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn hash_bot_shop_remaps() {
    let src = "import { Shop } from '#/bot/api/shop/Shop.js'; export default class T extends LoopingBot { loop() {} }";
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn walk_executor_stays_unloadable() {
    let src = "import x from '../../event/webwalk/WalkExecutor.js'; export default class T extends LoopingBot { loop() {} }";
    assert_eq!(
        script::first_unloadable_specifier(src).as_deref(),
        Some("../../event/webwalk/WalkExecutor.js")
    );
}

#[test]
fn webwalk_navigator_find_path_is_loadable() {
    let src = "import { Navigator } from '../../event/webwalk/Navigator.js'; export default class T extends LoopingBot { async loop() { await Navigator.findPath({x:1,z:2,level:0},{x:3,z:4,level:0},{timeoutMs:8000}); } }";
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn webwalk_worker_and_navworker_stay_unloadable() {
    for spec in [
        "../../event/webwalk/Worker.js",
        "../../event/webwalk/navworker.js",
        "../../event/webwalk/collision.lcnav.gz",
    ] {
        let src = format!(
            "import x from '{spec}'; export default class T extends LoopingBot {{ loop() {{}} }}"
        );
        assert_eq!(
            script::first_unloadable_specifier(&src).as_deref(),
            Some(spec)
        );
    }
}

#[test]
fn firemaking_and_light_fire_import_lines_remap() {
    let src = r#"
import { FIRE_SPOTS, LOG_LEVELS, lightFire } from '../../api/firemaking/Firemaking.js';
import { lightFire as light } from '../../api/firemaking/LightFire.js';
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn input_bothost_and_model_paths_remap() {
    let src = r#"
import { Input } from '../../input/Input.js';
import { BotHost } from '../../runtime/BotHost.js';
import { Npc } from '../../api/model/Npc.js';
import { Loc } from '../../api/model/Loc.js';
import { Player } from '../../api/model/Player.js';
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn autofighter_shaped_combat_imports_remap() {
    let src = r#"
import { countMatching, matchesAny } from '../../api/inventory/packRules.js';
import { Special } from '../../api/combat/Special.js';
import { swingStartedThisTick, buryOneInFight } from '../../api/combat/fightUpkeep.js';
import { shouldHoldEat } from '../../api/combat/eatTiming.js';
import { BOOST_POTIONS } from '../../api/combat/boostPotions.js';
import { combatKeepNames } from '../../api/combat/keepList.js';
import { rangeLoadoutOf } from '../../api/combat/ranged.js';
import { BOWS } from '../../api/combat/equipment.js';
import { paintClueProgress } from '../../api/ai/clues/cluePaint.js';
import { Sustain } from '../../api/sustain/Sustain.js';
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn log_from_who_is_not_an_import_specifier() {
    let src = "export default class T extends LoopingBot { loop() { this.log(`declining a trade from '${who}'`); } }";
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn type_only_imports_never_block_a_card() {
    // BrimhavenMossGiants `bank.ts`: the danger-zone type is erased by the transpile.
    let src = r#"
import { Navigator } from '../../event/webwalk/Navigator.js';
import type { DangerZoneRect } from '../../event/webwalk/data/dangerZones.js';
import { type PathPolicy } from '../../event/webwalk/types.js';
export type { WorldStateData } from '../../event/webwalk/worldStateData.js';
const ZONE: DangerZoneRect = { minX: 1, maxX: 2, minZ: 1, maxZ: 2 };
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn a_value_binding_beside_a_type_binding_still_blocks() {
    let src = r#"
import type { Task } from '../../api/bot/Bot.js';
import { type DangerZoneRect, resolveDangerZones } from '../../event/webwalk/data/dangerZones.js';
export * from '../../event/webwalk/WalkExecutor.js';
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(
        script::first_unloadable_specifier(src).as_deref(),
        Some("../../event/webwalk/data/dangerZones.js")
    );
}

#[test]
fn herb_cleaner_parent_sibling_remaps() {
    let src = "import { HERBS } from '../HerbCleaner/HerbCleanerLogic.js'; export default class T extends LoopingBot { loop() {} }";
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn herbs_data_import_remaps() {
    let src = r#"
import { HERBS, HERB_OPTIONS } from '../../data/herbs.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = [HERBS.length, HERB_OPTIONS.length]; }
}
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn autofighter_shaped_herbs_import_remaps() {
    let src = "import { HERBS, HERB_OPTIONS } from '../../data/herbs.js'; export default class T extends LoopingBot { loop() {} }";
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn herb_cleaner_logic_transitive_herbs_scan_is_loadable() {
    use script::load::first_unloadable_for_card;

    let dir = std::env::temp_dir().join(format!("274bot-herb-logic-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let card_dir = dir.join("HerbCleaner");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(
        card_dir.join("HerbCleanerLogic.js"),
        "import { HERBS, HERB_OPTIONS } from '../../data/herbs.js';\nexport { HERBS, HERB_OPTIONS };\n",
    )
    .unwrap();
    let card_path = card_dir.join("HerbCleaner.ts");
    let origin = "import { HERBS } from './HerbCleanerLogic.js'; export default class T extends LoopingBot { loop() {} }";
    std::fs::write(&card_path, origin).unwrap();
    assert_eq!(
        first_unloadable_for_card(origin, &card_path),
        None,
        "transitive ../../data/herbs.js must remap through HerbCleanerLogic"
    );
}
