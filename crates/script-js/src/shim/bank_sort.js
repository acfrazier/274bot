import { runMachine } from '../../shim/_kernel.js';

export const ARRANGE_SWAP_COM = 8130;
export const ARRANGE_INSERT_COM = 8131;
export const BANK_INSERT_VARP = 304;

// Rust plans, sets the arrange mode, drags one row per observed tick,
// verifies the order and restores the original mode. `unmatched` arrives in
// bounded chunks (the seat channel's array limit) and is joined here.
export async function sortBank(opts = {}) {
    const overrides = opts.categoryOverrides;
    const out = await runMachine(
        'bank_task',
        { kind: 'sort', overrides: overrides == null ? null : Array.from(overrides) },
        { log: typeof opts.log === 'function' ? opts.log : undefined },
    );
    if (out.kind === 'done') {
        out.value.unmatched = out.value.unmatched.flat();
        return out.value;
    }
    return { sorted: false, moves: 0, mode: null, unmatched: [], reason: String(out.reason ?? out.kind) };
}
