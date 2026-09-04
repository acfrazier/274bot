import Tile from '../../geometry/Tile.js';
import { host, notImpl } from '../../shim/_kernel.js';

export const PICKPOCKET_TARGET_NAMES = [
    'Man',
    'Woman',
    'Farmer',
    'Warrior woman',
    'Al-Kharid warrior',
    'Rogue',
    'Guard',
    'Knight of Ardougne',
    'Watchman',
    'Paladin',
    'Hero',
];
export const ARDOUGNE_PICKPOCKET_TARGETS = ['Guard', 'Knight of Ardougne', 'Paladin', 'Hero'];

function pickpocketSpots() {
    return (host().content && host().content.pickpocket_spots) || [];
}

function spotRow(target) {
    const spots = pickpocketSpots();
    if (!spots.length) {
        return null;
    }
    const want = String(target || '').trim().toLowerCase();
    const hit = spots.find((p) => String(p.name).toLowerCase() === want);
    if (hit) {
        return hit;
    }
    return spots.find((p) => String(p.name).toLowerCase() === 'guard') || spots[0];
}

export function targetSpot(target) {
    const bag = host().settingsBag || {};
    const spots = bag.campTiles || bag.spots;
    const row = spots && spots[target];
    if (row && row.anchor && typeof row.anchor.x === 'number') {
        return { anchor: Tile.from(row.anchor), leash: row.leash ?? 19 };
    }
    const posted = spotRow(target);
    if (!posted || typeof posted.x !== 'number' || typeof posted.z !== 'number') {
        throw notImpl('targetSpot');
    }
    return {
        anchor: new Tile(posted.x, posted.z, posted.level ?? 0),
        leash: posted.leash ?? 19,
    };
}

export function requiredThieving(target) {
    const bag = host().settingsBag || {};
    const levels = bag.pickpocketLevels || bag.thievingLevels;
    if (levels && typeof levels[target] === 'number') {
        return levels[target];
    }
    const posted = spotRow(target);
    if (!posted || typeof posted.required_thieving !== 'number') {
        throw notImpl('requiredThieving');
    }
    return posted.required_thieving;
}

export const HOSTILE_NAMES = [];

export function isHostileAttacker(_c, _maxDistance) {
    throw notImpl('isHostileAttacker');
}

export function chooseTarget(candidatesNearestFirst, reachable) {
    for (const c of candidatesNearestFirst) {
        if (reachable(c)) {
            return { target: c, blocked: null };
        }
    }
    return { target: null, blocked: candidatesNearestFirst[0] ?? null };
}
