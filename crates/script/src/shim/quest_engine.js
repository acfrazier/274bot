// QuestEngine is not v1 — every member throws on use.
const notV1 = (name) => new Error('not v1: ' + name);

export const QuestEngine = new Proxy(
    {},
    {
        get(_target, prop) {
            if (typeof prop === 'symbol') return undefined;
            throw notV1('QuestEngine.' + String(prop));
        },
    },
);
