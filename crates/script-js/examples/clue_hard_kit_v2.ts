import type { NativeApi } from '../host-js/index.d.ts';

/** Read-only hard-kit status over caller facts. Not a clue machine. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const kit = api.clue.hardKit({
        attack: 60,
        lostCity: true,
        items: [
            { id: 1231, count: 1 },
            { id: 185, count: 1 },
            { id: 385, count: 15 },
        ],
    });
    api.log(JSON.stringify(kit));
}
