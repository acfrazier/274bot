// Catalog Firemaking URL: SETTINGS keys + lightFire. Burn-lane search is not impl.
import Tile from '../../geometry/Tile.js';
import { host, notImpl } from '../../shim/_kernel.js';

export { lightFire } from './LightFire.js';

export const TINDERBOX = 'Tinderbox';
export const CANT_LIGHT = /can't light a fire here/i;

function fireSpots() {
    const out = {};
    for (const p of (host().content && host().content.fire_plots) || []) {
        out[p.name] = {
            bank: new Tile(p.bank.x, p.bank.z, p.bank.level ?? 0),
            x0: p.x0,
            x1: p.x1,
            z0: p.z0,
            z1: p.z1,
        };
    }
    return out;
}

export const FIRE_SPOTS = fireSpots();

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
