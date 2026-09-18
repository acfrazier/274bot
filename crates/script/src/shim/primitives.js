import { notImpl, queue } from '../../../../shim/_kernel.js';
import { talkOp } from '../../../npcs/Npcs.js';
import { Traversal } from '../../../walking/Traversal.js';
import { Execution } from '../../../execution/Execution.js';

export { talkOp };

function callDialog(payload) {
    const fn =
        globalThis.rustyscript && globalThis.rustyscript.functions
            ? globalThis.rustyscript.functions.__rs2b0t_dialog
            : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('primitives.driveDialog');
    }
    return fn(payload);
}

function emitLog(step, log) {
    if (typeof step?.log === 'string' && typeof log === 'function') {
        log(step.log);
    }
}

function dispatchVerb(step) {
    if (step.kind === 'npc') {
        queue({
            op: 'npc',
            name: step.name,
            action: step.action,
            ...(typeof step.index === 'number' ? { index: step.index } : {}),
        });
        return true;
    }
    if (step.kind === 'ops') {
        for (const op of step.ops || []) queue(op);
        return true;
    }
    return false;
}

async function run(input, log) {
    const begin = callDialog({ op: 'begin', ...input });
    if (!begin) return false;
    emitLog(begin, log);
    if (begin.kind === 'notImpl') {
        throw notImpl('primitives.' + (input.kind || 'driveDialog'), begin.reason);
    }
    if (begin.kind === 'aborted') return false;
    if (begin.kind === 'done') return begin.result === true;
    const token = begin.token;
    let current = begin;
    while (current) {
        emitLog(current, log);
        if (current.kind === 'done') return current.result === true;
        if (current.kind === 'aborted') return false;
        if (current.kind === 'notImpl') {
            throw notImpl('primitives.' + (input.kind || 'driveDialog'), current.reason);
        }
        if (current.kind === 'npc' || current.kind === 'ops') {
            dispatchVerb(current);
        } else if (current.kind !== 'wait') {
            return false;
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callDialog({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return false;
}

function preferList(prefer) {
    return Array.isArray(prefer) ? prefer.map((row) => String(row)) : [];
}

function optionalGap(gapMs) {
    return typeof gapMs === 'number' && Number.isFinite(gapMs) && gapMs >= 0 ? gapMs : undefined;
}

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
