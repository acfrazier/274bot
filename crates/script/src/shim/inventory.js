// Our Inventory module: `count`/`first` read the host-posted snapshot's
// inv rows (one `{name, count}` row per non-empty backpack slot), like
// rs2b0t. `first` returns an item handle whose `interact(action)` queues
// a held-item op on `__rs2b0t_host.interact` (`{op:'held', name, action}`)
// for the host to dispatch through the slot Driver; `actions()` has no
// posted op labels and fails closed empty. A row whose obj name the host
// table does not know is posted with a null name and never matches; a
// missing snapshot reads 0 / null — never a fake value. Every other
// member throws `not v1`.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const snap = () => host().snapshot || {};
const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};

const held = (row) => ({
    name: row.name,
    count: row.count,
    interact(action) {
        queue({ op: 'held', name: row.name, action: String(action) });
        return true;
    },
    actions() {
        // The posted inv rows carry no op labels; the host resolves
        // the action at dispatch. Empty is fail-closed, not a guess.
        return [];
    },
});

export const Inventory = new Proxy(
    {
        count(name) {
            const wanted = String(name).toLowerCase();
            return (snap().inv || [])
                .filter(
                    (row) =>
                        row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
                )
                .reduce((sum, row) => sum + (typeof row.count === 'number' ? row.count : 0), 0);
        },
        first(name) {
            const wanted = String(name).toLowerCase();
            const row = (snap().inv || []).find(
                (r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted,
            );
            return row ? held(row) : null;
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Inventory.' + String(prop));
        },
    },
);
