import { Execution } from '../execution/Execution.js';
import { notImpl, proxy, queue } from '../../shim/_kernel.js';

function callReach(payload) {
    const fn =
        globalThis.rustyscript && globalThis.rustyscript.functions
            ? globalThis.rustyscript.functions.__rs2b0t_reach
            : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('Reach.npcDialog');
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
    if (step.kind === 'walk-near') {
        queue({
            op: 'walk-near',
            x: step.x,
            z: step.z,
            level: step.level,
            radius: step.radius,
            allow_teleports: step.allow_teleports === true,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: Number(step.request_id) || 0,
        });
        return true;
    }
    return false;
}

async function run(input, log) {
    const begin = callReach({ op: 'begin', ...input });
    if (!begin) return 'retry';
    emitLog(begin, log);
    if (begin.kind === 'notImpl') {
        throw notImpl('Reach.npcDialog', begin.reason);
    }
    if (begin.kind === 'aborted') return 'retry';
    if (begin.kind === 'done') return begin.status === 'done' || begin.status === 'unreachable' || begin.status === 'retry'
        ? begin.status
        : 'retry';
    const token = begin.token;
    let current = begin;
    while (current) {
        emitLog(current, log);
        if (current.kind === 'done') {
            return current.status === 'done' || current.status === 'unreachable' || current.status === 'retry'
                ? current.status
                : 'retry';
        }
        if (current.kind === 'aborted') return 'retry';
        if (current.kind === 'notImpl') {
            throw notImpl('Reach.npcDialog', current.reason);
        }
        if (current.kind === 'npc' || current.kind === 'walk-near') {
            dispatchVerb(current);
        } else if (current.kind !== 'wait') {
            return 'retry';
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callReach({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return 'retry';
}

function optionalOpenMs(openMs) {
    return typeof openMs === 'number' && Number.isFinite(openMs) && openMs >= 0
        ? Math.floor(openMs)
        : undefined;
}

export const Reach = proxy('Reach', {
    async entityOp(opts) {
        if (opts.expect()) return 'done';
        const entity = opts.find();
        if (!entity) return 'retry';
        const ok = await entity.interact(opts.op);
        if (!ok) return 'retry';
        const settled = await Execution.delayUntil(opts.expect, opts.expectMs ?? 5000);
        return settled ? 'done' : 'retry';
    },
    async npcDialog(opts) {
        const near = opts && opts.near ? opts.near : {};
        const openMs = optionalOpenMs(opts && opts.openMs);
        return run(
            {
                name: String(opts && opts.name != null ? opts.name : ''),
                x: near.x,
                z: near.z,
                level: near.level ?? 0,
                ...(openMs !== undefined ? { openMs } : {}),
            },
            opts && opts.log,
        );
    },
});
