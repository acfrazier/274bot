// Thin reader/actions (ClientAdapter shape). Scene views read the posted
// snapshot; missing members throw `not impl` — never a fake value.
import { snap, queue, proxy, optionalText, notImpl } from '../shim/_kernel.js';

const host = () => globalThis.__rs2b0t_host || {};

function varp(index) {
    const row = (snap().varps || []).find((v) => v && v.index === index);
    return row ? row.value : 0;
}

function finiteInt(n) {
    return typeof n === 'number' && Number.isInteger(n);
}

// Posted native scene origin/dims/level. Missing, unavailable, or a
// zero-dim old buffer must not convert through a silent zero base.
function sceneReach() {
    const reach = snap().reach;
    if (!reach || reach.available !== true) return null;
    if (
        !finiteInt(reach.base_x) ||
        !finiteInt(reach.base_z) ||
        !finiteInt(reach.level) ||
        !finiteInt(reach.width) ||
        !finiteInt(reach.height)
    ) {
        return null;
    }
    if (reach.width <= 0 || reach.height <= 0) return null;
    return reach;
}

function inScene(lx, lz, reach) {
    return lx >= 0 && lz >= 0 && lx < reach.width && lz < reach.height;
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
        throw notImpl('reader.chatContinueComId');
    },
    chatOptions() {
        return (snap().chat_options || []).map((o) => ({
            text: o.text,
            comId: typeof o.comId === 'number' ? o.comId : -1,
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
            base: row.base ?? 0,
            effective: row.effective ?? 0,
        };
    },
    skillCount() {
        return (snap().stats || []).length;
    },
    sideTabInterface(tab) {
        const rows = snap().side_tab_ifaces;
        if (!Array.isArray(rows)) {
            throw notImpl('reader.sideTabInterface');
        }
        const row = rows.find((r) => r && r.index === tab);
        if (!row || typeof row.id !== 'number') {
            throw notImpl('reader.sideTabInterface');
        }
        return row.id;
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
        throw notImpl('reader.bankComId');
    },
    makeProducts() {
        return (snap().make_products || []).map((p) => ({
            name: p.name,
            buttons: (p.buttons || []).map((b) => ({ qty: b.qty, comId: b.comId })),
        }));
    },
    toLocal(x, z) {
        if (!finiteInt(x) || !finiteInt(z)) return null;
        const reach = sceneReach();
        if (!reach) return null;
        const lx = x - reach.base_x;
        const lz = z - reach.base_z;
        if (!inScene(lx, lz, reach)) return null;
        return { lx, lz };
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
    walkTo(lx, lz) {
        if (!finiteInt(lx) || !finiteInt(lz)) return false;
        const reach = sceneReach();
        if (!reach) return false;
        if (!inScene(lx, lz, reach)) return false;
        queue({
            op: 'walk-to',
            x: reach.base_x + lx,
            z: reach.base_z + lz,
            level: reach.level,
        });
        return true;
    },
});
