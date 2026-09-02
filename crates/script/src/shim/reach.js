import { Execution } from '../execution/Execution.js';
import { proxy } from '../../shim/_kernel.js';

export const Reach = proxy('Reach', {
    async entityOp(opts) {
        if (opts.expect()) return 'done';
        const entity = opts.find();
        if (!entity) return 'retry';
        const ok = await entity.interact(opts.op);
        if (!ok) return 'retry';
        const settled = await Execution.delayUntil(opts.expect, opts.expectMs ?? 5000);
        return settled ? 'done' : 'retry';
    },
});
