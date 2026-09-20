// Host-posted selected-revision drop names (`__rs2b0t_host.content.drop_db`).
// Import resolves without touching the catalog; reading DROP_DB throws when
// the host omitted the required four-card tables (fail-closed at use).
const host = () => globalThis.__rs2b0t_host || {};

function postedDropDb() {
    const content = host().content;
    const dropDb = content && content.drop_db;
    if (!dropDb || typeof dropDb !== 'object' || Array.isArray(dropDb) || Object.keys(dropDb).length === 0) {
        throw new Error('selected-revision drop facts are required but missing or empty');
    }
    return dropDb;
}

export const DROP_DB = new Proxy({}, {
    get(_target, prop) {
        const rows = postedDropDb();
        const value = Reflect.get(rows, prop, rows);
        if (typeof value === 'function') return value.bind(rows);
        return value;
    },
    ownKeys() {
        return Reflect.ownKeys(postedDropDb());
    },
    getOwnPropertyDescriptor(_target, prop) {
        return Reflect.getOwnPropertyDescriptor(postedDropDb(), prop);
    },
    has(_target, prop) {
        return Reflect.has(postedDropDb(), prop);
    },
});
