import type { NativeApi, ScriptManifest } from '../host-js/index.d.ts';

/** Read-only pack targets over caller numbers. Not a clue machine. */
export const manifest = {
    apiVersion: 2,
    name: 'CluePackPlan',
    version: '1',
    author: '274bot',
    description: 'Logs the clue pack plan for fixed caller numbers. It takes no game action.',
    category: 'Examples',
    tags: ['clue'],
    settings: {},
    capabilities: [],
} as const satisfies ScriptManifest;

export function tick(api: NativeApi): void {
    const plan = api.clue.packPlan({
        hostWant: 20,
        freeSlots: 5,
        reserveSlots: 3,
        perCast: 3,
        weaponName: 'Rune scimitar',
        weaponInBackpack: true,
        weaponEquipped: false,
        casketAlias: 'trail_clue_hard_sextant001_casket',
    });
    api.log(JSON.stringify(plan));
}
