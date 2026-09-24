// Shared helpers for kernel shim facades (snapshot reads + interact queue).
import { parkMachine } from '../api/execution/Execution.js';

export const host = () => globalThis.__rs2b0t_host || {};
export const snap = () => host().snapshot || {};
export const notImpl = (name, reason) =>
    new Error(reason ? 'not impl: ' + name + ': ' + reason : 'not impl: ' + name);
export const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};

// Start one Rust step machine and await its single completion. Resolves
// `{ kind: 'done', value }`, `{ kind: 'refused', reason }` (nothing
// started) or `{ kind: 'aborted', reason }` (ResetSession, superseded).
export function runMachine(family, args) {
    const started = globalThis.__rs2b0t_machine_start(family, args);
    return started.kind === 'running' ? parkMachine(started.handle) : Promise.resolve(started);
}

export const proxy = (ns, members) =>
    new Proxy(members, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl(ns + '.' + String(prop));
        },
    });

export function distanceTo(a, b) {
    if (!a || !b) return Infinity;
    return globalThis.__rs2b0t_distance(a, b);
}


export function presentOps(actions) {
    return (actions || []).filter((a) => a && a !== 'hidden');
}

export function opIndex(actions, action) {
    const wanted = String(action).toLowerCase();
    for (let i = 0; i < (actions || []).length; i++) {
        const a = actions[i];
        if (a && String(a).toLowerCase() === wanted) return i + 1;
    }
    return -1;
}

/** Snapshot row → EntityQuery view ({ name, ops, tile, distance }). */
export function entitySnapView(row) {
    if (!row) return null;
    return {
        ...row,
        tile: { x: row.x, z: row.z, level: row.level ?? 0 },
        ops: row.actions || [],
        distance: typeof row.distance === 'number' ? row.distance : 0,
    };
}

export function optionalText(value) {
    if (value == null || value === '') return null;
    return String(value);
}
