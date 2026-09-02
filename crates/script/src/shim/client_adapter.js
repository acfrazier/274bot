// Thin reader/actions (ClientAdapter shape). Scene views read the posted
// snapshot; missing members throw `not v1` — never a fake value.
import { snap, queue, proxy, optionalText } from '../shim/_kernel.js';

const host = () => globalThis.__rs2b0t_host || {};

function varp(index) {
    const row = (snap().varps || []).find((v) => v && v.index === index);
    return row ? row.value : 0;
}

export const reader = proxy('reader', {
    worldTile() {
        return host().tile || snap().here || null;
    },
    inventorySize() {
        return typeof host().invSize === 'number' ? host().invSize : 0;
    },
    ingame() {
        return snap().ingame === true;
    },
    npcs() {
        return snap().npcs || [];
    },
    locs() {
        return snap().locs || [];
    },
    players() {
        return snap().players || [];
    },
    groundItems() {
        return snap().ground || [];
    },
    equipment() {
        return snap().equipment || [];
    },
    modals() {
        const s = snap();
        return {
            main: typeof s.main_modal_id === 'number' ? s.main_modal_id : -1,
            chat: typeof s.chat_modal_id === 'number' ? s.chat_modal_id : -1,
            side: -1,
        };
    },
    chatContinueComId() {
        return snap().chat_continue === true ? 1 : -1;
    },
    chatOptions() {
        return (snap().chat_options || []).map((o, i) => ({
            text: o.text,
            comId: i + 1,
        }));
    },
    chatModalTexts() {
        const t = optionalText(snap().chat_text);
        return t ? [t] : [];
    },
    activeSideTab() {
        return typeof snap().side_tab === 'number' ? snap().side_tab : -1;
    },
    localPlayerName() {
        return optionalText(snap().my_name);
    },
    inCombat() {
        return snap().in_combat === true;
    },
    selfAnim() {
        return snap().animating === true ? 1 : -1;
    },
    energy() {
        return typeof snap().run_energy === 'number' ? snap().run_energy : 0;
    },
    varp(index) {
        return varp(index);
    },
    stat(i) {
        const row = (snap().stats || [])[i];
        if (!row) return { name: '', xp: 0, base: 0, effective: 0 };
        return {
            name: row.name,
            xp: row.xp ?? 0,
            base: row.level ?? 0,
            effective: row.effective ?? row.level ?? 0,
        };
    },
    skillCount() {
        return (snap().stats || []).length;
    },
    sideTabInterface(_tab) {
        return -1;
    },
    selectButtonLabelsByVarp(_root, _varp) {
        return (snap().combat_styles || []).map((s) => ({ mode: s.mode, label: s.label }));
    },
    selectButtonByVarp(_root, _varp, mode) {
        const row = (snap().combat_styles || []).find((s) => s.mode === mode);
        return row ? row.component_id : -1;
    },
    targetButtonByBase(_root, label) {
        const wanted = String(label).toLowerCase();
        const row = (snap().combat_styles || []).find(
            (s) => s.label && String(s.label).toLowerCase() === wanted,
        );
        return row ? row.component_id : -1;
    },
    bankComId() {
        return snap().bank_open === true ? 1 : -1;
    },
});

export const actions = proxy('actions', {
    closeModal() {
        const s = snap();
        const main = typeof s.main_modal_id === 'number' ? s.main_modal_id : -1;
        const chat = typeof s.chat_modal_id === 'number' ? s.chat_modal_id : -1;
        if (main === -1 && chat === -1) return false;
        queue({ op: 'close-modal' });
        return true;
    },
    ifButton(componentId) {
        queue({ op: 'if-button', component_id: componentId });
        return true;
    },
    clickSideTab(tab) {
        queue({ op: 'side-tab', tab });
        return true;
    },
    setRetaliate(on) {
        queue({ op: 'set-retaliate', on: !!on });
        return true;
    },
});
