import type { NativeApi } from '../host-js/index.d.ts';

/** Read-only keep predicate over caller names. Not a bank arm. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const kept = api.clue.keep({ name: 'Clue scroll', extra: ['Rune scimitar'] });
    api.log(JSON.stringify(kept));
}
