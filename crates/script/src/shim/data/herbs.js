// Host-posted herb pairs (`__rs2b0t_host.content.herbs`). Empty when the
// isolate has no selected-revision content blob yet.
const host = () => globalThis.__rs2b0t_host || {};

export const HERBS = (host().content && host().content.herbs) || [];
export const HERB_OPTIONS = HERBS.map((herb) => herb.name);
