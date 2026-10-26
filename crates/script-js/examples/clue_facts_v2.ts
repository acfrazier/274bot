import type { NativeApi } from '../host-js/index.d.ts';

/** Read-only clue membership row. Not a clue machine and not a solve. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const row = api.clue.row({ id: 3554 });
    api.log(JSON.stringify(row));
}
