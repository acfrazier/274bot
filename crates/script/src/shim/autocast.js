// Autocast: one Rust step machine per arm. Rust owns the four presses, the
// varp waits, the deadlines and every reason's log line; the shim maps the
// settled outcome onto the declared boolean (or the frozen `not impl`).
import { notImpl, runMachine } from '../../shim/_kernel.js';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_autocast(payload);
}

function controls() {
    return call({ op: 'controls' });
}

function live() {
    return call({ op: 'observe' });
}

export const Autocast = {
    armed() {
        const c = controls();
        if (!c || c.available !== true) return false;
        return live().armed === true;
    },
    staffTabAttached() {
        const c = controls();
        if (!c || c.available !== true) return false;
        return live().staff_attached === true;
    },
    async arm(spellName, log) {
        const out = await runMachine('autocast', { spell: String(spellName) });
        if (out.kind !== 'done') return false;
        if (out.value.message) log?.(out.value.message);
        if (out.value.not_impl) throw notImpl('Autocast.arm', out.value.not_impl);
        return out.value.ok === true;
    },
};
