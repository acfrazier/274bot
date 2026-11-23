// MarketMaker helper surface is not impl — every member throws on use.
import { notImpl } from '../../shim/_kernel.js';

export const MarketMaker = new Proxy(
    {},
    {
        get(_target, prop) {
            if (typeof prop === 'symbol') return undefined;
            throw notImpl('MarketMaker.' + String(prop));
        },
    },
);
