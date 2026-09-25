import { runMachine } from '../../shim/_kernel.js';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_cake_stall(payload);
}

function callback(opts, name) {
    const fn = opts[name];
    return typeof fn === 'function' ? (...args) => fn.apply(opts, args) : undefined;
}

function booleanCallback(opts, name) {
    const fn = opts[name];
    return typeof fn === 'function' ? () => !!fn.call(opts) : undefined;
}

function lockoutTick(opts) {
    const value = opts.lockedOutUntil;
    const tick = typeof value === 'function' ? value.call(opts) : value;
    return typeof tick === 'number' && Number.isFinite(tick) ? tick : 0;
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
    const out = await runMachine(
        'cake_stall',
        { fill_to: fillTo },
        {
            abort: booleanCallback(opts, 'abort'),
            shouldEat: booleanCallback(opts, 'shouldEat'),
            lockedOutUntil: () => lockoutTick(opts),
            setStatus: callback(opts, 'setStatus'),
            log: callback(opts, 'log'),
            onSteal: callback(opts, 'onSteal'),
            onReset: callback(opts, 'onReset'),
        },
    );
    if (out.kind !== 'done') return 'aborted';
    return typeof out.value === 'string' ? out.value : 'no-progress';
}