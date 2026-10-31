// Host-loop base classes are re-exported from prelude globals so `extends`
// and `instanceof` agree with the tick wrapper. Abstract task bases live here;
// TaskBot sequencing remains in the prelude and TreeBot sequencing is native.
export const LoopingBot = globalThis.LoopingBot;
export const TaskBot = globalThis.TaskBot;
export const TreeBot = globalThis.TreeBot;
export const AbstractBot = globalThis.LoopingBot;

export class BranchTask {}
globalThis.__rs2b0t_branch_task_ctor = BranchTask;

export class LeafTask {}
