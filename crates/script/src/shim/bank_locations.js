import Tile from '../../geometry/Tile.js';
import { Game } from '../game/Game.js';
import { snap } from '../../shim/_kernel.js';

function useQuickly(actions) {
    return (actions || []).some((a) => a && String(a).toLowerCase() === 'use-quickly');
}

function cheb(a, b) {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

/** Packed/scene booth on the same plane (Use-quickly). No booth → null. */
export function nearestBank(_hint) {
    const here = Game.tile();
    if (!here) {
        return null;
    }
    const plane = here.level ?? 0;
    let best = null;
    let bestD = Infinity;
    for (const loc of snap().locs || []) {
        if (!loc || (loc.level ?? 0) !== plane || !useQuickly(loc.actions)) {
            continue;
        }
        const d = cheb(here, loc);
        if (d < bestD) {
            bestD = d;
            best = {
                tile: new Tile(loc.x, loc.z, loc.level ?? 0),
                name: loc.name || 'Bank booth',
                op: 'Use-quickly',
            };
        }
    }
    if (best) {
        return best;
    }
    for (const booth of snap().booths || []) {
        if (!booth || (booth.level ?? 0) !== plane) {
            continue;
        }
        const d = cheb(here, booth);
        if (d < bestD) {
            bestD = d;
            best = {
                tile: new Tile(booth.x, booth.z, booth.level ?? 0),
                name: 'Bank booth',
                op: 'Use-quickly',
            };
        }
    }
    return best;
}
