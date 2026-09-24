// Host-posted selected-revision drop names (`__rs2b0t_host.content.drop_db`).
// Import resolves without touching the catalog; reading DROP_DB throws when
// the host omitted the required four-card tables (fail-closed at use).
const host = () => globalThis.__rs2b0t_host || {};

let cached = null;

function postedDropDb() {
    const content = host().content;
    // One validation per posted content object: identity is the cache key.
    if (cached !== null && cached.content === content) {
        return cached.db;
    }
    const dropDb = content && content.drop_db;
    if (!dropDb || typeof dropDb !== 'object' || Array.isArray(dropDb) || Object.keys(dropDb).length === 0) {
        cached = null;
        throw new Error('selected-revision drop facts are required but missing or empty');
    }
    cached = { content, db: dropDb };
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
