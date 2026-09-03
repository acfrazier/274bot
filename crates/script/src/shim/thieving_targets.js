import Tile from '../../geometry/Tile.js';
import { host, notV1 } from '../../shim/_kernel.js';

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

export function targetSpot(target) {
    const bag = host().settingsBag || {};
    const spots = bag.campTiles || bag.spots;
    const row = spots && spots[target];
    if (row && row.anchor && typeof row.anchor.x === 'number') {
        return { anchor: Tile.from(row.anchor), leash: row.leash ?? 19 };
    }
    throw notV1('targetSpot');
}

export function requiredThieving(target) {
    const bag = host().settingsBag || {};
    const levels = bag.pickpocketLevels || bag.thievingLevels;
    if (levels && typeof levels[target] === 'number') {
        return levels[target];
    }
    throw notV1('requiredThieving');
}

export const HOSTILE_NAMES = ['Guard', 'Knight of Ardougne', 'Paladin', 'Hero'];

export function isHostileAttacker(_c, _maxDistance) {
    throw notV1('isHostileAttacker');
}

export function chooseTarget(candidatesNearestFirst, reachable) {
    for (const c of candidatesNearestFirst) {
        if (reachable(c)) {
            return { target: c, blocked: null };
        }
    }
    return { target: null, blocked: candidatesNearestFirst[0] ?? null };
}
