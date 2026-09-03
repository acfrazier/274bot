// Our Skills module: reads posted stats rows. Missing members throw `not v1`.
import { notV1, proxy, snap } from '../../shim/_kernel.js';

const stats = () => snap().stats || [];

function mustStat(name, op) {
    const i = stats().findIndex(
        (row) => row && typeof row.name === 'string' && row.name.toLowerCase() === String(name).toLowerCase(),
    );
    if (i === -1) throw notV1(op);
    const row = stats()[i];
    if (!row) throw notV1(op);
    return row;
}

export const Skills = new Proxy(
    {
        index(name) {
            const wanted = String(name).toLowerCase();
            return stats().findIndex(
                (row) => row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
            );
        },
        xp(name) {
            const row = mustStat(name, 'Skills.xp');
            if (typeof row.xp !== 'number') throw notV1('Skills.xp');
            return row.xp;
        },
        level(name) {
            const row = mustStat(name, 'Skills.level');
            if (typeof row.level !== 'number') throw notV1('Skills.level');
            return row.level;
        },
        effective(name) {
            const row = mustStat(name, 'Skills.effective');
            if (typeof row.effective === 'number') return row.effective;
            if (typeof row.level === 'number') return row.level;
            throw notV1('Skills.effective');
        },
        hpFraction() {
            const base = Skills.level('hitpoints');
            if (base <= 0) throw notV1('Skills.hpFraction');
            return Skills.effective('hitpoints') / base;
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
