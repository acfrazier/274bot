// The `@rs2b0t/api` bundle: our shim names, re-exported. The source
// rewrite in `remap_rs2b0t_api` points bare `@rs2b0t/api` imports here.
export { Game } from '../../api/game/Game.js';
export { Inventory } from '../../api/inventory/Inventory.js';
export { Skills } from '../../api/skills/Skills.js';
export { Bank } from '../../api/bank/Bank.js';
export {
    Banking,
    COMMON_BANK_LOOT,
    depositAllExcept,
    depositMatcher,
    matchesCommonBankLoot,
} from '../../api/bank/Banking.js';
export { EventSignal } from '../../api/execution/EventSignal.js';
export { Execution } from '../../api/execution/Execution.js';
export { reader, actions } from '../../adapter/ClientAdapter.js';
export { LoopingBot, TaskBot, TreeBot } from '../../api/bot/Bot.js';
export { Paint } from '../../paint/Paint.js';
export { ScriptRunner } from '../../runtime/ScriptRunner.js';
export const defineBot = globalThis.defineBot;
