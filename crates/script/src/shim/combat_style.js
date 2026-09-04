// Combat-style constants ChickenKiller SETTINGS needs at eval time.
import { notImpl } from '../../shim/_kernel.js';

const COMBAT_STYLE = {
    attack: 'attack',
    accurate: 'attack',
    strength: 'strength',
    aggressive: 'strength',
    controlled: 'controlled',
    shared: 'controlled',
    defence: 'defence',
    defense: 'defence',
    defensive: 'defence',
};

export const COMBAT_STYLE_OPTIONS = ['attack', 'strength', 'controlled', 'defence'];
export const RANGE_STYLE_OPTIONS = ['accurate', 'rapid', 'longrange'];

export function parseCombatStyle(name) {
    const hit = COMBAT_STYLE[String(name).trim().toLowerCase()];
    if (!hit) throw notImpl('parseCombatStyle');
    return hit;
}

export function tryParseCombatStyle(name) {
    return COMBAT_STYLE[String(name).trim().toLowerCase()] ?? null;
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
    const n = String(name).trim().toLowerCase();
    const idx = RANGE_STYLE_OPTIONS.indexOf(n);
    if (idx < 0) throw notImpl('parseRangeStyle');
    return idx;
}

export function describeCombatStyle(resolution) {
    if (!resolution || typeof resolution.requested !== 'string') {
        throw notImpl('describeCombatStyle', 'missing requested');
    }
    return resolution.requested;
}
