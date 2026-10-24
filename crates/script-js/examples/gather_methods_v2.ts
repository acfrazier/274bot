type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Read-only gather fact. Not an iron action. Not F-PROOF. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const methods = api.gatherMethods();
    api.log(JSON.stringify(methods));
}
