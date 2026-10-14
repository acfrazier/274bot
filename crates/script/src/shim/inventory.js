// Our Inventory module: reads posted inv rows; item handles queue held/use-on
// ops on `__rs2b0t_host.interact`. Missing members throw `not impl`.
import { snap, notImpl, queue, proxy } from '../../shim/_kernel.js';

const inventoryIdentity = Symbol('inventoryIdentity');

function rows() {
    return snap().inv || [];
}

function capacity() {
    const h = globalThis.__rs2b0t_host || {};
    if (typeof h.invSize === 'number') return h.invSize;
    return typeof snap().inv_size === 'number' ? snap().inv_size : 0;
}


export function backpackSnapshots() {
    throw notImpl('backpackSnapshots');
}

export function backpackCapacity() {
    throw notImpl('backpackCapacity');
}
function useOnKind(target) {
    const cn = target?.constructor?.name;
    if (cn === 'Npc') return 'npc';
    if (cn === 'Loc') return 'loc';
    if (cn === 'GroundItem') return 'obj';
    if (cn === 'Player') return 'player';
    throw notImpl('Inventory.useOn');
}

function held(row) {
    return new InvItem(row);
}

const inventoryRow = Symbol('inventoryRow');

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
        queue({ op: 'held', name: this.name, action: String(action) });
        return true;
    }

    useOn(target) {
        if (!target) return false;
        if (target.name && !target.snap) {
            const targetIdentity = target[inventoryIdentity];
            queue({
                op: 'use-on',
                name: this.name,
                kind: 'inv',
                target_name: target.name,
                source_item_id: this[inventoryIdentity]?.id ?? null,
                source_item_slot: this[inventoryIdentity]?.slot ?? null,
                target_item_id: targetIdentity?.id ?? null,
                target_item_slot: targetIdentity?.slot ?? null,
                x: 0,
                z: 0,
                level: 0,
                index: null,
            });
            return true;
        }
        if (target.name && target.snap) {
            const kind = useOnKind(target);
            queue({
                op: 'use-on',
                name: this.name,
                kind,
                target_name: target.name ?? target.snap.name ?? null,
                source_item_id: this[inventoryIdentity]?.id ?? null,
                source_item_slot: this[inventoryIdentity]?.slot ?? null,
                x: target.snap?.x ?? 0,
                z: target.snap?.z ?? 0,
                level: target.snap?.level ?? 0,
                index:
                    kind === 'npc'
                        ? (target.snap?.index ?? target.index ?? null)
                        : null,
            });
            return true;
        }
        return false;
    }
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
            const rs = rows();
            if (rs.length === 0) {
                return 0;
            }
            if (!rs.some((r) => r && typeof r.id === 'number' && r.id !== 0)) {
                throw notImpl('Inventory.countById');
            }
            const want = Number(id);
            return rs
                .filter((r) => r && r.id === want)
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
            throw notImpl('Inventory.' + String(prop));
        },
    },
);
