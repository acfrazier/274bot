// Host-posted selected herb pairs. Access fails closed when selected facts
// are active but the bounded herb table is missing.
const host = () => globalThis.__rs2b0t_host || {};

function herbRows() {
    const content = host().content;
    if (!content) return [];
    const selectedFacts = content.selected_facts === true;
    const herbs = content.herbs;
    if (!Array.isArray(herbs) || herbs.length === 0) {
        if (selectedFacts) {
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
