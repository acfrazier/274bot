// Thin reader/actions (ClientAdapter shape). The three stubs this tag
// read the host handle and fail closed (no snapshot); every other member
// throws `not v1` — never a fake value.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const proxy = (ns, members) =>
    new Proxy(members, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1(ns + '.' + String(prop));
        },
    });

export const reader = proxy('reader', {
    worldTile() {
        return host().tile || null;
    },
    inventorySize() {
        return typeof host().invSize === 'number' ? host().invSize : 0;
    },
});

export const actions = proxy('actions', {
    closeModal() {
        return false;
    },
});
