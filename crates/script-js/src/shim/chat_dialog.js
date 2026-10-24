import { reader } from '../../../adapter/ClientAdapter.js';
import { snap, proxy, optionalText, notImpl, runMachine } from '../../../shim/_kernel.js';

// One `chat-dialog` machine await: Rust picks the product/option/row,
// presses it and owns the waits.
async function run(kind, args) {
    const out = await runMachine('chat-dialog', { kind, ...args });
    if (out.kind === 'refused') throw notImpl('ChatDialog.' + kind, out.reason);
    return out.kind === 'done' && out.value === true;
}

const text = (match) => (match == null ? null : String(match));

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
        return run('make', { match: text(match) });
    },
    async makeX(match, count) {
        return run('makeX', { match: String(match ?? ''), count: count ?? 0 });
    },
    async continue() {
        return run('continue', {});
    },
    async chooseOption(match) {
        return run('chooseOption', { match: text(match) });
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
    async makeFromPanel(match, op) {
        return run('makeFromPanel', { match: String(match ?? ''), op: op == null ? null : String(op) });
    },
    async makeFromPanelMax(match) {
        return run('makeFromPanelMax', { match: String(match ?? '') });
    },
    async makeOne() {
        throw notImpl('ChatDialog.makeOne');
    },
});
