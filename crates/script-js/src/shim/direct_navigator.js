import { proxy, notImpl, machineNow, runMachine } from '../../shim/_kernel.js';

// Frozen DirectNavigator (`DirectNavigator.ts`): the same-scene click, not
// the Traveller route (`Traversal.walkTo`). Rust owns the ±48 clamp, the
// scene-window check, the click and the walkTo reissue loop.
export const DirectNavigator = proxy('DirectNavigator', {
    walk(dest) {
        if (!dest || typeof dest.x !== 'number' || typeof dest.z !== 'number') return false;
        const out = machineNow('direct-walk-click', {
            dest: { x: dest.x, z: dest.z, level: dest.level ?? 0 },
        });
        if (out.kind === 'refused') throw notImpl('DirectNavigator.walk', out.reason);
        return out.kind === 'done' && out.value === true;
    },
    async walkTo(dest, radius = 2, timeoutMs = 45000) {
        if (!dest || typeof dest.x !== 'number' || typeof dest.z !== 'number') return false;
        const out = await runMachine('direct-walk', {
            dest: { x: dest.x, z: dest.z, level: dest.level ?? 0 },
            radius: Math.floor(radius),
            timeoutMs: Math.max(0, Math.floor(timeoutMs)),
        });
        if (out.kind === 'refused') throw notImpl('DirectNavigator.walkTo', out.reason);
        return out.kind === 'done' && out.value === true;
    },
});
