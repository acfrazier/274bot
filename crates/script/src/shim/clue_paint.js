// Frozen cluePaint.ts: the live clue's own leg/step frame, or the caller's
// idle line when nothing is being solved. The clue machine owns the trail and
// no per-leg paint rows are published here, so a live clue fails closed
// instead of painting a stale leg — never a silent no-op.
import { notImpl } from '../../../shim/_kernel.js';

export function paintClueProgress(p, idle = 'no clue in progress') {
    if (globalThis.rustyscript.functions.__rs2b0t_machine_live('clue')) {
        throw notImpl('paintClueProgress', 'no clue leg rows');
    }
    p.text(String(idle));
}
