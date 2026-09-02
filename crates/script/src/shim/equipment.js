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
            }));
    },
    contains(name) {
        const wanted = String(name).toLowerCase();
        return rows().some((r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted);
    },
    async equip(name) {
        if (Equipment.contains(name)) return true;
        const item = Inventory.first(name);
        if (!item) return false;
        const op = item.actions().find((o) => /wield|wear|equip/i.test(o));
        if (!op) return false;
        if (!(await item.interact(op))) return false;
        return Execution.delayUntil(() => Equipment.contains(name), 3000);
    },
    async unequip(name) {
        if (!Equipment.contains(name)) return true;
        queue({ op: 'wear', name: String(name) });
        return Execution.delayUntil(() => !Equipment.contains(name), 3000);
    },
});
