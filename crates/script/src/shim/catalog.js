// notedOf / unnotedOf are built from posted inv+bank cert links (no ITEM_DB).
import { notV1 } from '../../shim/_kernel.js';

const EMPTY = {
    byId: new Map(),
    notedOf: new Map(),
    unnotedOf: new Map(),
    items: [],
    aliases: new Map(),
};

const host = () => globalThis.__rs2b0t_host || {};
const snap = () => host().snapshot || {};

function certMapsFromSnapshot() {
    const notedOf = new Map();
    const unnotedOf = new Map();
    for (const rows of [snap().inv || [], snap().bank || [], snap().bank_side || []]) {
        for (const row of rows) {
            if (!row || typeof row.id !== 'number') continue;
            const cert = row.cert ?? -1;
            if (cert >= 0) {
                notedOf.set(row.id, cert);
                unnotedOf.set(cert, row.id);
            }
        }
    }
    return { notedOf, unnotedOf };
}

export function liveCatalog() {
    const { notedOf, unnotedOf } = certMapsFromSnapshot();
    return { ...EMPTY, notedOf, unnotedOf };
}

export function clientName(_id) {
    throw notV1('clientName');
}

export function displayName(_id) {
    throw notV1('displayName');
}

export function notedId(id) {
    return liveCatalog().notedOf.get(id) ?? id;
}

export function unnotedId(id) {
    return liveCatalog().unnotedOf.get(id) ?? id;
}
