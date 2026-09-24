// Special bar: posted varps if present. Cost/bar/arm need selected-cache
// facts; the arm is one Rust step machine and the bar root comes from the
// scene the isolate already decoded.
import { snap, notImpl, proxy, runMachine } from '../../shim/_kernel.js';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_special(payload);
}

function controls() {
    return call({ op: 'controls' });
}

function postedVarp(index) {
    const row = (snap().varps || []).find((v) => v && v.index === index);
    if (!row || typeof row.value !== 'number') {
        throw notImpl('Special');
    }
    return row.value;
}

function requireControls() {
    const c = controls();
    if (!c || c.available !== true) {
        throw notImpl('Special');
    }
    return c;
}

export const SA_MAX_ENERGY = (() => {
    try {
        const c = controls();
        if (c && c.available === true && typeof c.max_energy === 'number') {
            return c.max_energy;
        }
    } catch (_) {
        /* module load without a host still exports the audited 1000 */
    }
    return 1000;
})();

export const Special = proxy('Special', {
    energy() {
        const c = controls();
        const index = c && c.available === true ? c.energy_varp : 300;
        return postedVarp(index);
    },
    armed() {
        const c = controls();
        const index = c && c.available === true ? c.armed_varp : 301;
        const armedValue = c && c.available === true ? c.armed_value : 1;
        return postedVarp(index) === armedValue;
    },
    wielded() {
        const eq = snap().equipment || [];
        const row = eq.find((r) => r && r.slot === 3) || eq[3];
        return row && row.name ? row.name : '';
    },
    cost(weaponName) {
        requireControls();
        const value = call({ op: 'cost', weapon: String(weaponName ?? '') });
        return typeof value === 'number' ? value : null;
    },
    ready(weaponName) {
        const cost = Special.cost(weaponName);
        return cost !== null && Special.energy() >= cost;
    },
    barComponent() {
        requireControls();
        const bar = call({ op: 'bar' });
        return typeof bar === 'number' ? bar : -1;
    },
    async arm() {
        const out = await runMachine('special', {});
        if (out.kind === 'refused') throw notImpl('Special', out.reason);
        return out.kind === 'done' && out.value === true;
    },
});
