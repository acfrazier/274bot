#![cfg(feature = "test-hooks")]

use rustyscript::Module;
use script_js::shim::{filter_gated_modules_for_test, shim_modules, BOT_MODULE};
const SIBLING_MODULE: &str = "/rs2b0t/bot/scripts/bot/sibling.js";

const THIEVER_SOURCE: &str = r#"
import { foodHealAmount, shouldEatToUseFood, MIN_EAT_HP } from '../../api/combat/food.js';
import { createReturnToAnchorTask, resolveRunAnchor, tileWithinLeash } from '../../api/tasks/Anchor.js';
import { TaskBot } from '../../api/bot/Bot.js';
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
import Tile from '../../geometry/Tile.js';
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
import { Bank } from '../../api/bank/Bank.js';
import { openBankAccess } from '../../api/bank/Banking.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Paint } from '../../paint/Paint.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
import { SettingsStore } from '../../runtime/Settings.js';
import { Skills } from '../../api/skills/Skills.js';
import { ContinueDialog } from '../../api/tasks/ContinueDialog.js';
import { GroundItems } from '../../api/grounditems/GroundItems.js';
import { Npcs } from '../../api/npcs/Npcs.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { nearestBank } from '../../api/bank/BankLocations.js';
import { walkOpening } from '../../event/webwalk/walkOpening.js';
import { PICKPOCKET_TARGET_NAMES } from '../../data/pickpocketTargets.js';
import { fmtDuration } from '../../paint/paintLogic.js';
import { chooseTarget } from '../../api/thieving/targets.js';
import {
    STUN_COMBAT_TICKS,
    autoFoodBanking,
    canStealNow,
    closeBankAndConfirmCount,
    countFood,
    foodMatches,
    shouldRestockFood,
    THIEVER_BANKING_OPTIONS,
    withdrawTo
} from '../../api/thieving/stealRules.js';
import { scriptFood } from '../../api/loadout/loadoutPlan.js';
import { LOADOUT_SETTING } from '../../api/loadout/loadoutSetting.js';

export default class ThieverImportGraphProbe extends TaskBot {
    onStart() {}
    loop() {}
}
"#;

fn thiever_modules() -> Vec<Module> {
    let mut modules = shim_modules();
    modules.push(Module::new(
        SIBLING_MODULE,
        "import '../../api/map/regions.js';",
    ));
    modules.push(Module::new(BOT_MODULE, THIEVER_SOURCE));
    modules
}

#[test]
fn per_start_import_closure_allocation_is_bounded() {
    // The process-wide shim graph is built once. Measure only the work for this
    // bot's imports and closure, as a new isolate start would do after warm-up.
    let mut warm_modules = thiever_modules();
    filter_gated_modules_for_test(&mut warm_modules, [SIBLING_MODULE, BOT_MODULE]);

    let mut modules = thiever_modules();
    let allocated = allocation_counter::measure(|| {
        filter_gated_modules_for_test(&mut modules, [SIBLING_MODULE, BOT_MODULE]);
    });
    const LIMIT: u64 = 128 * 1024;
    eprintln!(
        "per-start import closure allocations={} bytes; limit={LIMIT}",
        allocated.bytes_total
    );
    assert!(
        allocated.bytes_total <= LIMIT,
        "per-start import closure allocated {} bytes, above {LIMIT}",
        allocated.bytes_total
    );
}

#[test]
fn missing_shim_import_error_names_module_path() {
    const MISSING_PATH: &str = "/rs2b0t/bot/api/map/__cf029_missing_module.js";
    let source = r#"
import '../../api/map/__cf029_missing_module.js';
export default class MissingShimModule extends LoopingBot {}
"#;
    let isolate = script_js::LoadIsolate::spawn(
        source.to_owned(),
        script_js::LoadShape::CompatClass,
        Vec::new(),
    )
    .expect("spawn missing-module fixture");

    // Probe waits for setup completion; the following read retains the exact
    // loader diagnostic instead of reducing it to the probe channel failure.
    let _ = isolate.probe("true");
    let error = match isolate.poll_ready() {
        script_js::Ready::Failed(error) => error,
        other => panic!("missing shim import unexpectedly loaded: {other:?}"),
    };
    assert!(
        error.contains("__cf029_missing_module.js"),
        "missing shim import error must name {MISSING_PATH}: {error}"
    );
    isolate.join();
}
