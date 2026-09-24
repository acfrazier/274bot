// Materialized only when a catalog consumer reads ITEM_DB; the whole table is
// no longer posted or allocated in every isolate.
const selectedFacts = globalThis.__rs2b0t_selected_facts;
const rows = [];
let loaded = false;

function itemRows() {
    if (!loaded) {
        loaded = true;
        const selected = typeof selectedFacts === 'function' ? selectedFacts('item-db') : [];
        for (const row of selected) rows.push(row);
    }
    return rows;
}

export const ITEM_DB = new Proxy(rows, {
    get(_target, prop) {
        const value = Reflect.get(itemRows(), prop, itemRows());
        return typeof value === 'function' ? value.bind(itemRows()) : value;
    },
    set(_target, prop, value) {
        return Reflect.set(itemRows(), prop, value, itemRows());
    },
    has(_target, prop) {
        return Reflect.has(itemRows(), prop);
    },
    ownKeys() {
        return Reflect.ownKeys(itemRows());
    },
    getOwnPropertyDescriptor(_target, prop) {
        itemRows();
        return Reflect.getOwnPropertyDescriptor(rows, prop);
    },
});
