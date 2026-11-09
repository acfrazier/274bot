// The native nearest-world bank machine selects the bank access kind, walks,
// opens it and waits for the fresh bank page. This adapter only preserves the
// catalog callback signature; it starts the private typed request directly.
import { runMachine } from '../../../shim/_kernel.js';

export async function openClueBank(_log) {
    const out = await runMachine('bank_open', { mode: 'open-nearest-world', stand: null });
    return out.kind === 'done' && out.value === true;
}
