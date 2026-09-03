// QuestEngine is not impl — every member throws on use.
const notImpl = (name) => new Error('not impl: ' + name);

export const QuestEngine = new Proxy(
    {},
    {
        get(_target, prop) {
            if (typeof prop === 'symbol') return undefined;
            throw notImpl('QuestEngine.' + String(prop));
        },
    },
);
