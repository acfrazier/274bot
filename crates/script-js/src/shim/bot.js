// The bot base classes, re-exported from the prelude globals so `extends`
// and `instanceof` agree with the tick wrapper. The classes themselves
// (including TaskBot's validate-first task loop) live in the prelude.
import { notImpl } from '../../shim/_kernel.js';
export const LoopingBot = globalThis.LoopingBot;
export const TaskBot = globalThis.TaskBot;
export const TreeBot = globalThis.TreeBot;
export const AbstractBot = globalThis.LoopingBot;

export function resolveLoopCadence() {
    throw notImpl('resolveLoopCadence');
}

export class BranchTask {
    constructor() {
        throw notImpl('BranchTask.constructor');
    }

    validate() {
        throw notImpl('BranchTask.validate');
    }

    success() {
        throw notImpl('BranchTask.success');
    }

    failure() {
        throw notImpl('BranchTask.failure');
    }
}

export class LeafTask {
    constructor() {
        throw notImpl('LeafTask.constructor');
    }

    execute() {
        throw notImpl('LeafTask.execute');
    }
}
