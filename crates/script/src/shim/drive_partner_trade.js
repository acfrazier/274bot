// Name-map driveActivePartnerTrade onto one Rust-owned `partner-trade`
// machine. Rust calls the caller's callbacks in the frozen order, reads
// their offer from the scene, sequences offer/confirm and never treats a
// close as a transfer.
import { notImpl, runMachine } from '../../shim/_kernel.js';

// The caller callbacks the machine may call; only functions are handed
// over, each read from `opts` at call time.
const HOOKS = [
    'productNamesToOffer',
    'inventoryMetric',
    'myOfferReady',
    'theirProductMatch',
    'setStatus',
    'onMissingPartner',
    'receiverCanAccept',
    'onComplete',
    'onDecline',
];

function callables(opts) {
    const hooks = {};
    for (const name of HOOKS) {
        if (typeof opts[name] === 'function') hooks[name] = (...args) => opts[name](...args);
    }
    return hooks;
}

export async function driveActivePartnerTrade(opts) {
    const options = opts || {};
    const names = options.productNamesToOffer;
    const out = await runMachine(
        'partner-trade',
        {
            role: options.role ?? null,
            partners: Array.isArray(options.partners)
                ? options.partners.map((n) => String(n ?? ''))
                : [],
            verifyGiverPartner: options.verifyGiverPartner === true,
            names: typeof names === 'function' ? null : (names ?? null),
            labels: options.labels || {},
        },
        callables(options),
    );
    if (out.kind === 'refused') throw notImpl('driveActivePartnerTrade', out.reason);
}
