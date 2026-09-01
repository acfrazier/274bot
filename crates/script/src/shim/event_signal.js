// Our EventSignal module: `pending()` is true while the posted snapshot's
// `hold` or `ours` is set (the guardian's freeze, or a detected event owned
// by this player) — the cooperative-interrupt signal rs2b0t scripts poll.
// `ignoredRandoms()` returns the host-posted last-known ignore list,
// default `[]` (the bot-instance source lands with the guardian wiring).
// Every other member throws `not v1` — never a fake value.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);

export const EventSignal = new Proxy(
    {
        pending() {
            const snap = host().snapshot || {};
            return snap.hold === true || snap.ours === true;
        },
        ignoredRandoms() {
            const list = host().ignoredRandoms;
            return Array.isArray(list) ? list : [];
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('EventSignal.' + String(prop));
        },
    },
);
