// Thin ClientAdapter reader and action boundary. Readers use one typed native
// projection from the latest posted snapshot; actions queue typed requests.
import { queue, proxy } from '../shim/_kernel.js';

const nativeReader = (method, ...args) =>
    globalThis.__rs2b0t_selected_facts('client-adapter-reader', method, ...args);

// One native decision per action: it reads the posted snapshot and answers
// with what to queue, or `null` when the action must not be queued.
const nativeAction = (method, ...args) =>
    globalThis.__rs2b0t_selected_facts('client-adapter-action', method, ...args);

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
        const text = nativeAction('sayPublic', value);
        if (text === null) return false;
        queue({ op: 'say-public', text });
        return true;
    },
    continueDialog() {
        const componentId = nativeAction('continueDialog');
        if (componentId === null) return false;
        queue({ op: 'continue', component_id: componentId });
        return true;
    },
    closeModal() {
        if (nativeAction('closeModal') !== true) return false;
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
        const tile = nativeAction('walkTo', lx, lz);
        if (tile === null) return false;
        const [x, z, level] = tile;
        queue({ op: 'walk-to', x, z, level });
        return true;
    },
    // Queue the existing host answer-count. Rust validates the count and the
    // open dialog (Interactions::answer_count: non-negative i32) and still owns
    // stale/closed/logout refusal if a later tick dispatches.
    answerCountDialog(value) {
        if (nativeAction('answerCountDialog', value) !== true) return false;
        queue({ op: 'answer-count', value });
        return true;
    },
});
