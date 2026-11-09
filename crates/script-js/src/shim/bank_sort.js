import { runMachine } from '../../shim/_kernel.js';

export const ARRANGE_SWAP_COM = 8130;
export const ARRANGE_INSERT_COM = 8131;
export const BANK_INSERT_VARP = 304;

// Rust plans, sets the arrange mode, drags one row per observed tick,
// verifies the order and restores the original mode, and settles the whole
// `BankSortResult` (a refusal or abort is a failed result with its reason).
export async function sortBank(opts = {}) {
    const overrides = opts.categoryOverrides;
    const out = await runMachine(
        'bank_task',
        { kind: 'sort', overrides: overrides == null ? null : Array.from(overrides) },
        { log: typeof opts.log === 'function' ? opts.log : undefined },
    );
    return out.value;
}
