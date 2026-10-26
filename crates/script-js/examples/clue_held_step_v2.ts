import type { NativeApi } from '../host-js/index.d.ts';

/** Read-only held-step identify over the posted pack page. Not a clue machine. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const step = api.clue.heldStep();
    api.log(JSON.stringify(step));
}
