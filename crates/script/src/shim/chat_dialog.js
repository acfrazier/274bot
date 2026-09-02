import { Execution } from '../../execution/Execution.js';
import { actions, reader } from '../../../adapter/ClientAdapter.js';
import { snap, queue, proxy, optionalText } from '../../../shim/_kernel.js';

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
        const products = reader.makeProducts();
        if (products.length === 0) {
            return false;
        }
        const want = match?.toLowerCase();
        const product = want
            ? products.find((p) => p.name.toLowerCase().includes(want))
            : products[0];
        if (!product) {
            return false;
        }
        const btn =
            product.buttons.find((b) => b.qty === -1) ??
            product.buttons.filter((b) => b.qty > 0).sort((a, b) => b.qty - a.qty)[0];
        if (!btn) {
            return false;
        }
        actions.ifButton(btn.comId);
        await Execution.delayTicks(1);
        if (typeof count === 'number' && count > 0) {
            queue({ op: 'if-button', component_id: btn.comId + 1 });
        }
        return Execution.delayUntil(() => !ChatDialog.isMakeMenu(), 5000);
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
});
