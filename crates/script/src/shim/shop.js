import { snap, queue, notImpl } from '../../shim/_kernel.js';

function findStock(name) {
    const wanted = String(name).toLowerCase();
    return (snap().shop_stock || []).filter(
        (row) => row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
    );
}

function shopTradeAction(actions) {
    const labels = (actions || []).filter((a) => a && a !== 'hidden');
    for (const a of labels) {
        if (String(a).toLowerCase() === 'trade') return String(a);
    }
    for (const a of labels) {
        if (/shop/i.test(String(a))) return String(a);
    }
    return null;
}

function fixedBuyQty(qty) {
    if (typeof qty === 'string' && qty.toLowerCase() === 'all') return true;
    const n = Number(qty);
    return n === 1 || n === 5 || n === 10;
}

export const Shop = new Proxy(
    {
        isOpen() {
            return snap().shop_open === true;
        },
        stock() {
            if (!Shop.isOpen()) return [];
            return (snap().shop_stock || []).map((row) => ({
                name: row?.name ?? null,
                count: row?.count ?? 0,
            }));
        },
        buy(name, qty) {
            if (!Shop.isOpen()) return;
            const rows = findStock(name);
            if (rows.length === 0) return;
            const row = rows[0];
            if (typeof row.component_id !== 'number' || row.component_id < 0) {
                throw notImpl('Shop.buy');
            }
            queue({ op: 'if-button', component_id: row.component_id });
            if (fixedBuyQty(qty)) return;
            const n = Number(qty);
            if (!Number.isFinite(n) || n < 1) return;
            queue({ op: 'answer-count', value: n });
        },
        close() {
            queue({ op: 'close-modal' });
        },
        open(npcName) {
            const wanted = String(npcName).toLowerCase();
            const npc = (snap().npcs || []).find(
                (row) =>
                    row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
            );
            if (!npc) {
                throw notImpl('Shop.open', 'npc not found');
            }
            const action = shopTradeAction(npc.actions);
            if (!action) {
                throw notImpl('Shop.open', 'no Trade or shop op');
            }
            queue({ op: 'npc', name: String(npcName), action });
        },
        sell() {
            throw notImpl('Shop.sell');
        },
        buyById() {
            throw notImpl('Shop.buyById');
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl('Shop.' + String(prop));
        },
    },
);
