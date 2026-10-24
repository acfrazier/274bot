// Skills preserves the compat surface while Rust reads the already-posted stats.
import { notImpl } from '../../shim/_kernel.js';

export const Skills = new Proxy(
    {
        index(name) {
            return globalThis.__rs2b0t_skills(0, name);
        },
        xp(name) {
            return globalThis.__rs2b0t_skills(1, name);
        },
        // Unboosted / max HP — posted `base`.
        level(name) {
            return globalThis.__rs2b0t_skills(2, name);
        },
        // Current / boosted — posted `effective`.
        effective(name) {
            return globalThis.__rs2b0t_skills(3, name);
        },
        hpFraction() {
            return globalThis.__rs2b0t_skills(4, Skills);
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl('Skills.' + String(prop));
        },
    },
);
