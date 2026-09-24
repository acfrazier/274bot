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

function callStep(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_target_step(payload);
}

/**
 * First candidate the caller accepts wins. The native step owns the decisions:
 * it asks for the element's `reachable` answer, names the first truthy answer
 * as the hit, stops the scan there, and only the caller's exhaustion reaches
 * the blocked fallback.
 *
 * The walk is the engine's own `for...of` over the caller's iterable and
 * nothing else: the iterator is acquired once, its `next` is read once and
 * called per step, a primitive `next()` result, a missing or non-callable
 * `Symbol.iterator`, a non-callable `return` and the close on the returning
 * hit keep their exact engine order. Only the element read, the `reachable(c)`
 * call and the truthiness conversion happen in JS, and no candidate, callback
 * answer or collected reachability crosses the bridge.
 */
export function chooseTarget(candidatesNearestFirst, reachable) {
    const start = callStep({ op: 'choose' });
    if (start.kind !== 'next') throw notImpl('Thieving.chooseTarget', start.reason);
    for (const c of candidatesNearestFirst) {
        const held = callStep({ op: 'choose', done: false });
        if (held.kind !== 'probe') throw notImpl('Thieving.chooseTarget', held.reason);
        const verdict = callStep({ op: 'choose', probed: !!reachable(c) });
        if (verdict.kind === 'hit') return { target: c, blocked: null };
        if (verdict.kind !== 'next') throw notImpl('Thieving.chooseTarget', verdict.reason);
    }
    const end = callStep({ op: 'choose', done: true });
    if (end.kind !== 'exhausted') throw notImpl('Thieving.chooseTarget', end.reason);
    return { target: null, blocked: candidatesNearestFirst[0] ?? null };
}
