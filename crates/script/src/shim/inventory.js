// Our Inventory module: `count` reads the host-posted snapshot's inv rows
// (one `{name, count}` row per non-empty backpack slot) and sums the counts
// of the rows whose name matches (case-insensitive), like rs2b0t. A row
// whose obj name the host table does not know is posted with a null name
// and never matches; a missing snapshot reads 0 — never a fake value.
// Every other member throws `not v1`.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const snap = () => host().snapshot || {};

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
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Inventory.' + String(prop));
        },
    },
);
