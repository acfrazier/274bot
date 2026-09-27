// Our EventSignal module: `pending()` is true while the posted snapshot's
// `hold` or `ours` is set (the guardian's freeze, or a detected event owned
// by this player) — the cooperative-interrupt signal rs2b0t scripts poll.
// `ignoredRandoms()` reads the bot instance's method (the rs2b0t
// `setIgnoredRandoms` source), default `[]`; the host reads the same list
// through its knock path so it skips act on those names.
// Every other member throws `not impl` — never a fake value.
const host = () => globalThis.__rs2b0t_host || {};
const notImpl = (name) => new Error('not impl: ' + name);

export const EventSignal = new Proxy(
    {
        pending() {
            const h = host();
            if (h.hold === true || h.ours === true) return true;
            const callback = globalThis.__rs2b0t_event_interrupt;
            return typeof callback === 'function' && callback() === true;
        },
        setInterrupt(callback) {
            if (callback !== null && callback !== undefined && typeof callback !== 'function') {
                throw new TypeError('EventSignal.setInterrupt requires a function or null');
            }
            globalThis.__rs2b0t_event_interrupt = callback || null;
        },
        ignoredRandoms() {
            const inst = globalThis.__rs_bot;
            const list =
                inst && typeof inst.ignoredRandoms === 'function'
                    ? inst.ignoredRandoms()
                    : [];
            return Array.isArray(list) ? list : [];
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl('EventSignal.' + String(prop));
        },
    },
);
