// Catalog Firemaking URL: SETTINGS keys + lightFire. Rust selects and ranks
// lanes inside the posted host AABB and walks the caller's lane callbacks;
// this shim marshals inputs and results.
import Tile from '../../geometry/Tile.js';
import { host, snap, notImpl } from '../../shim/_kernel.js';

export { lightFire } from './LightFire.js';

export const TINDERBOX = 'Tinderbox';
export const CANT_LIGHT = /can't light a fire here/i;
export const FIRE_START_TICKS = 14;
export const FIRE_LIGHT_TICKS = 150;
export const BURN_WEST = { dx: -1, dz: 0 };
export const BURN_DIRS = [BURN_WEST, { dx: 1, dz: 0 }, { dx: 0, dz: -1 }, { dx: 0, dz: 1 }];

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

function native(op, ...args) {
    return globalThis.__rs2b0t_firemaking(op, ...args);
}

/** Frozen box of `half` tiles (default 8, at least 2) around `origin`. */
export function localFirePlot(origin, half) {
    return native('localFirePlot', Tile, origin, half);
}

export const LOG_LEVELS = Object.fromEntries((host().content && host().content.log_levels) || []);

export function tileKey(t) {
    return `${t.x},${t.z}`;
}

/** Tiles that answered CANT_LIGHT this session; the set itself is held in Rust. */
export class NoLightTiles {
    #slot = native('noLightNew');

    add(tile) {
        native('noLightAdd', this.#slot, tile);
    }

    has(tile) {
        return native('noLightHas', this.#slot, tile);
    }

    get size() {
        return native('noLightSize', this.#slot);
    }

    merge(occupied) {
        return native('noLightMerge', this.#slot, occupied);
    }

    clear() {
        native('noLightClear', this.#slot);
    }
}

function callFire(payload, feature = 'Firemaking.findBurnLane') {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_fire
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl(feature);
    }
    return fn(payload);
}

export function inFirePlot(t, plot) {
    return !!callFire({ op: 'in-fire-plot', tile: t, plot });
}

export function burnLaneWant(logCount) {
    return Number(callFire({ op: 'burn-lane-want', logCount }));
}

export function isBurnWest(dir) {
    return !!callFire({ op: 'is-burn-west', dir });
}

export function fireReactionTicks() {
    return Number(callFire({ op: 'fire-reaction-ticks' }));
}

/** Frozen lane walk; the caller's `walkable` / `canStep` run once per visited tile. */
export function runInDir(from, plot, dir, occupied, walkable, canStep, cap) {
    return native('runInDir', from, plot, dir, occupied, walkable, canStep, cap);
}

export function findBurnLane(plot, here, occupied, want = 1, _walkable, _canStep, directions = BURN_DIRS) {
    if (!plot || typeof plot.x0 !== 'number' || typeof plot.x1 !== 'number') {
        throw notImpl('Firemaking.findBurnLane');
    }
    const refused = occupied instanceof Set
        ? [...occupied]
        : Array.isArray(occupied)
          ? occupied.slice()
          : [];
    const step = callFire({
        op: 'next-tile',
        plot: {
            x0: plot.x0,
            x1: plot.x1,
            z0: plot.z0,
            z1: plot.z1,
            bank: plot.bank || { x: 0, z: 0, level: 0 },
        },
        here: here || snap().here || null,
        refused,
        want: burnLaneWant(want),
        directions: directions || BURN_DIRS,
    });
    if (!step || step.kind === 'none') return null;
    if (step.kind === 'notImpl') throw notImpl('Firemaking.findBurnLane', step.reason);
    if (step.kind !== 'tile') return null;
    return {
        start: new Tile(step.x, step.z, step.level ?? 0),
        run: step.run,
        dir: step.dir || BURN_WEST,
    };
}
