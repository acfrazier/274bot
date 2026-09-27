// Frozen `src/bot/api/combat/rangedSettings.ts:3-14`: static names and one
// typed local Rust calculation; no settings policy lives in the shim.
export const CUSTOM_RANGED_SETTINGS = {
    customBow: { type: 'string', default: '', label: 'Custom ranged weapon', group: 'Combat', showIf: { key: 'bow', anyOf: ['Other'] }, help: 'exact item name; for thrown weapons use the same name for ammo' },
    customAmmo: { type: 'string', default: '', label: 'Custom ammunition', group: 'Combat', showIf: { key: 'ammo', anyOf: ['Other'] }, help: 'exact item name, such as Bolts or Rune knife' },
};

export function rangedItem(settings, key, fallback) {
    return globalThis.__rs2b0t_selected_facts(
        'ranged-item',
        settings.str(key, fallback),
        settings.str(key === 'bow' ? 'customBow' : 'customAmmo', ''),
        fallback,
        key,
    );
}
