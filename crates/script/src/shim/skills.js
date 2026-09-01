// Our Skills module: `index` scans the host-posted snapshot's stats rows
// for the name (case-insensitive; -1 when absent), `xp` reads that row's
// xp (0 when absent). Missing snapshot or missing stat fail closed —
// never a fake value. Every other member throws `not v1`.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const snap = () => host().snapshot || {};
const stats = () => snap().stats || [];

export const Skills = new Proxy(
    {
        index(name) {
            const wanted = String(name).toLowerCase();
            return stats().findIndex(
                (row) => row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
            );
        },
        xp(name) {
            const i = Skills.index(name);
            if (i === -1) return 0;
            const row = stats()[i];
            return row && typeof row.xp === 'number' ? row.xp : 0;
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Skills.' + String(prop));
        },
    },
);
