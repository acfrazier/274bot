// Combat-style constants ChickenKiller SETTINGS needs at eval time; the style
// tables are Rust's (`load/combat_style_v8.rs`).

export const COMBAT_STYLE_OPTIONS = ['attack', 'strength', 'controlled', 'defence'];
export const RANGE_STYLE_OPTIONS = ['accurate', 'rapid', 'longrange'];

export function parseCombatStyle(name) {
    return globalThis.__rs2b0t_parse_combat_style(String(name));
}

export function tryParseCombatStyle(name) {
    return globalThis.__rs2b0t_try_parse_combat_style(String(name));
}

/** SETTINGS split: a leftover melee token in combatStyle is kind melee. */
export function resolveSplitCombatSettings(rawCombatStyle, rawMeleeStyle) {
    const legacy = tryParseCombatStyle(rawCombatStyle);
    if (legacy !== null) {
        return {
            kind: 'melee',
            meleeStyle: rawMeleeStyle !== undefined ? parseCombatStyle(rawMeleeStyle) : legacy,
            legacyMigrated: rawMeleeStyle === undefined ? legacy : null,
        };
    }
    const kindRaw = String(rawCombatStyle || '')
        .trim()
        .toLowerCase();
    const kind = kindRaw === 'mage' || kindRaw === 'range' ? kindRaw : 'melee';
    return {
        kind,
        meleeStyle: parseCombatStyle(rawMeleeStyle ?? 'strength'),
        legacyMigrated: null,
    };
}

export function parseRangeStyle(name) {
    return globalThis.__rs2b0t_parse_range_style(String(name));
}

export function describeCombatStyle(resolution) {
    return globalThis.__rs2b0t_describe_combat_style(resolution);
}
