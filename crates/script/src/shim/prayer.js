import { actions } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';

export const PROTECT_FROM_MAGIC = 'Protect from Magic';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}

function classifyOn(on) {
    if (on === undefined) return { kind: 'undefined' };
    if (typeof on === 'boolean') return { kind: 'boolean', value: on };
    return { kind: 'other', truthy: Boolean(on) };
}

async function run(step) {
    while (step && step.kind !== 'done' && step.kind !== 'aborted') {
        if (step.kind === 'if-button') {
            actions.ifButton(step.component_id);
        } else if (step.kind !== 'wait') {
            return step;
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = call({
                op: 'next',
                token: step.token,
            });
            return next?.kind !== 'wait';
        }, 0);
        step = next;
    }
    return step;
}

export const Prayer = {
    points() {
        return call({ op: 'points' }).value;
    },
    max() {
        return call({ op: 'max' }).value;
    },
    full() {
        return call({ op: 'full' }).value;
    },
    known(name) {
        return call({ op: 'known', name: name.trim() }).value;
    },
    available(name) {
        return call({ op: 'available', name: name.trim() }).value;
    },
    active(name) {
        return call({ op: 'active', name: name.trim() }).value;
    },
    async set(name, on) {
        const step = await run(
            call({
                op: 'begin-set',
                name: name.trim(),
                on: classifyOn(on),
            }),
        );
        return step?.kind === 'done' && step.ok === true;
    },
    async clear() {
        await run(call({ op: 'begin-clear' }));
    },
};
