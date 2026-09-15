// Catalog Firemaking URL: SETTINGS keys + lightFire. Rust selects and ranks
// lanes inside the posted host AABB; this shim marshals inputs and results.
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

/**
 * First posted fire plot whose bank level matches `origin` and whose inclusive
 * AABB contains it, else a half-tile box around `origin`.
 *
 * The scan is native (`__rs2b0t_fire` `local-plot`): it names the posted index
 * to read, gates the level before the containment and stops at the first hit.
 * The posted row, the two scalar facts (`lv === level` and the inclusive AABB)
 * and the fallback box stay here at the original expressions, so `typeof`,
 * `?? 0`, NaN/Infinity and a mutated posted row keep their JS coercion.
 */
export function localFirePlot(origin, half = 4) {
    const o = origin || snap().here || {};
    const x = o.x;
    const z = o.z;
    const level = o.level ?? 0;
    if (typeof x !== 'number' || typeof z !== 'number') {
        throw notImpl('localFirePlot');
    }
    const plots = (host().content && host().content.fire_plots) || [];
    let index = 0;
    let row = null;
    let rowLevel = 0;
    let report = 'start';
    let levelOk = false;
    let contained = false;
    for (;;) {
        const payload = { op: 'local-plot' };
        if (report === 'absent') {
            payload.index = index;
            payload.present = false;
        } else if (report === 'level') {
            payload.index = index;
            payload.present = true;
            payload.level_ok = levelOk;
        } else if (report === 'contains') {
            payload.index = index;
            payload.contained = contained;
        }
        const step = callFire(payload, 'Firemaking.localFirePlot');
        if (step.kind === 'fallback') {
            const h = Math.max(0, Math.floor(Number(half) || 4));
            return {
                bank: new Tile(x, z, level),
                x0: x - h,
                x1: x + h,
                z0: z - h,
                z1: z + h,
            };
        }
        if (step.kind === 'hit') {
            return {
                bank: new Tile(row.bank.x, row.bank.z, rowLevel),
                x0: row.x0,
                x1: row.x1,
                z0: row.z0,
                z1: row.z1,
            };
        }
        if (step.kind === 'contains') {
            contained = x >= row.x0 && x <= row.x1 && z >= row.z0 && z <= row.z1;
            report = 'contains';
            continue;
        }
        if (step.kind !== 'plot') throw notImpl('Firemaking.localFirePlot', step.reason);
        index = step.index;
        row = null;
        rowLevel = 0;
        if (index < plots.length) {
            row = plots[index];
            rowLevel = (row.bank && row.bank.level) ?? 0;
            levelOk = rowLevel === level;
            report = 'level';
        } else {
            report = 'absent';
        }
    }
}

export const LOG_LEVELS = {
    Logs: 1,
    'Oak logs': 15,
    'Willow logs': 30,
    'Maple logs': 45,
    'Yew logs': 60,
    'Magic logs': 75,
};

export function tileKey(t) {
    return `${t.x},${t.z}`;
}

export class NoLightTiles {
    constructor() {
        this.refused = new Set();
    }

    add(tile) {
        this.refused = new Set(callFire({
            op: 'no-light', action: 'add', keys: [...this.refused], key: tileKey(tile),
        }));
    }

    has(tile) {
        return !!callFire({
            op: 'no-light', action: 'has', keys: [...this.refused], key: tileKey(tile),
        });
    }

    get size() {
        return Number(callFire({ op: 'no-light', action: 'size', keys: [...this.refused] }));
    }

    merge(occupied) {
        return new Set(callFire({
            op: 'no-light', action: 'merge', keys: [...this.refused], occupied: [...occupied],
        }));
    }

    clear() {
        this.refused = new Set(callFire({ op: 'no-light', action: 'clear', keys: [...this.refused] }));
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

/** Native posted-reach run; callback args preserve the frozen signature. */
export function runInDir(from, plot, dir, occupied, walkable, _canStep, cap) {
    const step = callFire({
        op: 'run-in-dir', from, plot, dir,
        occupied: occupied && typeof occupied.has === 'function' ? [...occupied] : [],
        hasWalkable: typeof walkable === 'function', cap,
    });
    if (step.kind === 'callback') {
        return callFire({ op: 'run-in-dir-result', walkable: !!walkable(from), from, dir,
            plot, occupied: occupied && typeof occupied.has === 'function' ? [...occupied] : [], cap }).run;
    }
    return step.run || 0;
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
