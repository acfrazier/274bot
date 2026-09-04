// Special bar: posted varps if present. Arming the bar is not impl until an IF is posted.
import { snap, notImpl, proxy } from '../../shim/_kernel.js';

const SA_ENERGY_VARP = 300;
const SA_ARMED_VARP = 301;

export const SA_MAX_ENERGY = 1000;

function varp(index) {
    const row = (snap().varps || []).find((v) => v && v.index === index);
    return row && typeof row.value === 'number' ? row.value : 0;
}

export const Special = proxy('Special', {
    energy() {
        return varp(SA_ENERGY_VARP);
    },
    armed() {
        return varp(SA_ARMED_VARP) !== 0;
    },
    wielded() {
        const eq = snap().equipment || [];
        const row = eq.find((r) => r && r.slot === 3) || eq[3];
        return row && row.name ? row.name : '';
    },
    cost() {
        throw notImpl('Special.cost');
    },
    ready() {
        throw notImpl('Special.ready');
    },
    barComponent() {
        throw notImpl('Special.barComponent');
    },
    arm() {
        throw notImpl('Special.arm');
    },
});
