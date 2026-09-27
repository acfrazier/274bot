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

// Start one Rust step machine and await its single completion. `hooks`
// holds the script callbacks the family declares (called as methods of
// `hooks`). Resolves `{ kind: 'done', value }`, `{ kind: 'refused', reason }`
// (nothing started) or `{ kind: 'aborted', reason }` (ResetSession,
// superseded); rejects with what a script callback threw when the machine
// failed on it.
export async function runMachine(family, args, hooks) {
    const started = globalThis.__rs2b0t_machine_start(family, args, hooks);
    const out = started.kind === 'running' ? await parkMachine(started.handle) : started;
    if (out.kind === 'failed') throw out.error;
    return out;
}

// Start a Rust machine family that settles inside its begin (a frozen
// synchronous call) and return its envelope without awaiting:
// `{ kind: 'done', value }` or `{ kind: 'refused', reason }`.
export function machineNow(family, args) {
    return globalThis.__rs2b0t_machine_start(family, args, {});
}

export const proxy = (ns, members) =>
    new Proxy(members, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl(ns + '.' + String(prop));
        },
    });

/**
 * A declared value no shim module owns: any use of it — a member read, a
 * string conversion, arithmetic, even `Object.prototype.toString` — throws
 * `not impl`, so a bundle importer sees the honest miss instead of a fake
 * `[]`/`''`/`-1`, `[object Object]` or `NaN`.
 */
export const notImplValue = (ns) =>
    new Proxy(Object.create(null), {
        get(_target, prop) {
            throw notImpl(typeof prop === 'string' ? ns + '.' + prop : ns);
        },
    });

export function distanceTo(a, b) {
    if (!a || !b) return Infinity;
    return globalThis.__rs2b0t_distance(a, b);
}

export function planarDistanceTo(a, b) {
    if (!a || !b) return Infinity;
    return globalThis.__rs2b0t_distance(a, b, true);
}

// Walk arrival (frozen `isArrived`) from the posted player tile, in Rust.
export function arrived(dest, radius) {
    return globalThis.__rs2b0t_reach('arrived', dest, radius) === true;
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
