import Tile from '../../geometry/Tile.js';
import { Game } from '../game/Game.js';
import { Traversal } from '../walking/Traversal.js';

export const HOME_ARRIVE_RADIUS = 8;

export function shouldWalkHomeToGatherAnchor(distToAnchor, arriveRadius = HOME_ARRIVE_RADIUS) {
    if (distToAnchor == null || !Number.isFinite(distToAnchor)) {
        return false;
    }
    const r = Math.max(0, Math.floor(Number.isFinite(arriveRadius) ? arriveRadius : HOME_ARRIVE_RADIUS));
    return distToAnchor > r;
}

export function shouldSoftHomeFromGatherMiss(distToAnchor, leash = 64) {
    if (distToAnchor == null || !Number.isFinite(distToAnchor)) {
        return false;
    }
    const L = Math.max(2, Math.floor(Number.isFinite(leash) ? leash : 64));
    const threshold = Math.max(HOME_ARRIVE_RADIUS + 12, Math.min(L, 28));
    return distToAnchor > threshold;
}

function distanceToAnchor(host, here = Game.tile()) {
    if (!here) {
        return null;
    }
    return host.getAnchor().distanceTo(here);
}

export function beyondLeash(host, here = Game.tile(), slack = 0) {
    const d = distanceToAnchor(host, here);
    return d !== null && d > host.leashRadius() + slack;
}

export function tileWithinLeash(host, tile, slack = 0) {
    return host.getAnchor().distanceTo(tile) <= host.leashRadius() + slack;
}

export function resolveRunAnchor(here, locationSpot) {
    if (locationSpot) {
        return locationSpot;
    }
    return new Tile(here.x, here.z, here.level);
}

export function createReturnToAnchorTask(host, opts = {}) {
    const slack = opts.slack ?? 6;
    const arriveRadius = opts.arriveRadius ?? 8;
    const status = opts.status ?? 'returning to anchor';
    return {
        validate() {
            if (opts.suppress?.()) {
                return false;
            }
            const d = distanceToAnchor(host);
            return d !== null && d > arriveRadius + slack;
        },
        execute() {
            host.setStatus?.(status);
            const anchor = host.getAnchor();
            Traversal.walkResilient(anchor.x, anchor.z, anchor.level);
        },
    };
}
