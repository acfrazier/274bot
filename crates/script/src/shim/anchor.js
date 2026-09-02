import Tile from '../../geometry/Tile.js';
import { Game } from '../game/Game.js';
import { Traversal } from '../walking/Traversal.js';
import { host, notV1 } from '../../shim/_kernel.js';

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

function campTile(bot) {
    if (bot && typeof bot.getAnchor === 'function') {
        const a = bot.getAnchor();
        if (a && typeof a.x === 'number' && typeof a.z === 'number') {
            return Tile.from(a);
        }
    }
    const bag = host().settingsBag || {};
    const t = bag.camp || bag.anchor;
    if (t && typeof t.x === 'number' && typeof t.z === 'number') {
        return Tile.from(t);
    }
    throw notV1('createReturnToAnchorTask');
}

function distanceToAnchor(bot, here = Game.tile()) {
    if (!here) {
        return null;
    }
    return campTile(bot).distanceTo(here);
}

export function beyondLeash(bot, here = Game.tile(), slack = 0) {
    const d = distanceToAnchor(bot, here);
    return d !== null && d > bot.leashRadius() + slack;
}

export function tileWithinLeash(bot, tile, slack = 0) {
    return campTile(bot).distanceTo(tile) <= bot.leashRadius() + slack;
}

export function resolveRunAnchor(here, locationSpot) {
    if (locationSpot) {
        return locationSpot;
    }
    return new Tile(here.x, here.z, here.level);
}

export function createReturnToAnchorTask(bot, opts = {}) {
    const slack = opts.slack ?? 6;
    const arriveRadius = opts.arriveRadius ?? 8;
    const status = opts.status ?? 'returning to anchor';
    return {
        validate() {
            if (opts.suppress?.()) {
                return false;
            }
            try {
                const d = distanceToAnchor(bot);
                return d !== null && d > arriveRadius + slack;
            } catch (_) {
                return false;
            }
        },
        async execute() {
            bot.setStatus?.(status);
            const tile = campTile(bot);
            return Traversal.walkResilient(tile, {
                radius: arriveRadius,
                timeoutMs: opts.timeoutMs ?? 60_000,
            });
        },
    };
}
