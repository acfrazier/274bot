// Name-map Trade onto the Rust-owned request/offer/remove/accept/decline
// sequence. This shim marshals the call argument, runs the optional pick
// callback as a projection, and dispatches returned verbs. It does not
// choose rows, wait the count dialog, or treat a queued click as a transfer.
import { snap, queue, notImpl } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';

function callTrade(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_trade
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('Trade');
    }
    return fn(payload);
}

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

function dispatchVerb(step) {
    for (const op of step.ops || []) queue(op);
}

function settle(kind, step) {
    if (kind === 'decline') return undefined;
    return step.result === true;
}

function abortedResult(kind) {
    if (kind === 'decline') return undefined;
    return false;
}

function projectPick(candidates, pick) {
    if (!Array.isArray(candidates) || candidates.length === 0) return null;
    if (typeof pick !== 'function') return candidates[0];
    return candidates.find(pick) || null;
}

async function run(kind, name, n, pick) {
    const begin = callTrade({
        op: 'begin',
        kind,
        name: name ?? '',
        n: n ?? 0,
    });
    if (!begin) return abortedResult(kind);
    let current = begin;
    if (current.kind === 'candidates') {
        const chosen = projectPick(current.candidates, pick);
        if (!chosen) {
            callTrade({ op: 'select', token: current.token });
            return abortedResult(kind);
        }
        current = callTrade({
            op: 'select',
            token: current.token,
            id: chosen.id,
            slot: chosen.slot,
        });
    }
    if (!current) return abortedResult(kind);
    if (current.kind === 'notImpl') throw notImpl('Trade.' + kind, current.reason);
    if (current.kind === 'aborted') return abortedResult(kind);
    if (current.kind === 'done') {
        dispatchVerb(current);
        return settle(kind, current);
    }
    const token = current.token;
    while (current) {
        if (current.kind === 'done') {
            dispatchVerb(current);
            return settle(kind, current);
        }
        if (current.kind === 'aborted') return abortedResult(kind);
        if (current.kind === 'notImpl') throw notImpl('Trade.' + kind, current.reason);
        if (current.kind === 'ops') {
            dispatchVerb(current);
        } else if (current.kind !== 'wait') {
            return abortedResult(kind);
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callTrade({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return abortedResult(kind);
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
