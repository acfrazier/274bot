// Host-posted bounded shop records. Access fails closed when selected facts
// are active but the supported shop table is missing. Unpublished keepers
// remain absent.
const host = () => globalThis.__rs2b0t_host || {};

let cached = null;

function shopRows() {
    const content = host().content;
    // One validation per posted content object: identity is the cache key.
    if (cached !== null && cached.content === content) {
        return cached.rows;
    }
    if (!content) {
        cached = { content, rows: {} };
        return cached.rows;
    }
    const selectedFacts = content.selected_facts === true;
    const shops = content.shops;
    if (!shops || typeof shops !== 'object' || Object.keys(shops).length === 0) {
        if (selectedFacts) {
            cached = null;
            throw new Error('selected-revision shop facts are required but missing or empty');
        }
        cached = { content, rows: {} };
        return cached.rows;
    }
    cached = { content, rows: shops };
    return shops;
}

export const SHOP_DB = new Proxy(
    {},
    {
        ownKeys() {
            return Reflect.ownKeys(shopRows());
        },
        getOwnPropertyDescriptor(_target, prop) {
            const desc = Object.getOwnPropertyDescriptor(shopRows(), prop);
            if (!desc) return undefined;
            return { ...desc, configurable: true, enumerable: true };
        },
        get(_target, prop) {
            if (typeof prop === 'symbol') return undefined;
            return shopRows()[prop];
        },
        has(_target, prop) {
            return Object.prototype.hasOwnProperty.call(shopRows(), prop);
        },
    },
);
