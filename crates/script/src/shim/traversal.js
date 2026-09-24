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
    // Frozen Traversal.walkResilient retries walkTo until arrived, interrupted,
    // attempts-without-progress, or timeout. A single WalkNear NoPath from a
    // stale tile (ridge fall exactmove) must not finish the recovery walk.
    async walkResilient(tile, opts = {}) {
        const attempts = opts.attempts;
        if (typeof attempts !== 'number') {
            return walkWorld(tile, opts);
        }
        const radius = opts.radius ?? 0;
        const timeoutMs = opts.timeoutMs ?? 60_000;
        const deadline = performance.now() + timeoutMs;
        const target = { x: tile.x, z: tile.z, level: tile.level ?? 0 };
        let noProgress = 0;
        let lastHere = null;
        while (true) {
            const here = snap().here;
            if (!here) return false;
            if (arrived(target, radius)) return true;
            const remaining = deadline - performance.now();
            if (remaining <= 0) return false;
            const hereKey = here.x + ',' + here.z + ',' + (here.level ?? 0);
            const ok = await walkWorld(tile, { ...opts, timeoutMs: remaining });
            if (ok || arrived(target, radius)) return true;
            if (performance.now() >= deadline) return false;
            if (lastHere === hereKey) noProgress += 1;
            else {
                noProgress = 1;
                lastHere = hereKey;
            }
            if (noProgress >= attempts) return false;
            await Execution.delayTicks(1);
        }
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
