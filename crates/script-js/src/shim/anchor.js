import Tile from '../../geometry/Tile.js';
import { Game } from '../game/Game.js';
import { Sustain } from '../sustain/Sustain.js';
import { host, notImpl, runMachine } from '../../shim/_kernel.js';

export const HOME_ARRIVE_RADIUS = 8;

export function shouldWalkHomeToGatherAnchor(_distToAnchor, _arriveRadius) {
    throw notImpl('shouldWalkHomeToGatherAnchor');
}

export function shouldSoftHomeFromGatherMiss(_distToAnchor, _leash) {
    throw notImpl('shouldSoftHomeFromGatherMiss');
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
    throw notImpl('createReturnToAnchorTask');
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
    const status = opts.status ?? 'returning to anchor';
    return {
        // Frozen Anchor.ts:95-100: the bot's leash plus slack.
        validate() {
            if (opts.suppress?.()) {
                return false;
            }
            try {
                return beyondLeash(bot, Game.tile(), slack);
            } catch (_) {
                return false;
            }
        },
        // Frozen Anchor.ts:101-127: Rust owns the legs (`return-to-anchor`).
        async execute() {
            bot.setStatus?.(status);
            const tile = campTile(bot);
            const out = await runMachine(
                'return-to-anchor',
                {
                    anchor: { x: tile.x, z: tile.z, level: tile.level ?? 0 },
                    ...(typeof opts.arriveRadius === 'number'
                        ? { arriveRadius: Math.floor(opts.arriveRadius) }
                        : {}),
                    ...(typeof opts.timeoutMs === 'number'
                        ? { timeoutMs: Math.floor(opts.timeoutMs) }
                        : {}),
                    obstacles: Array.isArray(opts.obstacles) ? opts.obstacles : [],
                    ...(typeof opts.longRangeTiles === 'number'
                        ? { longRangeTiles: Math.floor(opts.longRangeTiles) }
                        : {}),
                },
                { log: (m) => bot.log?.(m), sustain: () => Sustain.run() },
            );
            if (out.kind === 'refused') throw notImpl('createReturnToAnchorTask', out.reason);
        },
    };
}
