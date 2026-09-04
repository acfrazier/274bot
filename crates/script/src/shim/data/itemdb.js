// Host-posted obj rows (`__rs2b0t_host.content.items`). Empty when the
// isolate has no content blob yet. Not a cloned catalog database.
const host = () => globalThis.__rs2b0t_host || {};

export const ITEM_DB = (host().content && host().content.items) || [];
