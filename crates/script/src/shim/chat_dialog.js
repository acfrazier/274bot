import { Execution } from '../../execution/Execution.js';
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
