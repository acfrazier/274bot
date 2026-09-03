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
