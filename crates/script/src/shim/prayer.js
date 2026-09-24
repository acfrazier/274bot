// Prayer: one Rust step machine per set/clear. The shim coerces the
// caller's `on` the frozen way and maps the settled envelope onto the
// declared boolean/void results; Rust clicks, waits and sweeps.
import { runMachine } from '../../shim/_kernel.js';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}

export const PROTECT_FROM_MAGIC = 'Protect from Magic';

function classifyOn(on) {
    if (on === undefined) return { kind: 'undefined' };
    if (typeof on === 'boolean') return { kind: 'boolean', value: on };
    return { kind: 'other', truthy: Boolean(on) };
}

export const Prayer = {
    points() {
        return call({ op: 'points' }).value;
    },
    max() {
        return call({ op: 'max' }).value;
    },
    full() {
        return call({ op: 'full' }).value;
    },
    known(name) {
        return call({ op: 'known', name: name.trim() }).value;
    },
    available(name) {
        return call({ op: 'available', name: name.trim() }).value;
    },
    active(name) {
        return call({ op: 'active', name: name.trim() }).value;
    },
    async set(name, on) {
        const out = await runMachine('prayer', {
            op: 'set',
            name: name.trim(),
            on: classifyOn(on),
        });
        return out.kind === 'done' && out.value.ok === true;
    },
    async clear() {
        await runMachine('prayer', { op: 'clear' });
    },
};
