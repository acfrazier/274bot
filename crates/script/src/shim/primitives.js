import { notImpl } from '../../../../shim/_kernel.js';
import { Npcs, talkOp } from '../../../npcs/Npcs.js';
import { ChatDialog } from '../../../ui/dialogue/ChatDialog.js';
import { Traversal } from '../../../walking/Traversal.js';

export { talkOp };

export function pickPreferred(options, prefer) {
    const opts = options || [];
    for (const p of prefer || []) {
        const want = String(p).toLowerCase();
        const hit = opts.find((o) => String(o).toLowerCase().includes(want));
        if (hit) return hit;
    }
    return null;
}

export function pickByLine() {
    throw notImpl('primitives.pickByLine');
}

export function isUnderground(t) {
    return !!(t && t.z > 6400);
}

export function needsHop() {
    throw notImpl('primitives.needsHop');
}

export async function walkWithHops(dest) {
    if (!dest || typeof dest.x !== 'number') return false;
    Traversal.walkTo({ x: dest.x, z: dest.z, level: dest.level ?? 0 });
    return true;
}

export async function gotoNpc(stop) {
    throw notImpl('primitives.gotoNpc');
}

export async function driveDialog(prefer) {
    const opts = ChatDialog.options ? ChatDialog.options() : [];
    const hit = pickPreferred(opts, prefer || []);
    if (hit && ChatDialog.chooseOption) return ChatDialog.chooseOption(hit);
    throw notImpl('primitives.driveDialog');
}

export async function openDialogue(npcName) {
    const npc = (Npcs.all() || []).find((n) => n && n.name === npcName);
    if (!npc) return false;
    const op = talkOp(npc.actions()) || 'Talk-to';
    return npc.interact(op);
}

export async function talkThrough(npcName, prefer, log) {
    await openDialogue(npcName);
    if (prefer && prefer.length) await driveDialog(prefer, log);
    return true;
}

export function talkStrict(npcName, prefer, log) {
    return talkThrough(npcName, prefer, log);
}

export async function talkChoosingBy() {
    throw notImpl('primitives.talkChoosingBy');
}
