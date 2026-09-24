// Name-map driveActivePartnerTrade onto one `partner-trade` machine await:
// one frozen iteration per call. Rust calls the caller's callbacks in the
// frozen drivePartnerTrade.ts order and reads the trade screens from the
// scene.
import { notImpl, runMachine } from '../../shim/_kernel.js';

const CALLBACKS = [
    'productNamesToOffer',
    'inventoryMetric',
    'myOfferReady',
    'theirProductMatch',
    'setStatus',
    'log',
    'onMissingPartner',
    'receiverCanAccept',
    'onComplete',
    'onDecline',
    'baseline',
];

// Call-time wrappers with receiver `opts`. Metric values cross as
// `String(Number(v))`, the coercion frozen arithmetic applies, so NaN
// survives; onComplete gets its delta back as a number.
function hooks(opts) {
    const h = {};
    for (const name of CALLBACKS) {
        if (opts[name] != null) h[name] = (...args) => opts[name](...args);
    }
    if (h.inventoryMetric) h.inventoryMetric = () => String(Number(opts.inventoryMetric()));
    if (h.baseline) {
        h.baseline = () => {
            const v = opts.baseline();
            return v == null ? null : String(Number(v));
        };
    }
    if (h.onComplete) h.onComplete = (d) => opts.onComplete(Number(d));
    return h;
}

export async function driveActivePartnerTrade(opts) {
    const out = await runMachine(
        'partner-trade',
        {
            role: opts.role ?? null,
            partners: opts.partners ?? null,
            verifyGiverPartner: !!opts.verifyGiverPartner,
            labels: opts.labels ?? null,
        },
        hooks(opts),
    );
    if (out.kind === 'refused') throw notImpl('driveActivePartnerTrade', out.reason);
}
