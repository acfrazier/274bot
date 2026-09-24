// notedOf / unnotedOf are selected-revision certificate links from Rust.
import { notImpl } from '../../shim/_kernel.js';

const EMPTY = {
    byId: new Map(),
    notedOf: new Map(),
    unnotedOf: new Map(),
    items: [],
    aliases: new Map(),
};

export function liveCatalog() {
    const fn = globalThis.__rs2b0t_selected_facts;
    if (typeof fn !== 'function') return { ...EMPTY };
    const maps = fn('cert-maps');
    return {
        ...EMPTY,
        notedOf: new Map(maps.notedOf),
        unnotedOf: new Map(maps.unnotedOf),
    };
}

export function tradeable(_id) {
    throw notImpl('tradeable', 'untradeable ids are not in selected facts');
}

export function clientName(_id) {
    throw notImpl('clientName');
}

export function displayName(_id) {
    throw notImpl('displayName');
}

export function notedId(id) {
    const n = liveCatalog().notedOf.get(id);
    if (n === undefined) throw notImpl('notedId');
    return n;
}

export function unnotedId(id) {
    const n = liveCatalog().unnotedOf.get(id);
    if (n === undefined) throw notImpl('unnotedId');
    return n;
}
