// Name-map Trade onto the Rust-owned request/offer/remove/accept/decline
// machine. Rust calls the optional pick callback over the same-name side
// rows, waits the count dialog and never treats a queued click as a
// transfer.
import { snap, notImpl, runMachine } from '../../shim/_kernel.js';

function rowList(key) {
    return snap()[key] || [];
}

function nameCountRows(key) {
    return rowList(key).map((row) => ({
        id: row?.id ?? 0,
        name: row?.name ?? null,
        count: row?.count ?? 0,
    }));
}

async function run(kind, name, n, pick) {
    const out = await runMachine(
        'trade',
        { kind, name: name ?? '', n: n ?? 0 },
        { pick: typeof pick === 'function' ? pick : undefined },
    );
    if (out.kind === 'refused') throw notImpl('Trade.' + kind, out.reason);
    if (out.kind !== 'done') return kind === 'decline' ? undefined : false;
    return out.value ?? undefined;
}

export const Trade = new Proxy(
    {
        active() {
            return snap().trade_offer_open === true || snap().trade_confirm_open === true;
        },
        onOfferScreen() {
            return snap().trade_offer_open === true;
        },
        onConfirmScreen() {
            return snap().trade_confirm_open === true;
        },
        partner() {
            const p = snap().trade_partner;
            return typeof p === 'string' && p.length > 0 ? p : null;
        },
        myOffer() {
            return nameCountRows('trade_mine');
        },
        theirOffer() {
            return nameCountRows('trade_theirs');
        },
        request(playerName) {
            return run('request', String(playerName ?? ''), 0);
        },
        offerAll(name, pick) {
            return run('offerAll', String(name ?? ''), 0, pick);
        },
        offer(name, n, pick) {
            return run('offer', String(name ?? ''), n ?? 0, pick);
        },
        removeAll() {
            return run('removeAll', '', 0);
        },
        accept() {
            return run('accept', '', 0);
        },
        decline() {
            return run('decline', '', 0);
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl('Trade.' + String(prop));
        },
    },
);
