// liveCatalog is empty / fail-closed until a real market feed exists.
const EMPTY = {
    byId: new Map(),
    notedOf: new Map(),
    unnotedOf: new Map(),
    items: [],
    aliases: new Map(),
};

export function liveCatalog() {
    return EMPTY;
}

export function clientName(_id) {
    return '';
}

export function displayName(_id) {
    return '';
}

export function notedId(id) {
    return id;
}

export function unnotedId(id) {
    return id;
}
