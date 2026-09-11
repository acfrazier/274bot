// Special bar: posted varps if present. Cost/bar/arm need selected-cache facts.
import { snap, notImpl, proxy } from '../../shim/_kernel.js';
import { actions, reader } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';

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
        let root = -1;
        try {
            const id = reader.sideTabInterface(0);
            root = typeof id === 'number' ? id : -1;
        } catch (_) {
            return -1;
        }
        if (root === -1) return -1;
        const bar = call({ op: 'bar', combat_tab_root: root });
        return typeof bar === 'number' ? bar : -1;
    },
    async arm() {
        const c = requireControls();
        if (Special.armed()) {
            return true;
        }
        const bar = Special.barComponent();
        if (bar === -1) {
            return false;
        }
        const token = call({ op: 'begin' }).token;
        const aborted = () => call({ op: 'current_token' }) !== token;
        if (!actions.ifButton(bar)) {
            return false;
        }
        const ticks =
            typeof c.arm_confirm_ticks === 'number' && c.arm_confirm_ticks > 0
                ? c.arm_confirm_ticks
                : 2;
        const armed = await Execution.delayUntilTicks(
            () => aborted() || Special.armed(),
            ticks,
        );
        if (aborted()) return false;
        return armed === true;
    },
});
