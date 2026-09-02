// Our Skills module: reads posted stats rows. Missing members throw `not v1`.
import { notV1, proxy, snap } from '../../shim/_kernel.js';

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
        level(name) {
            const i = Skills.index(name);
            if (i === -1) return 0;
            const row = stats()[i];
            return row && typeof row.level === 'number' ? row.level : 0;
        },
        effective(name) {
            const i = Skills.index(name);
            if (i === -1) return 0;
            const row = stats()[i];
            if (row && typeof row.effective === 'number') return row.effective;
            return row && typeof row.level === 'number' ? row.level : 0;
        },
        hpFraction() {
            const base = Skills.level('hitpoints');
            return base > 0 ? Skills.effective('hitpoints') / base : 1;
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
