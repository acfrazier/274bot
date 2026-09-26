import { snap, proxy, notImpl, runMachine } from '../../shim/_kernel.js';
import { Sustain } from '../sustain/Sustain.js';


function allowTeleports(opts) {
    return opts.useTeleportCatalog === true || opts.policy?.useTeleports === true;
}

// Frozen Traversal.walkTo: Rust owns the walk, its settle, the boat-fare
// recovery and the one re-walk (`walk-to`). The shim only coerces options.
async function walkWorld(tile, opts = {}) {
    const out = await runMachine(
        'walk-to',
        {
            tile: { x: tile.x, z: tile.z, level: tile.level ?? 0 },
            radius: opts.radius ?? 0,
            ...(typeof opts.timeoutMs === 'number'
                ? { timeoutMs: Math.floor(opts.timeoutMs) }
                : {}),
            allowTeleports: allowTeleports(opts),
        },
        { log: typeof opts.log === 'function' ? opts.log : undefined, sustain: () => Sustain.run() },
    );
    if (out.kind === 'refused') throw notImpl('Traversal.walkTo', out.reason);
    return out.kind === 'done' && out.value === true;
}

export const Traversal = proxy('Traversal', {
    walkTo: walkWorld,
    // Frozen Traversal.walkResilient: Rust owns the baked walk, retries,
    // attempts/no-progress, verify probe, interrupt, arrival and the
    // teleport policy (`walk-resilient`). The shim only coerces options.
    async walkResilient(tile, opts = {}) {
        const policy = opts.policy || {};
        const out = await runMachine(
            'walk-resilient',
            {
                tile: { x: tile.x, z: tile.z, level: tile.level ?? 0 },
                opts: {
                    radius: opts.radius ?? 0,
                    ...(typeof opts.attempts === 'number'
                        ? { attempts: Math.floor(opts.attempts) }
                        : {}),
                    ...(typeof opts.timeoutMs === 'number'
                        ? { timeoutMs: Math.floor(opts.timeoutMs) }
                        : {}),
                    ...(typeof opts.sceneRadius === 'number'
                        ? { sceneRadius: Math.floor(opts.sceneRadius) }
                        : {}),
                    ...(typeof opts.useTeleportCatalog === 'boolean'
                        ? { useTeleportCatalog: opts.useTeleportCatalog }
                        : {}),
                    policy: {
                        ...(typeof policy.useTeleports === 'boolean'
                            ? { useTeleports: policy.useTeleports }
                            : {}),
                        ...(typeof policy.distanceBeforeTeleport === 'number'
                            ? { distanceBeforeTeleport: Math.floor(policy.distanceBeforeTeleport) }
                            : {}),
                    },
                    avoidZones: Array.isArray(opts.avoidZones) ? opts.avoidZones.length : 0,
                    ...(typeof opts.maxBudget === 'number' && opts.maxBudget >= 0
                        ? { maxBudget: Math.floor(opts.maxBudget) }
                        : {}),
                },
            },
            { log: typeof opts.log === 'function' ? opts.log : undefined, sustain: () => Sustain.run() },
        );
        if (out.kind === 'refused') throw notImpl('Traversal.walkResilient', out.reason);
        return out.kind === 'done' && out.value === true;
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
