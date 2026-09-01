// The `@rs2b0t/api` bundle: our shim names, re-exported. The source
// rewrite in `remap_rs2b0t_api` points bare `@rs2b0t/api` imports here.
export { Game } from '../../api/game/Game.js';
export { reader, actions } from '../../adapter/ClientAdapter.js';
export { LoopingBot, TaskBot, TreeBot } from '../../api/bot/Bot.js';
export { Paint } from '../../paint/Paint.js';
export { ScriptRunner } from '../../runtime/ScriptRunner.js';
export const defineBot = globalThis.defineBot;
