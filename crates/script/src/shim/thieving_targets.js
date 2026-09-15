import Tile from '../../geometry/Tile.js';
import { host, notImpl } from '../../shim/_kernel.js';

export const PICKPOCKET_TARGET_NAMES = [
    'Man',
    'Woman',
    'Farmer',
    'Warrior woman',
    'Al-Kharid warrior',
    'Rogue',
    'Guard',
    'Knight of Ardougne',
    'Watchman',
    'Paladin',
    'Hero',
];
export const ARDOUGNE_PICKPOCKET_TARGETS = ['Guard', 'Knight of Ardougne', 'Paladin', 'Hero'];

function pickpocketSpots() {
    return (host().content && host().content.pickpocket_spots) || [];
}

function spotRow(target) {
    const spots = pickpocketSpots();
    if (!spots.length) {
        return null;
    }
    const want = String(target || '').trim().toLowerCase();
    const hit = spots.find((p) => String(p.name).toLowerCase() === want);
    if (hit) {
        return hit;
    }
    return spots.find((p) => String(p.name).toLowerCase() === 'guard') || spots[0];
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

/** `for...of` closes an iterator it leaves early: a returning body propagates a
 *  `return()` failure, a throwing body keeps its own error. */
function closeIterator(iterator) {
    const ret = iterator.return;
    if (ret === undefined || ret === null) return;
    if (typeof ret !== 'function') throw new TypeError('iterator.return is not callable');
    const inner = ret.call(iterator);
    if (inner === null || (typeof inner !== 'object' && typeof inner !== 'function')) {
        throw new TypeError('iterator.return must return an object');
    }
}

/**
 * First candidate the caller accepts wins. The native step owns the traversal,
 * the first hit, the short circuit and the exhaustion fallback; this shim keeps
 * the caller's iterator and runs only the element read, the `reachable(c)` call
 * and the truthiness conversion each step asks for.
 */
export function chooseTarget(candidatesNearestFirst, reachable) {
    const iterator = candidatesNearestFirst[Symbol.iterator]();
    let value;
    let done = null;
    let probed = null;
    for (;;) {
        const payload = { op: 'choose' };
        if (done !== null) payload.done = done;
        if (probed !== null) payload.probed = probed;
        const step = callStep(payload);
        if (step.kind === 'hit') {
            closeIterator(iterator);
            return { target: value, blocked: null };
        }
        if (step.kind === 'exhausted') {
            return { target: null, blocked: candidatesNearestFirst[0] ?? null };
        }
        if (step.kind === 'probe') {
            let answer;
            try {
                answer = reachable(value);
            } catch (e) {
                try {
                    closeIterator(iterator);
                } catch (_) {
                    // the callback's own error wins, exactly as `for...of` closes
                }
                throw e;
            }
            done = null;
            probed = !!answer;
            continue;
        }
        if (step.kind !== 'next') throw notImpl('Thieving.chooseTarget', step.reason);
        const next = iterator.next();
        done = !!next.done;
        probed = null;
        if (!done) value = next.value;
    }
}
