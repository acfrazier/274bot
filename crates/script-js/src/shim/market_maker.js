// MarketMaker helper surface is not impl — every member throws on use.
const notImpl = (name) => new Error('not impl: ' + name);

export const MarketMaker = new Proxy(
    {},
    {
        get(_target, prop) {
            if (typeof prop === 'symbol') return undefined;
            throw notImpl('MarketMaker.' + String(prop));
        },
    },
);
