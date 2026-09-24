// notedOf / unnotedOf are selected-revision certificate links from Rust.
import { notImpl } from '../../shim/_kernel.js';

const EMPTY = {
    byId: new Map(),
    notedOf: new Map(),
    unnotedOf: new Map(),
    items: [],
    aliases: new Map(),
};

let cachedCatalog = null;

export function liveCatalog() {
    if (cachedCatalog) return cachedCatalog;
    const fn = globalThis.__rs2b0t_selected_facts;
    if (typeof fn !== 'function') return { ...EMPTY };
    const maps = fn('cert-maps');
    cachedCatalog = {
        ...EMPTY,
        notedOf: new Map(maps.notedOf),
        unnotedOf: new Map(maps.unnotedOf),
    };
    return cachedCatalog;
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

function certLink(id, direction) {
    const fn = globalThis.__rs2b0t_selected_facts;
    if (typeof fn !== 'function') return undefined;
    const n = fn('cert-link', id, direction);
    return n == null ? undefined : n;
}

export function notedId(id) {
    const n = certLink(id, 'noted');
    if (n === undefined) throw notImpl('notedId');
    return n;
}

export function unnotedId(id) {
    const n = certLink(id, 'unnoted');
    if (n === undefined) throw notImpl('unnotedId');
    return n;
}
