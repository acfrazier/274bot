// Thin ClientAdapter reader and action boundary. Readers use one typed native
// projection from the latest posted snapshot; actions queue typed requests.
import { snap, queue, proxy } from '../shim/_kernel.js';

const nativeReader = (method, ...args) =>
    globalThis.__rs2b0t_selected_facts('client-adapter-reader', method, ...args);

function finiteInt(value) {
    return typeof value === 'number' && Number.isInteger(value);
}

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
    worldTile: () => nativeReader('worldTile'),
    serverTile: () => nativeReader('serverTile'),
    selfSlot: () => nativeReader('selfSlot'),
    inventorySize: () => nativeReader('inventorySize'),
    inventory: () => nativeReader('inventory'),
    bankSideItems: () => nativeReader('bankSideItems'),
    bankItems: () => globalThis.__rs2b0t_bank_items(),
    sceneState: () => nativeReader('sceneState'),
    ingame: () => nativeReader('ingame'),
    npcs: () => nativeReader('npcs'),
    locs: () => nativeReader('locs'),
    players: () => nativeReader('players'),
    groundItems: () => nativeReader('groundItems'),
    equipment: () => nativeReader('equipment'),
    modals: () => nativeReader('modals'),
    chatContinueComId: () => nativeReader('chatContinueComId'),
    chatOptions: () => nativeReader('chatOptions'),
    chatModalTexts: () => nativeReader('chatModalTexts'),
    activeSideTab: () => nativeReader('activeSideTab'),
    localPlayerName: () => nativeReader('localPlayerName'),
    combatLevel: () => nativeReader('combatLevel'),
    selfChat: () => nativeReader('selfChat'),
    hintTile: () => nativeReader('hintTile'),
    npcBox: (index) => nativeReader('npcBox', index),
    retaliateControls: () => nativeReader('retaliateControls'),
    inCombat: () => nativeReader('inCombat'),
    selfAnim: () => nativeReader('selfAnim'),
    selfFaceEntity: () => nativeReader('selfFaceEntity'),
    energy: () => nativeReader('energy'),
    varp: (index) => nativeReader('varp', index),
    stat: (index) => nativeReader('stat', index),
    skillCount: () => nativeReader('skillCount'),
    skillUsed: (index) => nativeReader('skillUsed', index),
    statsReady: () => nativeReader('statsReady'),
    sideTabInterface: (tab) => nativeReader('sideTabInterface', tab),
    selectButtonLabelsByVarp: (root, varp) =>
        nativeReader('selectButtonLabelsByVarp', root, varp),
    selectButtonByVarp: (root, varp, mode) =>
        nativeReader('selectButtonByVarp', root, varp, mode),
    targetButtonByBase: (root, label) => nativeReader('targetButtonByBase', root, label),
    objCatalog: () => globalThis.__rs2b0t_selected_facts('obj-records'),
    bankComId: () => nativeReader('bankComId'),
    ifText: (componentId) => nativeReader('ifText', componentId),
    countDialogOpen: () => nativeReader('countDialogOpen'),
    makeProducts: () => nativeReader('makeProducts'),
    toLocal: (x, z) => nativeReader('toLocal', x, z),
    weight: () => nativeReader('weight'),
    chat: (index) => nativeReader('chat', index),
    playerCount: () => nativeReader('playerCount'),
    npcCount: () => nativeReader('npcCount'),
    cameraYaw: () => nativeReader('cameraYaw'),
    cameraPitch: () => nativeReader('cameraPitch'),
    takingDamage: () => nativeReader('takingDamage'),
    inventorySnapshotReady: () => nativeReader('inventorySnapshotReady'),
    bankSnapshotReady: () => nativeReader('bankSnapshotReady'),
    bankSideSnapshotReady: () => nativeReader('bankSideSnapshotReady'),
    bankSnapshotGeneration: () => nativeReader('bankSnapshotGeneration'),
    runControls: () => nativeReader('runControls'),
    toWorld: (x, z) => nativeReader('toWorld', x, z),
    collisionFlags: (x, z) => nativeReader('collisionFlags', x, z),
    lastWalkPathWorld: () => nativeReader('lastWalkPathWorld'),
    loginMessage: () => nativeReader('loginMessage'),
    menuEntries: () => nativeReader('menuEntries'),
    shopInv: (componentId) => nativeReader('shopInv', componentId),
    puzzleBoardComId: () => nativeReader('puzzleBoardComId'),
    puzzleBoard: () => nativeReader('puzzleBoard'),
    puzzleBoardSize: () => nativeReader('puzzleBoardSize'),
    duelOffers: () => nativeReader('duelOffers'),
    tradeOfferOpen: () => nativeReader('tradeOfferOpen'),
    tradeConfirmOpen: () => nativeReader('tradeConfirmOpen'),
    tradeMyOffer: () => nativeReader('tradeMyOffer'),
    tradeTheirOffer: () => nativeReader('tradeTheirOffer'),
    tradeSidePack: () => nativeReader('tradeSidePack'),
    tradePartner: () => nativeReader('tradePartner'),
    tradeConfirmOffers: () => nativeReader('tradeConfirmOffers'),
    tradeConfirmReady: () => nativeReader('tradeConfirmReady'),
    closeButtonComId: (root) => nativeReader('closeButtonComId', root),
    mainModalTexts: () => nativeReader('mainModalTexts'),
    ifModelObjId: (componentId) => nativeReader('ifModelObjId', componentId),
    modalButtons: (root) => nativeReader('modalButtons', root),
    buttonByText: (root, text) => nativeReader('buttonByText', root, text),
    mainModalButtonNearText: (text) => nativeReader('mainModalButtonNearText', text),
    mainSkillMultiItems: () => nativeReader('mainSkillMultiItems'),
    questStatuses: () => nativeReader('questStatuses'),
    itemIconPixels: (id) => nativeReader('itemIconPixels', id),
    attached: () => nativeReader('attached'),
    mapBuildBase: () => nativeReader('mapBuildBase'),
    modalCloseObservation: () => nativeReader('modalCloseObservation'),
    overlayPosWorld: (x, z, height = 0, u = 0.5, v = 0.5) =>
        nativeReader('overlayPosWorld', x, z, height, u, v),
    projectAreaGameWorld: (x, z, height = 0, u = 0.5, v = 0.5) =>
        nativeReader('projectAreaGameWorld', x, z, height, u, v),
    locBox: (opts, areaGame = false) => nativeReader('locBox', opts, areaGame),
});
export const actions = proxy('actions', {
    sayPublic(value) {
        const text = String(value).trim();
        if (text.length === 0 || nativeReader('attached') !== true) return false;
        queue({ op: 'say-public', text });
        return true;
    },
    continueDialog() {
        const componentId = nativeReader('chatContinueComId');
        if (!finiteInt(componentId) || componentId === -1) return false;
        queue({ op: 'continue', component_id: componentId });
        return true;
    },
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
    setRun(on) {
        queue({ op: 'set-run', on: !!on });
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
    // Queue the existing host answer-count. Integer validation matches
    // Interactions::answer_count (non-negative i32). No dialog, or a
    // malformed count, returns false and queues nothing; Rust still
    // owns stale/closed/logout refusal if a later tick dispatches.
    answerCountDialog(value) {
        if (!finiteInt(value) || value < 0 || value > 2147483647) return false;
        if (snap().count_dialog_open !== true) return false;
        queue({ op: 'answer-count', value });
        return true;
    },
});
