// Name-map: marshal logName, dispatch tinderbox use-on, report lit/blocked/stalled.
// Rust owns the start/light tick waits and the XP / cannot-light outcome.
import { Execution } from '../execution/Execution.js';
import { queue, notImpl } from '../../shim/_kernel.js';

function callFire(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_fire
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('lightFire');
    }
    return fn(payload);
}

function dispatchVerb(step) {
    if (step.kind !== 'ops') return false;
    for (const op of step.ops || []) queue(op);
    return true;
}

function settle(step) {
    if (typeof step.result === 'string') return step.result;
    return 'stalled';
}

export async function lightFire(logName) {
    const begin = callFire({ op: 'begin', logName: String(logName ?? '') });
    if (!begin) return 'stalled';
    if (begin.kind === 'notImpl') throw notImpl('lightFire', begin.reason);
    if (begin.kind === 'aborted') return 'stalled';
    if (begin.kind === 'done') return settle(begin);
    const token = begin.token;
    let current = begin;
    while (current) {
        if (current.kind === 'done') return settle(current);
        if (current.kind === 'aborted') return 'stalled';
        if (current.kind === 'notImpl') throw notImpl('lightFire', current.reason);
        if (current.kind === 'ops') {
            dispatchVerb(current);
        } else if (current.kind !== 'wait') {
            return 'stalled';
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callFire({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return 'stalled';
}
