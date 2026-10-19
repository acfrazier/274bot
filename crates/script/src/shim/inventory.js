// Inventory row handles are materialized by the native snapshot adapter.
import { host, notImpl, queue, proxy } from '../../shim/_kernel.js';

const inventoryIdentity = Symbol('inventoryIdentity');
const inventoryRow = Symbol('inventoryRow');

export function backpackSnapshots() {
    return globalThis.__rs2b0t_inventory('backpackSnapshots');
}

export function backpackCapacity() {
    return globalThis.__rs2b0t_inventory('backpackCapacity', host());
}

export class InvItem {
    constructor(row) {
        const hasIdentity =
            Number.isInteger(row.id) && Number.isInteger(row.slot) && row.slot >= 0;
        this.name = row.name;
        this.count = row.count;
        this.id = row.id ?? 0;
        this.slot = row.slot ?? 0;
        this.noted = row.noted === true;
        this.cert = row.cert ?? -1;
        this[inventoryIdentity] = hasIdentity ? { id: row.id, slot: row.slot } : null;
        this[inventoryRow] = row;
    }

    actions() {
        const row = this[inventoryRow];
        return Array.isArray(row.ops) ? row.ops.slice() : [];
    }

    interact(action) {
        const request = globalThis.__rs2b0t_inventory('interact', this[inventoryRow], action);
        queue(request);
        return true;
    }

    useOn(target) {
        const request = globalThis.__rs2b0t_inventory(
            'useOn',
            this,
            this[inventoryRow],
            target,
            inventoryIdentity,
        );
        if (!request) return false;
        queue(request);
        return true;
    }
}

export const Inventory = new Proxy(
    {
        count(name) {
            return globalThis.__rs2b0t_inventory('count', name);
        },
        countById(id) {
            return globalThis.__rs2b0t_inventory('countById', id);
        },
        first(name) {
            return globalThis.__rs2b0t_inventory('first', name, InvItem);
        },
        items() {
            return globalThis.__rs2b0t_inventory('items', InvItem);
        },
        contains(name) {
            return Inventory.first(name) !== null;
        },
        used() {
            return globalThis.__rs2b0t_inventory('used');
        },
        isFull() {
            return globalThis.__rs2b0t_inventory('isFull', Inventory, host());
        },
        free() {
            return globalThis.__rs2b0t_inventory('free', Inventory, host());
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl('Inventory.' + String(prop));
        },
    },
);
