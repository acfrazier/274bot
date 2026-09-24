import { snap, queue, proxy } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';
import { Execution } from '../execution/Execution.js';

const rows = () => snap().equipment || [];

export const Equipment = proxy('Equipment', {
    items() {
        return rows()
            .filter((r) => r && r.name)
            .map((r) => ({
                name: r.name,
                count: typeof r.count === 'number' ? r.count : 1,
                id: typeof r.id === 'number' ? r.id : 0,
            }));
    },
    contains(name) {
        const wanted = String(name).toLowerCase();
        return rows().some((r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted);
    },
    // The host `wear` op owns the menu choice (its own Wear, else Wield,
    // else refuse); this module never picks an action label by regex.
    async equip(name) {
        if (Equipment.contains(name)) return true;
        if (!Inventory.first(name)) return false;
        queue({ op: 'wear', name: String(name) });
        return Execution.delayUntil(() => Equipment.contains(name), 3000);
    },
    async unequip(name) {
        if (!Equipment.contains(name)) return true;
        queue({ op: 'unequip', name: String(name) });
        return Execution.delayUntil(() => !Equipment.contains(name), 3000);
    },
});
