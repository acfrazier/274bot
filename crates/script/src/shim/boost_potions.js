// Boost-potion descriptors, planning and sip selection.
//
// The descriptor rows, the planning defaults, the boost floor and every
// selection decision live in `script::boost_potions`. Each export is one
// native call (`__rs2b0t_boost_potions`); Rust walks the caller's own carry
// list / plans and calls its `levels` / `held` callbacks in the frozen order.
function native(op, ...args) {
    return globalThis.__rs2b0t_boost_potions(op, ...args);
}

const TABLE = native('table');

export const BOOST_POTIONS = TABLE.potions;
export const SUPER_ATTACK = BOOST_POTIONS[0];
export const SUPER_STRENGTH = BOOST_POTIONS[1];
export const EMPTY_VIAL = TABLE.empty_vial;
export const BOOST_FLOOR = TABLE.floor;

/** Whether the boost has decayed back into the floor band. */
export function boostFaded(base, effective, floor) {
    return native('boostFaded', base, effective, floor);
}

/** The potions to carry, taking the dose form and count from the loadout and falling back to one 3-dose flask of each. */
export function plannedPotions(carry) {
    return native('plannedPotions', carry, BOOST_POTIONS);
}

/** The one potion to drink this tick, or null. */
export function potionToSip(s) {
    return native('potionToSip', s);
}
