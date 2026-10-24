// Name-map: one `light-fire` machine await. Rust sends the tinderbox use-on
// and owns the start/light tick waits and the XP / cannot-light verdict.
import { notImpl, runMachine } from '../../shim/_kernel.js';

export async function lightFire(logName) {
    const out = await runMachine('light-fire', { logName: String(logName ?? '') });
    if (out.kind === 'refused') throw notImpl('lightFire', out.reason);
    return out.kind === 'done' ? out.value : 'stalled';
}
