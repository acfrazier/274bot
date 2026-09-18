// Host-posted herb pairs (`__rs2b0t_host.content.herbs`). Import resolves without
// touching the catalog; accessing HERBS/HERB_OPTIONS throws when a non-empty
// item catalog was posted but herb facts are missing (fail-closed at use).
const host = () => globalThis.__rs2b0t_host || {};

function herbRows() {
    const content = host().content;
    if (!content) return [];
    const itemsPosted = Array.isArray(content.items) && content.items.length > 0;
    const herbs = content.herbs;
    if (!Array.isArray(herbs) || herbs.length === 0) {
        if (itemsPosted) {
            throw new Error('selected-revision herb facts are required but missing or empty');
        }
        return [];
    }
    return herbs;
}

function proxyCatalog(rows) {
    return new Proxy(rows, {
        get(target, prop, receiver) {
            const value = Reflect.get(target, prop, receiver);
            if (typeof value === 'function') return value.bind(target);
            return value;
        },
    });
}

export const HERBS = new Proxy([], {
    get(_target, prop, receiver) {
        return Reflect.get(proxyCatalog(herbRows()), prop, receiver);
    },
});

export const HERB_OPTIONS = new Proxy([], {
    get(_target, prop, receiver) {
        const options = herbRows().map((herb) => herb.name);
        return Reflect.get(proxyCatalog(options), prop, receiver);
    },
});
