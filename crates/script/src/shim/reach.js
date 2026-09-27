import { notImpl, proxy, runMachine } from '../../shim/_kernel.js';
import { Sustain } from '../sustain/Sustain.js';

function optionalOpenMs(openMs) {
    return typeof openMs === 'number' && Number.isFinite(openMs) && openMs >= 0
        ? Math.floor(openMs)
        : undefined;
}

export const Reach = proxy('Reach', {
    // One `reach-entity-op` await. `interact` is the frozen attempt's
    // `find()` then `interact(op)`; `target` is `find()?.tile()`.
    async entityOp(opts) {
        const out = await runMachine(
            'reach-entity-op',
            {
                expectMs: opts.expectMs ?? 5000,
                what: String(opts.what ?? opts.op),
                openWhenUnreachable: opts.openWhenUnreachable ?? false,
            },
            {
                expect: () => opts.expect(),
                interact: () => {
                    const entity = opts.find();
                    return entity ? entity.interact(opts.op) : false;
                },
                target: () => {
                    const t = opts.find()?.tile();
                    return t ? { x: t.x, z: t.z, level: t.level ?? 0 } : null;
                },
                log: opts.log,
            },
        );
        if (out.kind === 'refused') throw notImpl('Reach.entityOp', out.reason);
        return out.kind === 'done' ? out.value : 'retry';
    },
    async npcDialog(opts) {
        const near = opts && opts.near ? opts.near : {};
        const openMs = optionalOpenMs(opts && opts.openMs);
        const out = await runMachine(
            'reach-npc-dialog',
            {
                name: String(opts && opts.name != null ? opts.name : ''),
                near: { x: near.x, z: near.z, level: near.level ?? 0 },
                ...(openMs !== undefined ? { openMs } : {}),
            },
            {
                log: opts && typeof opts.log === 'function' ? opts.log : undefined,
                sustain: () => Sustain.run(),
            },
        );
        if (out.kind === 'refused') throw notImpl('Reach.npcDialog', out.reason);
        return out.kind === 'done' ? out.value : 'retry';
    },
});
