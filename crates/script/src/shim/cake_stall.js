import { Execution } from '../execution/Execution.js';
import { host, queue } from '../../shim/_kernel.js';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_cake_stall(payload);
}

function called(fn) {
    return typeof fn === 'function' && !!fn();
}

function lockoutTick(opts) {
    const value = opts && opts.lockedOutUntil;
    if (typeof value === 'function') {
        const n = value();
        return typeof n === 'number' && Number.isFinite(n) ? n : 0;
    }
    if (typeof value === 'number' && Number.isFinite(value)) {
        return value;
    }
    return 0;
}

function callbackResults(opts, withCallbacks, withLockout) {
    const h = host();
    const abort = withCallbacks && called(opts?.abort);
    const shouldEat = withCallbacks && !abort ? called(opts?.shouldEat) : false;
    return {
        abort,
        should_eat: shouldEat,
        facts_valid: h.content?.baker_stall != null,
        ...(withLockout ? { locked_out_until: lockoutTick(opts) } : {}),
    };
}

function dispatchStep(step) {
    if (step.kind === 'walk-to') {
        queue({ op: 'walk-to', x: step.x, z: step.z, level: step.level });
        return true;
    }
    if (step.kind === 'loc') {
        queue({
            op: 'loc',
            x: step.x,
            z: step.z,
            level: step.level,
            action: step.action,
            id: step.id,
        });
        return true;
    }
    return step.kind === 'wait';
}

export function carriedCakes() {
    return call({ op: 'count' });
}

export function needsCakeRestock(target) {
    const normalized = typeof target === 'number' && Number.isFinite(target) ? target : null;
    return call({
        op: 'needs_restock',
        target: normalized,
    });
}

export async function stealCakes(opts = {}) {
    const fillTo =
        typeof opts.fillTo === 'number' && Number.isFinite(opts.fillTo) ? opts.fillTo : null;
    let step = call({
        op: 'begin',
        fill_to: fillTo,
        ...callbackResults(opts, true, false),
    });
    const token = step?.token;
    while (step && step.kind !== 'done' && step.kind !== 'aborted') {
        if (step.kind === 'observe') {
            step = call({
                op: 'next',
                token,
                ...callbackResults(opts, step.callbacks === true, step.lockout === true),
            });
            continue;
        }
        if (step.kind === 'on-steal') {
            if (typeof opts.onSteal === 'function') {
                opts.onSteal();
            }
            step = call({
                op: 'next',
                token,
                ...callbackResults(opts, false, false),
            });
            continue;
        }
        if (!dispatchStep(step)) {
            return 'no-progress';
        }
        const withCallbacks = step.kind === 'loc';
        let next = null;
        await Execution.delayUntil(() => {
            next = call({
                op: 'next',
                token,
                ...callbackResults(opts, withCallbacks, false),
            });
            return next?.kind !== 'wait';
        }, 0);
        step = next;
    }
    if (step?.kind === 'done') {
        return typeof step.result === 'string' ? step.result : 'no-progress';
    }
    return 'aborted';
}