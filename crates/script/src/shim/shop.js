// Our Shop module. Open/buy/sell/close are one `shop` machine await each:
// Rust presses Trade, batches the Buy/Sell ops, owns every wait and reports
// the observed quantity. Rust calls `Shop.sell`'s optional pick callback over
// the same-name shop player rows. Reads shape the posted shop pages.
import { snap, notImpl, runMachine } from '../../shim/_kernel.js';

// What a call ended by a reset or a newer shop call resolves to.
const ABORTED = { open: false, buy: 0, sell: 0, close: undefined };

async function run(kind, name, qty, pick) {
    const out = await runMachine(
        'shop',
        { kind, name: name ?? '', qty: qty ?? 1 },
        { pick: typeof pick === 'function' ? pick : undefined },
    );
    if (out.kind === 'refused') throw notImpl('Shop.' + kind, out.reason);
    if (out.kind !== 'done') return ABORTED[kind];
    return out.value ?? undefined;
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
                slot: row?.slot ?? -1,
            }));
        },
        player() {
            if (!Shop.isOpen()) return [];
            return (snap().shop_player || []).map((row) => ({
                name: row?.name ?? null,
                count: row?.count ?? 0,
                slot: row?.slot ?? -1,
            }));
        },
        open(npcName) {
            return run('open', String(npcName ?? ''), 1);
        },
        buy(name, qty) {
            return run('buy', String(name ?? ''), qty ?? 1);
        },
        sell(name, qty, pick) {
            return run('sell', String(name ?? ''), qty ?? 1, pick);
        },
        close() {
            return run('close', '', 1);
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
