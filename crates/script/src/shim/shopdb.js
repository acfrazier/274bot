// Host-posted shop records (`__rs2b0t_host.content.shops`). Import resolves
// without touching the catalog; Object.values/lookup throw when a non-empty
// item catalog was posted but shop facts are missing (fail-closed at use).
// Only the shops the host posted exist here — unpublished keepers stay absent.
const host = () => globalThis.__rs2b0t_host || {};

function shopRows() {
    const content = host().content;
    if (!content) return {};
    const itemsPosted = Array.isArray(content.items) && content.items.length > 0;
    const shops = content.shops;
    if (!shops || typeof shops !== 'object' || Object.keys(shops).length === 0) {
        if (itemsPosted) {
            throw new Error('selected-revision shop facts are required but missing or empty');
        }
        return {};
    }
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
