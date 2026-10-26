// Name-map `meleeWeapons`: the weapon list (selected `melee_weapons` facts),
// wield levels, tier and stab/slash orders live in `script::melee_weapons`.

/** The best melee weapon among `available` the character can wield, or null. */
export function bestMeleeWeapon(available, pick) {
    const unusable = pick.unusable ? [...pick.unusable] : [];
    return globalThis.__rs2b0t_melee_weapon('best', available, pick.attack, pick.preferStab, unusable);
}

/** The melee weapon among `names` that the list knows, or null. */
export function knownMeleeWeapon(names) {
    return globalThis.__rs2b0t_melee_weapon('known', names);
}
