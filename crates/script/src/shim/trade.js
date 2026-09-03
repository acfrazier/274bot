import { snap, queue, proxy, notImpl } from '../../shim/_kernel.js';

function rowList(key) {
    return snap()[key] || [];
}

function nameCountRows(key) {
    return rowList(key).map((row) => ({
        name: row?.name ?? null,
        count: row?.count ?? 0,
    }));
}

function findRows(key, name) {
    const wanted = String(name).toLowerCase();
    return rowList(key).filter(
        (row) => row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
    );
}

function pressRow(row) {
    if (!row || typeof row.component_id !== 'number' || row.component_id < 0) {
        throw notImpl('Trade');
    }
    queue({ op: 'if-button', component_id: row.component_id });
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
            queue({ op: 'player', name: String(playerName), action: 'Trade' });
        },
        offer(name) {
            const rows = findRows('trade_side', name);
            if (rows.length === 0) return;
            pressRow(rows[0]);
        },
        offerAll(name) {
            for (const row of findRows('trade_side', name)) {
                pressRow(row);
            }
        },
        removeAll(name) {
            for (const row of findRows('trade_mine', name)) {
                pressRow(row);
            }
        },
        accept() {
            const id = snap().trade_accept_id;
            if (typeof id !== 'number' || id < 0) {
                throw notImpl('Trade.accept');
            }
            queue({ op: 'if-button', component_id: id });
        },
        decline() {
            const id = snap().trade_decline_id;
            if (typeof id !== 'number' || id < 0) {
                throw notImpl('Trade.decline');
            }
            queue({ op: 'if-button', component_id: id });
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
