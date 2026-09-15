import { snap, queue, proxy, chebyshev, notImpl } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';

function allowTeleports(opts) {
    return opts.useTeleportCatalog === true || opts.policy?.useTeleports === true;
}

function arrival(tile, opts) {
    const radius = opts.radius ?? 0;
    const here = snap().here;
    if (!here) return { here: null, target: null, radius, arrived: false };
    const target = { x: tile.x, z: tile.z, level: tile.level ?? 0 };
    return {
        here,
        target,
        radius,
        arrived: chebyshev(here, target) <= radius,
    };
}

function walkNative(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_walk(payload);
}

export const Traversal = proxy('Traversal', {
    async walkTo(tile, opts = {}) {
        const { target, radius, arrived } = arrival(tile, opts);
        if (!target) return false;
        if (arrived) return true;
        queue({ op: 'walk-to', x: target.x, z: target.z, level: target.level });
        return Execution.delayUntil(() => {
            const h = snap().here;
            return h && chebyshev(h, target) <= radius;
        }, opts.timeoutMs ?? 60_000);
    },
    async walkResilient(tile, opts = {}) {
        const { target, radius, arrived } = arrival(tile, opts);
        if (!target) return false;
        if (arrived) return true;
        const allow_teleports = allowTeleports(opts);
        const token = walkNative({
            op: 'begin',
            x: target.x,
            z: target.z,
            level: target.level,
            radius,
            allow_teleports,
        });
        queue({
            op: radius > 0 ? 'walk-near' : 'walk',
            ...(radius > 0 ? { radius } : {}),
            x: target.x,
            z: target.z,
            level: target.level,
            allow_teleports,
            request_id: token,
        });
        const done = await Execution.delayUntil(
            () => walkNative({ op: 'settled', token }) === true,
            opts.timeoutMs ?? 60_000,
        );
        return done === true && walkNative({ op: 'value', token }) === true;
    },
    preload() {
        // NavWorld already binds at template/Play construction.
    },
    remaining() {
        throw notImpl('Traversal.remaining');
    },
    teleportsEnabled() {
        return snap().teleports_enabled === true;
    },
    requestRepath(_reason) {
        throw notImpl('Traversal.requestRepath');
    },
    get pureWalk() {
        return { useTeleportCatalog: false, policy: { useTeleports: false } };
    },
    get withTeles() {
        return { useTeleportCatalog: true, policy: { useTeleports: true } };
    },
});
