type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Read-only quest fact. Not a colour witness. Not a points observation. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const row = api.questIdentity({ id: 'death' });
    api.log(JSON.stringify(row));
}
