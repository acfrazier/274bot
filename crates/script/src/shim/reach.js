import { Execution } from '../execution/Execution.js';
import { notImpl, proxy, runMachine } from '../../shim/_kernel.js';

function optionalOpenMs(openMs) {
    return typeof openMs === 'number' && Number.isFinite(openMs) && openMs >= 0
        ? Math.floor(openMs)
        : undefined;
}

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
    async npcDialog(opts) {
        const near = opts && opts.near ? opts.near : {};
        const openMs = optionalOpenMs(opts && opts.openMs);
        const out = await runMachine('reach-npc-dialog', {
            name: String(opts && opts.name != null ? opts.name : ''),
            near: { x: near.x, z: near.z, level: near.level ?? 0 },
            ...(openMs !== undefined ? { openMs } : {}),
        });
        if (out.kind === 'refused') throw notImpl('Reach.npcDialog', out.reason);
        return out.kind === 'done' ? out.value : 'retry';
    },
});
