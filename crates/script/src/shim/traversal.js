import { snap, proxy, notImpl, runMachine } from '../../shim/_kernel.js';
import { Sustain } from '../sustain/Sustain.js';

const whole = (value) => (typeof value === 'number' ? { value: Math.floor(value) } : null);
const count = (value) => (Array.isArray(value) ? value.length : 0);

// Frozen Traversal.walkTo(dest, WalkOptions): Rust owns the walk, the
// option defaults and refusals, the Sustain pumps, the boat-fare recovery
// and the one re-walk (`walk-to`). The shim only coerces options.
async function walkWorld(tile, opts = {}) {
    const policy = opts.policy || {};
    const radius = whole(opts.radius);
    const timeoutMs = whole(opts.timeoutMs);
    const span = whole(policy.distanceBeforeTeleport);
    const out = await runMachine(
        'walk-to',
        {
            tile: { x: tile.x, z: tile.z, level: tile.level ?? 0 },
            ...(radius ? { radius: radius.value } : {}),
            ...(timeoutMs ? { timeoutMs: timeoutMs.value } : {}),
            ...(typeof opts.useTeleportCatalog === 'boolean'
                ? { useTeleportCatalog: opts.useTeleportCatalog }
                : {}),
            policy: {
                ...(typeof policy.useTeleports === 'boolean' ? { useTeleports: policy.useTeleports } : {}),
                ...(span ? { distanceBeforeTeleport: span.value } : {}),
                allowTeleportIds: count(policy.allowTeleportIds),
                denyTeleportIds: count(policy.denyTeleportIds),
                ...(typeof policy.useShips === 'boolean' ? { useShips: policy.useShips } : {}),
                ...(typeof policy.useShortcuts === 'boolean' ? { useShortcuts: policy.useShortcuts } : {}),
            },
            avoidZones: Array.isArray(opts.avoidZones) ? opts.avoidZones : [],
            crossZones: Array.isArray(opts.crossZones) ? opts.crossZones : [],
            pathFollow: opts.pathFollow !== undefined && opts.pathFollow !== null,
            forceRepath: opts.forceRepath === true,
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
                    avoidZones: Array.isArray(opts.avoidZones) ? opts.avoidZones : [],
                    crossZones: Array.isArray(opts.crossZones) ? opts.crossZones : [],
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
