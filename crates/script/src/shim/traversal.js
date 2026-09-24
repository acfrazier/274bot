import { snap, queue, proxy, notImpl, arrived } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';

function allowTeleports(opts) {
    return opts.useTeleportCatalog === true || opts.policy?.useTeleports === true;
}

function walkNative(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_walk(payload);
}

async function walkWorld(tile, opts = {}) {
    const radius = opts.radius ?? 0;
    if (!snap().here) return false;
    const target = { x: tile.x, z: tile.z, level: tile.level ?? 0 };
    if (arrived(target, radius)) return true;
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
        allow_wilderness: true,
        allow_bank_fetch: true,
        request_id: token,
    });
    const done = await Execution.delayUntil(
        () => walkNative({ op: 'settled', token }) === true,
        opts.timeoutMs ?? 60_000,
    );
    return done === true && walkNative({ op: 'value', token }) === true;
}

export const Traversal = proxy('Traversal', {
    walkTo: walkWorld,
    walkResilient: walkWorld,
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
