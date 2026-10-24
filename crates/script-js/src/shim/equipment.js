import { queue, proxy } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';
import { Execution } from '../execution/Execution.js';

const rows = () => globalThis.__rs2b0t_inventory(11);

export const Equipment = proxy('Equipment', {
    items() {
        return rows();
    },
    contains(name) {
        return globalThis.__rs2b0t_inventory(12, name);
    },
    // The host `wear` op owns the menu choice (its own Wear, else Wield,
    // else refuse); this module never picks an action label by regex.
    async equip(name) {
        const request = globalThis.__rs2b0t_inventory(13, name, Equipment, Inventory);
        if (typeof request === 'boolean') return request;
        queue(request);
        return Execution.delayUntil(() => Equipment.contains(name), 3000);
    },
    async unequip(name) {
        const request = globalThis.__rs2b0t_inventory(14, name, Equipment);
        if (typeof request === 'boolean') return request;
        queue(request);
        return Execution.delayUntil(() => !Equipment.contains(name), 3000);
    },
});
