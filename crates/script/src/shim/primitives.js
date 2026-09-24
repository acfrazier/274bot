import { notImpl, runMachine } from '../../../../shim/_kernel.js';
import { talkOp } from '../../../npcs/Npcs.js';
import { Traversal } from '../../../walking/Traversal.js';

export { talkOp };

// One `dialog` machine await; Rust drives the pages and writes the frozen
// log lines through the caller's `log`.
async function run(input, log) {
    const out = await runMachine('dialog', input, {
        log: typeof log === 'function' ? log : undefined,
    });
    if (out.kind === 'refused') {
        throw notImpl('primitives.' + (input.kind || 'driveDialog'), out.reason);
    }
    return out.kind === 'done' && out.value === true;
}

function preferList(prefer) {
    return Array.isArray(prefer) ? prefer.map((row) => String(row)) : [];
}

function optionalGap(gapMs) {
    return typeof gapMs === 'number' && Number.isFinite(gapMs) && gapMs >= 0 ? gapMs : undefined;
}

export function pickPreferred(options, prefer) {
    const opts = options || [];
    const i = globalThis.__rs2b0t_pick_preferred(opts.map(String), [...(prefer || [])].map(String));
    return i < 0 ? null : opts[i];
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

// Ladder hops are not driven: the walk is the frozen final
// `walkResilient(dest, { radius, attempts: 3, log })`, awaited for its result.
export async function walkWithHops(dest, radius, _hops, log) {
    if (!dest || typeof dest.x !== 'number') return false;
    return Traversal.walkResilient(
        { x: dest.x, z: dest.z, level: dest.level ?? 0 },
        { radius: radius ?? 0, attempts: 3, log },
    );
}

export async function gotoNpc(stop) {
    throw notImpl('primitives.gotoNpc');
}

export async function driveDialog(prefer, log, gapMs) {
    return run(
        {
            kind: 'drive',
            prefer: preferList(prefer),
            ...(optionalGap(gapMs) !== undefined ? { gapMs: optionalGap(gapMs) } : {}),
        },
        log,
    );
}

export async function openDialogue(npcName, log) {
    return run({ kind: 'open', npc: String(npcName ?? '') }, log);
}

export async function talkThrough(npcName, prefer, log, gapMs) {
    return run(
        {
            kind: 'talk',
            npc: String(npcName ?? ''),
            prefer: preferList(prefer),
            ...(optionalGap(gapMs) !== undefined ? { gapMs: optionalGap(gapMs) } : {}),
        },
        log,
    );
}

export function talkStrict(npcName, prefer, log) {
    return talkThrough(npcName, prefer, log);
}

export async function talkChoosingBy() {
    throw notImpl('primitives.talkChoosingBy');
}
