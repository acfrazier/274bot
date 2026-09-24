// Host-posted bounded shop records. Access fails closed when selected facts
// are active but the supported shop table is missing. Unpublished keepers
// remain absent.
const host = () => globalThis.__rs2b0t_host || {};

function shopRows() {
    const content = host().content;
    if (!content) return {};
    const selectedFacts = content.selected_facts === true;
    const shops = content.shops;
    if (!shops || typeof shops !== 'object' || Object.keys(shops).length === 0) {
        if (selectedFacts) {
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
