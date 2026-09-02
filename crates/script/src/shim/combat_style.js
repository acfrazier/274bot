// Combat-style constants ChickenKiller SETTINGS needs at eval time.
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
    return COMBAT_STYLE[String(name).trim().toLowerCase()] ?? 'strength';
}

export function tryParseCombatStyle(name) {
    return COMBAT_STYLE[String(name).trim().toLowerCase()] ?? null;
}

export function parseRangeStyle(name) {
    const n = String(name).trim().toLowerCase();
    const idx = RANGE_STYLE_OPTIONS.indexOf(n);
    return idx >= 0 ? idx : 0;
}

export function describeCombatStyle(resolution) {
    return resolution?.requested ?? 'strength';
}
