import { Inventory } from '../inventory/Inventory.js';
import { Locs } from '../locs/Locs.js';
import { Game } from '../game/Game.js';
import { Traversal } from '../walking/Traversal.js';
import { Execution } from '../execution/Execution.js';
import { host } from '../../shim/_kernel.js';
import { tileFromPosted } from '../../geometry/Tile.js';
import { CAKE_ITEMS } from './cakeStallData.js';

const TARGET_RADIUS = 3;
const RESOLVE_MS = 2400;

function postedStall() {
    const row = host().content && host().content.baker_stall;
    if (!row || typeof row !== 'object') {
        return null;
    }
    const stall = tileFromPosted(row.stall);
    const stand = tileFromPosted(row.stand);
    const name = typeof row.name === 'string' && row.name.trim() ? row.name : null;
    const op = typeof row.op === 'string' && row.op.trim() ? row.op : null;
    const locId = Number.isInteger(row.loc_id) ? row.loc_id : null;
    if (!stall || !stand || !name || !op || locId == null) {
        return null;
    }
    return {
        locId,
        name,
        op,
        stall,
        stand,
        standAlt: tileFromPosted(row.stand_alt),
    };
}

function sameTile(a, b) {
    return !!(
        a &&
        b &&
        a.x === b.x &&
        a.z === b.z &&
        (a.level ?? 0) === (b.level ?? 0)
    );
}

function called(fn) {
    return typeof fn === 'function' && !!fn();
}

function lockoutTick(opts) {
    const value = opts && opts.lockedOutUntil;
    if (typeof value === 'function') {
        const n = value();
        return typeof n === 'number' && Number.isFinite(n) ? n : 0;
    }
    if (typeof value === 'number' && Number.isFinite(value)) {
        return value;
    }
    return 0;
}

function atGoal(opts) {
    if (Inventory.isFull()) {
        return true;
    }
    const n = opts && opts.fillTo;
    return typeof n === 'number' && Number.isFinite(n) && carriedCakes() >= n;
}

function matchingStall(facts) {
    return Locs.query()
        .name(facts.name)
        .action(facts.op)
        .where(
            (loc) =>
                Number.isInteger(loc.id) &&
                loc.id === facts.locId &&
                loc.tile().distanceTo(facts.stall) <= TARGET_RADIUS,
        )
        .nearest();
}

export function carriedCakes() {
    return CAKE_ITEMS.reduce((n, name) => n + Inventory.count(name), 0);
}

export function needsCakeRestock(target) {
    const want = typeof target === 'number' && Number.isFinite(target) ? target : 1;
    return !Inventory.isFull() && carriedCakes() < want;
}

export async function stealCakes(opts = {}) {
    if (called(opts.abort) || called(opts.shouldEat)) {
        return 'aborted';
    }
    if (Game.inCombat()) {
        return 'combat';
    }
    if (atGoal(opts)) {
        return 'stocked';
    }
    if (Game.tick() < lockoutTick(opts)) {
        return 'no-progress';
    }

    let facts = postedStall();
    if (!facts) {
        return 'no-progress';
    }

    const here = Game.tile();
    const onStand = sameTile(here, facts.stand) || sameTile(here, facts.standAlt);
    if (!onStand) {
        const arrived = await Traversal.walkTo(facts.stand);
        if (called(opts.abort) || called(opts.shouldEat)) {
            return 'aborted';
        }
        if (Game.inCombat()) {
            return 'combat';
        }
        if (atGoal(opts) || Game.tick() < lockoutTick(opts) || !arrived) {
            return atGoal(opts) ? 'stocked' : 'no-progress';
        }
        facts = postedStall();
        if (!facts) {
            return 'no-progress';
        }
    }

    const loc = matchingStall(facts);
    if (!loc) {
        return 'no-progress';
    }

    const before = carriedCakes();
    if (!loc.interact(facts.op)) {
        return 'no-progress';
    }

    await Execution.delayUntil(
        () =>
            called(opts.abort) ||
            called(opts.shouldEat) ||
            Game.inCombat() ||
            carriedCakes() > before,
        RESOLVE_MS,
    );

    if (called(opts.abort) || called(opts.shouldEat)) {
        return 'aborted';
    }
    const gained = carriedCakes() > before;
    if (gained && typeof opts.onSteal === 'function') {
        opts.onSteal();
    }
    if (Game.inCombat()) {
        return 'combat';
    }
    if (atGoal(opts)) {
        return 'stocked';
    }
    return 'no-progress';
}
