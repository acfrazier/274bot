// Our Shop module. Open/buy/sell/close are Rust-owned: this shim marshals the
// call argument, dispatches the verbs the runtime returns and reports the
// observed quantity. It never batches ops, picks rows or decides a wait.
import { snap, queue, notImpl } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';

function callShop(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_shop
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('Shop');
    }
    return fn(payload);
}

function dispatchVerb(step) {
    if (step.kind === 'npc') {
        queue({ op: 'npc', name: step.name, action: step.action });
        return true;
    }
    if (step.kind === 'close-modal') {
        queue({ op: 'close-modal' });
        return true;
    }
    if (step.kind === 'ops') {
        for (const op of step.ops || []) queue(op);
        return true;
    }
    return false;
}

// The frozen signatures: `open` resolves a boolean, `buy`/`sell` resolve the
// observed count and `close` resolves void.
function settle(kind, step) {
    if (kind === 'open') return step.result === true;
    if (kind === 'close') return undefined;
    return typeof step.quantity === 'number' ? step.quantity : 0;
}

function abortedResult(kind) {
    if (kind === 'open') return false;
    if (kind === 'close') return undefined;
    return 0;
}

// One call's whole sequence: dispatch every returned verb and park between
// waits. The runtime owns each deadline, so a stalled call still ends.
async function run(kind, name, qty, pick) {
    if (pick !== undefined && pick !== null) {
        // The frozen signature's same-name pack selector is not mapped: the
        // Rust runtime resolves rows itself and must not guess a caller row.
        throw notImpl('Shop.' + kind, 'pick unsupported');
    }
    const begin = callShop({ op: 'begin', kind, name: name ?? '', qty: qty ?? 1 });
    if (!begin) return abortedResult(kind);
    if (begin.kind === 'notImpl') throw notImpl('Shop.' + kind, begin.reason);
    if (begin.kind === 'aborted') return abortedResult(kind);
    if (begin.kind === 'done') return settle(kind, begin);
    const token = begin.token;
    let current = begin;
    while (current) {
        if (current.kind === 'done') return settle(kind, current);
        if (current.kind === 'aborted') return abortedResult(kind);
        if (current.kind === 'notImpl') throw notImpl('Shop.' + kind, current.reason);
        if (current.kind === 'ops' || current.kind === 'npc' || current.kind === 'close-modal') {
            dispatchVerb(current);
        } else if (current.kind !== 'wait') {
            return abortedResult(kind);
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callShop({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return abortedResult(kind);
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
