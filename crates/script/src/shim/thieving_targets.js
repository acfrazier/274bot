import Tile from '../../geometry/Tile.js';
import { host, notImpl } from '../../shim/_kernel.js';
export {
    PICKPOCKET_TARGET_NAMES,
    ARDOUGNE_PICKPOCKET_TARGETS,
} from '../../data/pickpocketTargets.js';

function spotRow(target) {
    const fn = globalThis.__rs2b0t_selected_facts;
    if (typeof fn !== 'function') return null;
    return fn('pickpocket-spot', String(target || '').trim().toLowerCase());
}

export function targetSpot(target) {
    const bag = host().settingsBag || {};
    const spots = bag.campTiles || bag.spots;
    const row = spots && spots[target];
    if (row && row.anchor && typeof row.anchor.x === 'number') {
        return { anchor: Tile.from(row.anchor), leash: row.leash ?? 19 };
    }
    const posted = spotRow(target);
    if (!posted || typeof posted.x !== 'number' || typeof posted.z !== 'number') {
        throw notImpl('targetSpot');
    }
    return {
        anchor: new Tile(posted.x, posted.z, posted.level ?? 0),
        leash: posted.leash ?? 19,
    };
}

export function requiredThieving(target) {
    const bag = host().settingsBag || {};
    const levels = bag.pickpocketLevels || bag.thievingLevels;
    if (levels && typeof levels[target] === 'number') {
        return levels[target];
    }
    const posted = spotRow(target);
    if (!posted || typeof posted.required_thieving !== 'number') {
        throw notImpl('requiredThieving');
    }
    return posted.required_thieving;
}

export const HOSTILE_NAMES = [];

function callOrValue(c, key) {
    if (c == null) return undefined;
    const value = c[key];
    return typeof value === 'function' ? value.call(c) : value;
}

export function isHostileAttacker(c, maxDistance) {
    const distance = callOrValue(c, 'distance');
    const actions = callOrValue(c, 'actions');
    return globalThis.rustyscript.functions.__rs2b0t_is_hostile_attacker({
        name: c == null ? null : c.name,
        inCombat: !!callOrValue(c, 'inCombat'),
        targetsAnotherPlayer: !!callOrValue(c, 'targetsAnotherPlayer'),
        distance: typeof distance === 'number' && Number.isFinite(distance) ? distance : null,
        actions: Array.isArray(actions) ? actions : [],
        maxDistance,
    });
}

/** First candidate the caller accepts wins; one native call walks the caller's iterable. */
export function chooseTarget(candidatesNearestFirst, reachable) {
    return globalThis.__rs2b0t_choose_target(candidatesNearestFirst, reachable);
}
