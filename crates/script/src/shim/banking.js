// Our Banking module: `open()` is a thin name onto the host's nearest
// Use-quickly loc interact (same plane; none fails closed). Deposit helpers
// stay here. Missing members throw `not impl`. rs2b0t's Banking.ts / webwalk
// are never executed.
import { Bank } from './Bank.js';
import { Execution } from '../execution/Execution.js';

const notImpl = (name, reason) =>
    new Error(reason ? 'not impl: ' + name + ': ' + reason : 'not impl: ' + name);

// Rust owns the published list and predicate. These are thin mappings so
// catalog callers keep their names and optional observed object id.
export const COMMON_BANK_LOOT = Object.freeze(
    (globalThis.__rs2b0t_host?.content?.common_bank_loot || []).slice(),
);

export const RANDOM_EVENT_CASKET_ID =
    globalThis.__rs2b0t_host?.content?.random_event_casket_id ?? -1;

export function matchesCommonBankLoot(name, id = -1) {
    return globalThis.rustyscript.functions.__rs2b0t_matches_common_bank_loot(
        String(name ?? ''),
        Number.isSafeInteger(id) ? id : -1,
    );
}

export function depositMatcher(own, includeCommon) {
    if (typeof own !== 'function') throw notImpl('depositMatcher', 'requires a function');
    return (name, id = -1) => own(name) || (!!includeCommon && matchesCommonBankLoot(name, id));
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
        async open(opts = {}) {
            opts = opts || {};
            const supported = new Set(['stand', 'boothName', 'boothOp', 'log']);
            for (const name of Object.keys(opts)) {
                if (!supported.has(name) && opts[name] !== undefined) {
                    throw notImpl('Banking.open', name);
                }
            }
            return Bank.openBooth(opts.stand, opts.boothName, opts.boothOp, opts.log);
        },

        async bankNearest({ deposit = false, commonJunk = false } = {}) {
            if (!(await Bank.openNearestWorld())) {
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
