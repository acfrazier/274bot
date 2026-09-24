// Hunting rules: a name map, one Rust helper call each
// (`crate::hunt_catalog`), plus the declared constants.
import Tile from '../../../geometry/Tile.js';

const logic = (name, ...args) => globalThis.__rs2b0t_hunt_logic(name, ...args);

export const SAFESPOT_BLIND_MS = 20_000;
export const PROTECT_FROM_MELEE = 'Protect from Melee';
export const PRAYER_SIP_FLOOR = 8;
export const PRAYER_SIP_FRACTION = 0.15;
export const LOOT_REACH = 10;
export const LOOT_REACH_OPEN = 14;
export const SHIELD_ABSORBS = /your shield absorbs most of the dragon fire/i;
export const POTION_PROTECTS = /your potion protects you from the heat/i;
export const ANTIFIRE_TICKS = 600;
export const ANTIFIRE_MARGIN_TICKS = 20;

export const nextSafespot = (s) => logic('nextSafespot', s);
export const hurtOnSpot = (s) => logic('hurtOnSpot', s);
export const retreatDue = (s) => logic('retreatDue', s);
export const lootHalts = (s) => logic('lootHalts', s);
export const holdDue = (s) => logic('holdDue', s);
export const nearestSpot = (from, spots) => logic('nearestSpot', from, spots);
export const bodyOrigin = (tile, size) => logic('bodyOrigin', tile, size);
export const noteSighting = (prev, tile, now) => logic('noteSighting', prev, tile, now);
export const settled = (s, now, ms) => logic('settled', s, now, ms);
export const retreatAim = (a) => logic('retreatAim', a);
export const chaseMode = (style, fireAtRange) => logic('chaseMode', style, fireAtRange);
export const prayerFor = (style, fireAtRange) => logic('prayerFor', style, fireAtRange);
export const prayerSipDue = (points, max) => logic('prayerSipDue', points, max);
export const lootReach = (fireAtRange) => logic('lootReach', fireAtRange);
export const attackRangeFor = (style) => logic('attackRangeFor', style);
export const engageRangeFor = (style) => logic('engageRangeFor', style);
export const gapTo = (from, tile, size) => logic('gapTo', from, tile, size);
export const shieldGate = (style, fireAtRange, hasShield) => logic('shieldGate', style, fireAtRange, hasShield);
export const styleGate = (style, fireAtRange) => logic('styleGate', style, fireAtRange);
export const antifireDue = (s) => logic('antifireDue', s);
export const antifireLapsed = (sawShield, sawPotion) => logic('antifireLapsed', sawShield, sawPotion);
export const nextApproachIndex = (stops, here) => logic('nextApproachIndex', stops, here);
export const isClueObj = (id) => logic('isClueObj', id);
export const keyStatus = (held, banked) => logic('keyStatus', held, banked);

// The loot filter crosses as plain data (its Set as a list).
export function wantsDrop(item, f) {
    return logic('wantsDrop', item, { ...f, loot: [...(f.loot ?? [])] });
}

// Caller-argument plumbing over the script's own settings objects.
export function siteTileOf(schema, bag, key, site) {
    const def = key === undefined ? undefined : schema[key]?.default;
    if (key === undefined || !(def instanceof Tile)) return site;
    const set = bag.tile(key, site);
    return set.equals(def) ? site : set;
}

export function keepDoses(potionDoses, antipoisonDoses, carriesAntipoison) {
    return carriesAntipoison ? [...potionDoses, ...antipoisonDoses] : [...potionDoses];
}
