import { Execution } from '../../execution/Execution.js';
import { actions, reader } from '../../../adapter/ClientAdapter.js';
import { snap, queue, proxy, optionalText, notImpl } from '../../../shim/_kernel.js';

function callProduction(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_production
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('ChatDialog');
    }
    return fn(payload);
}

function dispatchVerb(step) {
    if (step.kind !== 'ops') return false;
    for (const op of step.ops || []) queue(op);
    return true;
}

async function run(kind, match, count) {
    const begin = callProduction({
        op: 'begin',
        kind,
        match: match ?? '',
        count: count ?? 0,
    });
    if (!begin) return false;
    if (begin.kind === 'notImpl') throw notImpl('ChatDialog.' + kind, begin.reason);
    if (begin.kind === 'aborted') return false;
    if (begin.kind === 'done') return begin.result === true;
    const token = begin.token;
    let current = begin;
    while (current) {
        if (current.kind === 'done') return current.result === true;
        if (current.kind === 'aborted') return false;
        if (current.kind === 'notImpl') throw notImpl('ChatDialog.' + kind, current.reason);
        if (current.kind === 'ops') {
            dispatchVerb(current);
        } else if (current.kind !== 'wait') {
            return false;
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = callProduction({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    return false;
}

export const ChatDialog = proxy('ChatDialog', {
    isOpen() {
        const s = snap();
        return s.chat_modal_id !== undefined && s.chat_modal_id !== -1;
    },
    canContinue() {
        return snap().chat_continue === true;
    },
    options() {
        return (snap().chat_options || []).map((o) => o.text);
    },
    texts() {
        const t = optionalText(snap().chat_text);
        return t ? [t] : [];
    },
    isMakeMenu() {
        return reader.makeProducts().length > 0;
    },
    makeProducts() {
        return reader.makeProducts().map((p) => p.name);
    },
    async make(match) {
        const products = reader.makeProducts();
        if (products.length === 0) {
            return false;
        }
        const want = match?.toLowerCase();
        const product = want
            ? products.find((p) => p.name.toLowerCase().includes(want))
            : products[0];
        const btn = product?.buttons.filter((b) => b.qty > 0).sort((a, b) => b.qty - a.qty)[0];
        if (!btn) {
            return false;
        }
        const modalsBefore = reader.modals();
        const usingChat = modalsBefore.chat !== -1;
        const before = usingChat ? modalsBefore.chat : modalsBefore.main;
        if (!actions.ifButton(btn.comId)) {
            return false;
        }
        return Execution.delayUntil(() => {
            const m = reader.modals();
            return (usingChat ? m.chat : m.main) !== before;
        }, 3000);
    },
    async makeX(match, count) {
        return run('makeX', String(match ?? ''), count);
    },
    async continue() {
        if (!ChatDialog.canContinue()) return false;
        queue({ op: 'continue' });
        const before = snap().chat_modal_id;
        return Execution.delayUntil(
            () => snap().chat_modal_id !== before || !snap().chat_continue,
            3000,
        );
    },
    async chooseOption(match) {
        const opts = snap().chat_options || [];
        if (opts.length === 0) return false;
        const wanted = match?.toLowerCase();
        let pick = 1;
        if (wanted) {
            const idx = opts.findIndex((o) => String(o.text).toLowerCase().includes(wanted));
            if (idx === -1) return false;
            pick = idx + 1;
        }
        queue({ op: 'answer', option: pick });
        const before = snap().chat_modal_id;
        return Execution.delayUntil(
            () => snap().chat_modal_id !== before || snap().chat_continue,
            3000,
        );
    },
    isMainMakePanel() {
        if (snap().main_make_available !== true) {
            throw notImpl('ChatDialog.isMainMakePanel');
        }
        return (snap().main_make_items || []).length > 0;
    },
    mainMakeProducts() {
        if (snap().main_make_available !== true) {
            throw notImpl('ChatDialog.mainMakeProducts');
        }
        return (snap().main_make_items || []).map((row) => row.name ?? '');
    },
    async makeFromPanel() {
        throw notImpl('ChatDialog.makeFromPanel');
    },
    async makeFromPanelMax(match) {
        return run('makeFromPanelMax', String(match ?? ''), 0);
    },
    async makeOne() {
        throw notImpl('ChatDialog.makeOne');
    },
});
