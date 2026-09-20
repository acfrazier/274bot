// Our Modals module. close / closeIfOpen are Rust-owned: this shim marshals
// the call, queues the one returned close-modal and reports bool vs void.
// It does not decide a wait, walk a close button or send a second close.
import { snap, queue, notImpl, proxy } from '../../../shim/_kernel.js';
import { Execution } from '../../execution/Execution.js';

function callModals(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_modals
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('Modals');
    }
    return fn(payload);
}

function settle(kind, step) {
    if (kind === 'closeIfOpen') return undefined;
    return step.result === true;
}

function abortedResult(kind) {
    if (kind === 'closeIfOpen') return undefined;
    return false;
}

async function run(kind) {
    const begin = callModals({ op: 'begin', kind });
    if (!begin) return abortedResult(kind);
    if (begin.kind === 'notImpl') throw notImpl('Modals.' + kind, begin.reason);
    if (begin.kind === 'aborted') return abortedResult(kind);
    if (begin.kind === 'done') return settle(kind, begin);
    const token = begin.token;
    let current = begin;
    while (current) {
        if (current.kind === 'done') return settle(kind, current);
        if (current.kind === 'aborted') return abortedResult(kind);
        if (current.kind === 'notImpl') throw notImpl('Modals.' + kind, current.reason);
        if (current.kind === 'close-modal') {
            queue({ op: 'close-modal' });
        } else if (current.kind !== 'wait') {
            return abortedResult(kind);
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callModals({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return abortedResult(kind);
}

export const Modals = proxy('Modals', {
    main() {
        const id = snap().main_modal_id;
        return typeof id === 'number' ? id : -1;
    },
    isOpen() {
        return Modals.main() !== -1;
    },
    close() {
        return run('close');
    },
    closeIfOpen() {
        return run('closeIfOpen');
    },
});
