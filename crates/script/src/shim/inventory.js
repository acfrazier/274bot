// Inventory row handles are materialized by the native snapshot adapter.
import { host, notImpl, queue } from '../../shim/_kernel.js';

const inventoryIdentity = Symbol('inventoryIdentity');
const inventoryRow = Symbol('inventoryRow');

export function backpackSnapshots() {
    return globalThis.__rs2b0t_inventory(0);
}

export function backpackCapacity() {
    return globalThis.__rs2b0t_inventory(1, host());
}

export class InvItem {
    constructor(row) {
        globalThis.__rs2b0t_inventory(15, this, row, inventoryIdentity, inventoryRow);
    }

    actions() {
        return globalThis.__rs2b0t_inventory(16, this[inventoryRow]);
    }

    interact(action) {
        const request = globalThis.__rs2b0t_inventory(9, this[inventoryRow], action);
        queue(request);
        return true;
    }

    useOn(target) {
        const request = globalThis.__rs2b0t_inventory(10, this,
        this[inventoryRow],
        target,
        inventoryIdentity,);
        if (!request) return false;
        queue(request);
        return true;
    }
}

export const Inventory = new Proxy(
    {
        count(name) {
            return globalThis.__rs2b0t_inventory(5, name);
        },
        countById(id) {
            return globalThis.__rs2b0t_inventory(6, id);
        },
        first(name) {
            return globalThis.__rs2b0t_inventory(3, name, InvItem);
        },
        items() {
            return globalThis.__rs2b0t_inventory(2, InvItem);
        },
        contains(name) {
            return globalThis.__rs2b0t_inventory(17, Inventory, name);
        },
        used() {
            return globalThis.__rs2b0t_inventory(4);
        },
        isFull() {
            return globalThis.__rs2b0t_inventory(7, Inventory, host());
        },
        free() {
            return globalThis.__rs2b0t_inventory(8, Inventory, host());
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
