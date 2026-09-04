// Our Banking module: `open()` is a thin name onto the host's nearest
// Use-quickly loc interact (same plane; none fails closed). Deposit helpers
// stay here. Missing members throw `not impl`. rs2b0t's Banking.ts / webwalk
// are never executed.
import { Bank } from './Bank.js';
import { Execution } from '../execution/Execution.js';

const notImpl = (name) => new Error('not impl: ' + name);

// The rs2b0t bankRules junk list, kept as data. Names match as
// substrings; one that is not a 274 obj never matches a posted row (the
// host resolves names through ObjNames), so it drops itself.
export const COMMON_BANK_LOOT = [];

/** Never matches a posted obj id. Catalog compares against this sentinel. */
export const RANDOM_EVENT_CASKET_ID = -1;

export function matchesCommonBankLoot(_name) {
    throw notImpl('matchesCommonBankLoot');
}

// The rs2b0t depositMatcher shape, minus the obj-id arm (posted rows
// carry names only). Common-junk matching is not impl — no policy table.
export function depositMatcher(own, includeCommon) {
    if (includeCommon) {
        throw notImpl('depositMatcher.includeCommon');
    }
    return (name) => own(name);
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
            return Bank.openNearest();
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
            throw notImpl('Banking.' + String(prop));
        },
    },
);
