// Catalog Firemaking URL: SETTINGS keys + lightFire. Burn-lane search is not impl.
import Tile from '../../geometry/Tile.js';
import { notImpl } from '../../shim/_kernel.js';

export { lightFire } from './LightFire.js';

export const TINDERBOX = 'Tinderbox';
export const CANT_LIGHT = /can't light a fire here/i;

function stand(x, z, x0, x1, z0, z1) {
    return { bank: new Tile(x, z, 0), x0, x1, z0, z1 };
}

export const FIRE_SPOTS = {
    'Varrock East': stand(3253, 3420, 3235, 3275, 3418, 3432),
    'Varrock West': stand(3185, 3440, 3170, 3205, 3426, 3444),
    Draynor: stand(3093, 3243, 3078, 3098, 3240, 3252),
    Seers: stand(2725, 3491, 2710, 2735, 3482, 3494),
};

export const FIRE_SPOT_OPTIONS = Object.keys(FIRE_SPOTS);

export const LOG_LEVELS = {
    Logs: 1,
    'Oak logs': 15,
    'Willow logs': 30,
    'Maple logs': 45,
    'Yew logs': 60,
    'Magic logs': 75,
};

export class NoLightTiles extends Error {
    constructor() {
        super('not impl: Firemaking.NoLightTiles');
        this.name = 'NoLightTiles';
    }
}

export function findBurnLane() {
    throw notImpl('Firemaking.findBurnLane');
}

export function burnLaneWant() {
    throw notImpl('Firemaking.burnLaneWant');
}

export function fireReactionTicks() {
    throw notImpl('Firemaking.fireReactionTicks');
}

export function inFirePlot() {
    throw notImpl('Firemaking.inFirePlot');
}

export function isBurnWest() {
    throw notImpl('Firemaking.isBurnWest');
}

export function runInDir() {
    throw notImpl('Firemaking.runInDir');
}

export function tileKey() {
    throw notImpl('Firemaking.tileKey');
}
