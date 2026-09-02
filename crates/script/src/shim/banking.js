// Our Banking module: open a bank from the packed stand table and the
// scene booth list the host posts each tick, then deposit/withdraw through
// Bank. `open()` prefers a scene booth within `BOOTH_RANGE` (Chebyshev,
// level-equal): the open-booth request is queued and the caller waits for
// `bank_loaded`. Otherwise it queues a walk to the nearest packed stand —
// the host routes it through the slot's traveller with default FindOptions
// (no teleports, no wilderness; quest/wildy gates fail closed) — parks
// until the player stands next to the stand, then queues the stand's
// access op (booth Use-quickly or teller NPC op). Missing members throw
// `not v1`. rs2b0t's Banking.ts / webwalk are never executed.
import { Bank } from './Bank.js';
import { Execution } from '../execution/Execution.js';

const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const snap = () => host().snapshot || {};
const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};

// Chebyshev range that counts as "at" a booth/stand: adjacent is enough
// to click the booth (the client paths onto its tile on its own).
const BOOTH_RANGE = 1;
// How long to wait for the bank list after an open op is dispatched.
const LOAD_TIMEOUT_MS = 5000;
// How long a walk to a packed stand may take (the park is frozen by
// guardian hold like every wait; a refused route times out fail-closed).
const WALK_TIMEOUT_MS = 60000;

// Level-aware Chebyshev: a stand on another plane is never in range.
const dist = (a, b) => {
    if (!a || !b || a.level !== b.level) return Infinity;
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
};

const nearest = (here, rows) => {
    let best = null;
    let bestD = Infinity;
    for (const row of rows || []) {
        const d = dist(here, row);
        if (d < bestD) {
            best = row;
            bestD = d;
        }
    }
    return best;
};

const waitLoaded = () =>
    Execution.delayUntil(() => snap().bank_loaded === true, LOAD_TIMEOUT_MS);

const waitArrival = (stand) =>
    Execution.delayUntil(() => dist(snap().here, stand) <= BOOTH_RANGE, WALK_TIMEOUT_MS);

// The rs2b0t bankRules junk list, kept as data. Names match as
// substrings; one that is not a 274 obj never matches a posted row (the
// host resolves names through ObjNames), so it drops itself.
export const COMMON_BANK_LOOT = [
    'uncut',
    'sapphire',
    'emerald',
    'ruby',
    'diamond',
    'opal',
    'jade',
    'topaz',
    'strange fruit',
    'beer',
    'kebab',
];

export function matchesCommonBankLoot(name) {
    const n = String(name).toLowerCase();
    return COMMON_BANK_LOOT.some((p) => n.includes(p));
}

// The rs2b0t depositMatcher shape, minus the obj-id arm (posted rows
// carry names only): `own(name) || (includeCommon && matchesJunk(name))`.
export function depositMatcher(own, includeCommon) {
    return (name) => own(name) || (includeCommon && matchesCommonBankLoot(name));
}

export function depositAllExcept(keep) {
    const set = new Set(Array.from(keep || []).map((s) => String(s).toLowerCase()));
    return (name) => name.length > 0 && !set.has(name.toLowerCase());
}

const BANK_STRATEGY_OPTIONS = ['Off', 'Loot count', 'Time', 'Either'];

export const PERIODIC_BANK_SETTINGS = {
    bankStrategy: {
        type: 'string',
        default: 'Off',
        options: BANK_STRATEGY_OPTIONS,
        label: 'Periodic bank',
        help: 'save accumulated loot so a death does not lose it all',
    },
    bankEveryItems: { type: 'number', default: 15, min: 1, max: 27, label: 'Bank at N loot items' },
    bankEveryMinutes: { type: 'number', default: 10, min: 1, max: 120, label: 'Bank every N minutes' },
    bankCommonJunk: {
        type: 'boolean',
        default: true,
        label: 'Also bank gems/fruit/beer/kebabs/caskets',
    },
};

export function parseBankStrategy(label) {
    const n = String(label || '')
        .trim()
        .toLowerCase();
    if (n === 'off') return 'off';
    if (n === 'loot count') return 'loot';
    if (n === 'time') return 'time';
    if (n === 'either') return 'either';
    return 'off';
}

export const Banking = new Proxy(
    {
        async open() {
            for (;;) {
                const s = snap();
                if (s.bank_open === true) {
                    // Already open: still wait for the item list — an open
                    // bank whose list is still filling reads empty, which
                    // is not proof of an empty bank.
                    return waitLoaded();
                }
                const here = s.here;
                if (!here) {
                    return false;
                }
                const booth = nearest(here, s.booths);
                if (booth && dist(here, booth) <= BOOTH_RANGE) {
                    queue({ op: 'open-booth', x: booth.x, z: booth.z, level: booth.level });
                    return waitLoaded();
                }
                const stand = nearest(here, s.banks);
                if (!stand) {
                    // No packed stand and no booth in range: fail closed.
                    return false;
                }
                if (dist(here, stand) <= BOOTH_RANGE) {
                    // Next to the stand: use its access (booth Use-quickly
                    // or teller NPC op; `choose` is carried for the dialog
                    // the NPC op starts, answered by a later task).
                    queue({
                        op: 'open-stand',
                        x: stand.x,
                        z: stand.z,
                        level: stand.level,
                        kind: stand.kind,
                        name: stand.name,
                        stand_op: stand.op,
                        choose: stand.choose,
                    });
                    return waitLoaded();
                }
                queue({ op: 'walk', x: stand.x, z: stand.z, level: stand.level });
                if (!(await waitArrival(stand))) {
                    return false;
                }
            }
        },

        async bankNearest({ deposit = false, commonJunk = false } = {}) {
            if (!(await Banking.open())) {
                return false;
            }
            if (deposit) {
                if (typeof deposit === 'function') {
                    await Bank.depositAllMatching(depositMatcher(deposit, commonJunk));
                } else {
                    await Bank.depositInventory();
                }
            }
            await Execution.delayTicks(1);
            return true;
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Banking.' + String(prop));
        },
    },
);
