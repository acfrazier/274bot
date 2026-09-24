// Frozen `api/market/catalog.ts` names over the selected-revision item facts
// Rust derives from `SelectedGameData::items` (`load/selected_facts_v8.rs`,
// `market_catalog.rs`). A `cat` argument is the one catalog Rust answers for.
import { notImpl } from '../../shim/_kernel.js';

const facts = (...args) => globalThis.__rs2b0t_selected_facts(...args);

function selected(name, value) {
    if (value === undefined) throw notImpl(name, 'no selected game data');
    return value;
}

let cachedCatalog = null;

export function liveCatalog() {
    if (cachedCatalog) return cachedCatalog;
    const maps = facts('cert-maps');
    // byId / items / aliases are one Rust build on first read, so a card that
    // only reads the cert maps never materializes the record rows.
    let objs = null;
    const objCatalog = () => (objs ??= facts('obj-catalog'));
    cachedCatalog = {
        notedOf: new Map(maps.notedOf),
        unnotedOf: new Map(maps.unnotedOf),
        get byId() {
            return objCatalog().byId;
        },
        get items() {
            return objCatalog().items;
        },
        get aliases() {
            return objCatalog().aliases;
        },
    };
    return cachedCatalog;
}

export function tradeable(id) {
    return selected('tradeable', facts('item-tradeable', id));
}

export function clientName(_cat, id) {
    return selected('clientName', facts('item-name', id, 'client')) ?? undefined;
}

export function displayName(_cat, id) {
    return selected('displayName', facts('item-name', id, 'display'));
}

export function notedId(_cat, id) {
    return selected('notedId', facts('cert-link', id, 'noted'));
}

export function unnotedId(_cat, id) {
    return selected('unnotedId', facts('cert-link', id, 'unnoted')) ?? id;
}
