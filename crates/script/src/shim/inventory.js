// Our Inventory module: reads posted inv rows; item handles queue held/use-on
// ops on `__rs2b0t_host.interact`. Missing members throw `not v1`.
import { snap, notV1, queue, proxy } from '../../shim/_kernel.js';

const BACKPACK_CAPACITY = 28;

function rows() {
    return snap().inv || [];
}

function capacity() {
    const size = typeof snap().inv_size === 'number' ? snap().inv_size : 0;
    return size > 0 ? size : BACKPACK_CAPACITY;
}

function held(row) {
    return {
        name: row.name,
        count: row.count,
        id: row.id ?? 0,
        slot: row.slot ?? 0,
        interact(action) {
            queue({ op: 'held', name: row.name, action: String(action) });
            return true;
        },
        actions() {
            return [];
        },
        useOn(target) {
            if (!target) return false;
            if (target.name && target.snap) {
                queue({
                    op: 'use-on',
                    name: row.name,
                    kind: 'npc',
                    target_name: target.name ?? target.snap.name ?? null,
                    x: target.snap?.x ?? 0,
                    z: target.snap?.z ?? 0,
                    level: target.snap?.level ?? 0,
                    index: target.snap?.index ?? target.index ?? null,
                });
                return true;
            }
            return false;
        },
    };
}

export const Inventory = new Proxy(
    {
        count(name) {
            const wanted = String(name).toLowerCase();
            return rows()
                .filter((r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted)
                .reduce((sum, row) => sum + (typeof row.count === 'number' ? row.count : 0), 0);
        },
        countById(id) {
            return rows()
                .filter((r) => r && r.id === id)
                .reduce((sum, row) => sum + (typeof row.count === 'number' ? row.count : 0), 0);
        },
        first(name) {
            const wanted = String(name).toLowerCase();
            const row = rows().find(
                (r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted,
            );
            return row ? held(row) : null;
        },
        items() {
            return rows().filter((r) => r && r.name).map((r) => held(r));
        },
        contains(name) {
            return Inventory.first(name) !== null;
        },
        used() {
            return rows().filter((r) => r && r.name).length;
        },
        isFull() {
            const size = capacity();
            return size > 0 && Inventory.used() >= size;
        },
        free() {
            const size = capacity();
            return size > 0 ? Math.max(0, size - Inventory.used()) : 0;
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Inventory.' + String(prop));
        },
    },
);
