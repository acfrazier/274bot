// Shared helpers for kernel shim facades (snapshot reads + interact queue).
export const host = () => globalThis.__rs2b0t_host || {};
export const snap = () => host().snapshot || {};
export const notV1 = (name) => new Error('not v1: ' + name);
export const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};

export const proxy = (ns, members) =>
    new Proxy(members, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1(ns + '.' + String(prop));
        },
    });

export function chebyshev(a, b) {
    if (!a || !b || (a.level ?? 0) !== (b.level ?? 0)) return Infinity;
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
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
