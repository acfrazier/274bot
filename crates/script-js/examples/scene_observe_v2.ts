type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Read-only scene projection. Not an iron action. Not F-PROOF. Not a Mine. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    // Both calls are historical copies of the posted page. No scene is posted
    // here, so both fail closed.
    const locs = api.sceneLocs({ ids: [2092], limit: 8 });
    api.log(JSON.stringify(locs));
    // -1 is the posted absent NPC type. It is a legal caller integer and it
    // matches only a posted -1 row.
    const npcs = api.sceneNpcs({ types: [-1], actions: ['Attack'], limit: 8 });
    api.log(JSON.stringify(npcs));
}
