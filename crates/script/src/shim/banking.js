// Rust owns opening precedence, reachable selection and the bank-trip
// continuation. JavaScript only maps the catalog names and await plumbing.
import { runMachine } from '../../shim/_kernel.js';

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
            const out = await runMachine('banking_open', {
                stand: opts.stand ?? null,
                destination: opts.destination ?? null,
                boothName: opts.boothName,
                boothOp: opts.boothOp,
                preferNearby: opts.preferNearby,
                nearbyRadius: opts.nearbyRadius,
                obstacles: opts.obstacles ?? [],
            }, { log: opts.log });
            return out.kind === 'done' && out.value === true;
        },

        // Rust runs the frozen open → deposit → afterDeposit → return trip.
        async bankNearest(opts = {}) {
            opts = opts || {};
            const deposit = opts.deposit;
            const out = await runMachine(
                'bank_nearest',
                {
                    destination: opts.destination ?? null,
                    booth_name: opts.boothName,
                    booth_op: opts.boothOp,
                    return_to: opts.returnTo ?? null,
                    deposit_all: !!deposit && typeof deposit !== 'function',
                    common_junk: !!(opts.commonJunk ?? true),
                },
                {
                    deposit: typeof deposit === 'function' ? deposit : undefined,
                    afterDeposit: opts.afterDeposit,
                    log: opts.log,
                },
            );
            return out.kind === 'done' && out.value === true;
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
