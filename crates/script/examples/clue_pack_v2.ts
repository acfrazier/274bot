import type { NativeApi } from '../host-js/index.d.ts';

/** Read-only pack targets over caller numbers. Not a clue machine. */
export const apiVersion = 2;

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
