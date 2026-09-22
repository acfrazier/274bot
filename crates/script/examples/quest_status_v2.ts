type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Read-only posted quest-tab copy. Not an iron action. Not F-PROOF. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    // No page is posted here, so this fails closed with snapshot-unavailable
    // rather than inventing a status. The same call after `post_base` is
    // quest-tab-unbound: a posted null tab is not a missing page.
    const row = api.questStatus({ name: 'Death Plateau' });
    api.log(JSON.stringify(row));
}
