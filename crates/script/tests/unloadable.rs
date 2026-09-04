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
ScriptRegistry.register({ name: 'Woodcutter', create: () => new Woodcutter() });
ScriptRegistry.register({ name: 'CookBot', create: () => new CookBot() });
"#,
    )
    .unwrap();
    let body = "import { Game } from '../../api/game/Game.js'; export default class T extends LoopingBot { loop() {} }";
    std::fs::write(scripts.join("Woodcutter.ts"), body).unwrap();
    std::fs::create_dir_all(root.join("src/bot/scripts/CookBot")).unwrap();
    std::fs::write(root.join("src/bot/scripts/CookBot/CookBot.ts"), body).unwrap();

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
fn firemaking_and_light_fire_import_lines_remap() {
    let src = r#"
import { FIRE_SPOTS, LOG_LEVELS, lightFire } from '../../api/firemaking/Firemaking.js';
import { lightFire as light } from '../../api/firemaking/LightFire.js';
export default class T extends LoopingBot { loop() {} }
"#;
    assert_eq!(script::first_unloadable_specifier(src), None);
}
