import Tile from '../../geometry/Tile.js';
import { PICKPOCKET_TARGETS } from '../../data/pickpocketTargets.js';

const SPOTS = {
    Guard: { anchor: new Tile(2661, 3306, 0), leash: 19 },
    'Knight of Ardougne': { anchor: new Tile(2661, 3306, 0), leash: 29 },
    Paladin: { anchor: new Tile(2655, 3311, 0), leash: 12 },
    Hero: { anchor: new Tile(2657, 3311, 0), leash: 17 },
};

export function targetSpot(target) {
    return SPOTS[target] ?? SPOTS.Guard;
}

export function requiredThieving(target) {
    return PICKPOCKET_TARGETS.find((t) => t.name === target)?.level ?? 1;
}

export const HOSTILE_NAMES = ['Guard', 'Knight of Ardougne', 'Paladin', 'Hero'];

export function isHostileAttacker(c, maxDistance) {
    return (
        c.name !== null &&
        HOSTILE_NAMES.includes(c.name) &&
        c.inCombat &&
        !c.targetsAnotherPlayer &&
        c.distance <= maxDistance &&
        c.actions.includes('Attack')
    );
}

export function chooseTarget(candidatesNearestFirst, reachable) {
    for (const c of candidatesNearestFirst) {
        if (reachable(c)) {
            return { target: c, blocked: null };
        }
    }
    return { target: null, blocked: candidatesNearestFirst[0] ?? null };
}
