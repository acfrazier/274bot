// The bot base classes, re-exported from the prelude globals so `extends`
// and `instanceof` agree with the tick wrapper. The classes themselves
// (including TaskBot's validate-first task loop) live in the prelude.
export const LoopingBot = globalThis.LoopingBot;
export const TaskBot = globalThis.TaskBot;
export const TreeBot = globalThis.TreeBot;
export const AbstractBot = globalThis.LoopingBot;
