use client::config::Cache;
use client::dash3d::store::ModelStore;
use client::graphics::Pix32;
use rustyscript::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::sync::Arc;

const FAST_CHILD_ENV: &str = "SCRIPT_CLIENT_ADAPTER_FAST_SHAPE_TEST_CHILD";
const FAST_TEST_NAME: &str =
    "selected_facts_v8::client_adapter_v8::tests::client_adapter_objects_have_fast_properties";
const HEAP_CHILD_ENV: &str = "SCRIPT_CLIENT_ADAPTER_HEAP_SHAPE_TEST_CHILD";
const HEAP_TEST_NAME: &str =
    "selected_facts_v8::client_adapter_v8::tests::client_adapter_objects_have_bounded_v8_heap";
const STYLE_CHILD_ENV: &str = "SCRIPT_CLIENT_ADAPTER_STYLE_FILTER_TEST_CHILD";
const STYLE_TEST_NAME: &str =
    "selected_facts_v8::client_adapter_v8::tests::combat_style_labels_filter_on_both_tab_paths";
const CACHE_CHILD_ENV: &str = "SCRIPT_CLIENT_ADAPTER_CACHE_TEST_CHILD";
const CACHE_TEST_NAME: &str =
    "selected_facts_v8::client_adapter_v8::tests::generation_backed_lists_reuse_frozen_identity_across_posts_and_generation_moves";

const SETUP: &str = r#"
// r289 decodes the active NPC list length from an unsigned 8-bit field.
const NPC_SCENE_CAP = 255;
const itemRows = (count, idBase = 100) => Array.from({ length: count }, (_, slot) => ({
    slot,
    id: idBase + slot,
    name: `Item ${slot}`,
    count: slot + 1,
    component_id: 3214,
    ops: ['Use', 'Drop'],
}));
const PLAYER_SCENE_CAP = 2047;
const LOC_SCENE_CAP = 104 * 104 * 4;
const GROUND_SCENE_ROWS = 2048;
const entityRows = (count, idBase) => Array.from({ length: count }, (_, index) => ({
    id: idBase + index,
    index,
    name: `Entity ${index}`,
    x: 3200 + (index % 104),
    z: 3200 + Math.floor(index / 104),
    level: 0,
    distance: index % 20,
    ops: ['Talk-to', 'Examine'],
}));
const styleNames = [
    { mode: 1, label: '(Accurate)' },
    { mode: 2, label: '(Aggressive)' },
    { mode: 3, label: '(Controlled)' },
    { mode: 4, label: '(Defensive)' },
    { mode: 5, label: 'Punch' },
];
const styleWidgets = styleNames.flatMap(({ mode, label }, index) => {
    const y = 300 + index * 30;
    return [
        {
            component_id: 8000 + index * 2,
            root_component_id: 900,
            parent_id: 901,
            type_: 0,
            button_type: 5,
            script_opcode: 5,
            script_varp: 43,
            script_value: mode,
            x: 10,
            y,
            width: 20,
            height: 20,
        },
        {
            component_id: 8001 + index * 2,
            root_component_id: 900,
            parent_id: 901,
            type_: 4,
            text: label,
            x: 40,
            y,
            width: 80,
            height: 20,
        },
    ];
});
const modalWidgets = Array.from({ length: 288 }, (_, index) => ({
    component_id: 1000 + index,
    root_component_id: 100,
    parent_id: 100,
    type_: 4,
    text: `Action ${index}`,
    button_type: 1,
    button_text: 'Choose',
    hidden: false,
}));
const questWidgets = Array.from({ length: 69 }, (_, index) => ({
    component_id: 2000 + index,
    root_component_id: 200,
    parent_id: 200,
    type_: 4,
    text: `Quest ${index}`,
    colour: 0x55ff55,
}));
const widgetItems = (componentId, count) => ({
    component_id: componentId,
    root_component_id: 100,
    parent_id: 100,
    items: itemRows(count, componentId * 1000),
});
const widgets = [
    ...modalWidgets,
    ...questWidgets,
    ...styleWidgets,
    widgetItems(700, 28),
    widgetItems(3542, 14),
    widgetItems(3538, 14),
    widgetItems(3532, 14),
    widgetItems(3539, 14),
    { ...widgetItems(6669, 28), inventory_ready: true },
    { ...widgetItems(6670, 28), inventory_ready: true },
    { component_id: 3557, root_component_id: 100, parent_id: 100, text: 'Ready' },
    { component_id: 3558, root_component_id: 100, parent_id: 100, text: 'Ready' },
];
const products = Array.from({ length: 28 }, (_, index) => ({
    object_id: 3000 + index,
    name: `Product ${index}`,
    buttons: Array.from({ length: 4 }, (_, button) => ({
        qty: [1, 5, 10, 0][button],
        comId: index * 4 + button,
    })),
}));
const path = [];
for (let z = 0; z < 104; z++) {
    for (let at = 0; at < 104; at++) {
        path.push(z % 2 === 0 ? at : 103 - at, z);
    }
}
globalThis.__rs2b0t_host = {
    tile: null,
    snapshot: {
        ingame: true,
        here: { x: 3200, z: 3200, level: 0 },
        hint_tile: { x: 3201, z: 3202 },
        npcs: entityRows(NPC_SCENE_CAP, 10000),
        players: entityRows(PLAYER_SCENE_CAP, 20000),
        locs: entityRows(LOC_SCENE_CAP, 30000),
        ground: entityRows(GROUND_SCENE_ROWS, 40000),
        inv: itemRows(28),
        inv_size: 28,
        bank: itemRows(288),
        bank_side: itemRows(28),
        equipment: itemRows(14),
        trade_mine: itemRows(28),
        trade_theirs: itemRows(28),
        shop_open: true,
        shop_stock: itemRows(40, 5000),
        quest_statuses: Array.from({ length: 69 }, (_, index) => ({
            name: `Quest ${index}`,
            status: index === 0 ? '' : 'complete',
        })),
        trade_side: itemRows(28),
        main_make_items: itemRows(28),
        puzzle_board: { component_id: 7000, size: 28, items: itemRows(28) },
        stats: Array.from({ length: 25 }, (_, index) => ({
            effective: index + 1,
            base: index + 1,
            xp: (index + 1) * 100,
        })),
        npc_boxes: Array.from({ length: NPC_SCENE_CAP }, (_, index) => ({
            index,
            points: Array.from({ length: 8 }, (_, point) => ({ x: point * 2, y: point * 2 + 1 })),
        })),
        chat_options: Array.from({ length: 288 }, (_, index) => ({
            text: `Option ${index}`,
            comId: 400 + index,
        })),
        chat_lines: Array.from({ length: 100 }, (_, index) => ({
            text: `Message ${index}`,
            type: index % 4,
            username: index % 2 === 0 ? `Player ${index}` : null,
        })),
        main_modal_id: 6575,
        chat_modal_id: 123,
        side_modal_id: 300,
        side_tab_ifaces: [{ index: 0, id: 900 }, { index: 2, id: 200 }],
        combat_styles: styleNames.map(({ mode, label }) => ({ mode, label })),
        make_products: products,
        main_modal_texts: {
            root: 6575,
            texts: Array.from({ length: 288 }, (_, index) => `Main ${index}`),
        },
        client_adapter: {
            attached: true,
            session_generation: 7,
            modal_close_generation: 3,
            map_build_base: { x: 3200, z: 3200 },
            run_on_com_id: 91,
            run_off_com_id: 92,
            bank_com_id: 1,
            bank_open: true,
            bank_snapshot_generation: 10,
            chat_modal_texts: {
                root: 123,
                texts: Array.from({ length: 288 }, (_, index) => `Chat ${index}`),
            },
            read_generations: {
                npc: 1,
                player: 1,
                inv: 1,
                varp: 1,
                stat: 1,
                chat: 1,
                scene: 1,
                iface: 1,
                camera: 1,
                map_flag: 1,
                world: 1,
                player_info: 1,
                npc_info: 1,
                session: 1,
                invalidations: 1,
            },
            widgets,
            last_walk_path_local: path,
            server_tile: { level: 0 },
            projection_scene: {
                level: 0,
                camera_x: 0,
                camera_y: 0,
                camera_z: 0,
                sin_pitch: 0,
                cos_pitch: 65536,
                sin_yaw: 0,
                cos_yaw: 65536,
                heights: new Int32Array(105 * 105),
                heights_above: new Int32Array(105 * 105),
                link_below: new Uint8Array(104 * 104),
                locs: [{
                    id: 900,
                    level: 0,
                    rank: 1,
                    min_tile_x: 22,
                    max_tile_x: 22,
                    min_tile_z: 22,
                    max_tile_z: 22,
                    scene_x: 22 * 128 + 64,
                    scene_z: 22 * 128 + 64,
                    half_width: 64,
                    half_length: 64,
                    top_height: 128,
                }],
            },
        },
    },
};
const read = (method, ...args) =>
    __rs2b0t_selected_facts('client-adapter-reader', method, ...args);
const fast = new Function('value', 'return %HasFastProperties(value)');
const fastObject = (value) =>
    value !== null && typeof value === 'object' && fast(value);
const rowsFast = (method, expected, ...args) => {
    const rows = read(method, ...args);
    return Array.isArray(rows) && rows.length === expected && rows.every(fastObject);
};
const TestInvItem = function(row) {
    this.id = row.id;
    this.slot = row.slot;
    this.name = row.name;
    this.count = row.count;
};
const inventoryItems = () => __rs2b0t_inventory(2, TestInvItem);
const equipmentItems = () => __rs2b0t_inventory(11);
const rowsFastValue = (rows, expected) =>
    Array.isArray(rows) && rows.length === expected && rows.every(fastObject);
globalThis.__clientAdapterStylePaths = () => {
    const facts = globalThis.__rs2b0t_host.snapshot.client_adapter;
    const shown = read('selectButtonLabelsByVarp', 900, 43);
    const widgets = facts.widgets;
    const interfaceGeneration = facts.read_generations.iface;
    facts.widgets = widgets.filter((row) => row.root_component_id !== 900);
    facts.read_generations.iface = interfaceGeneration + 1;
    const hidden = read('selectButtonLabelsByVarp', 900, 43);
    facts.widgets = widgets;
    facts.read_generations.iface = interfaceGeneration + 2;
    return { shown, hidden };
};
globalThis.__clientAdapterFast = () => {
    const itemRows = [
        ['inventory', 28],
        ['bankSideItems', 28],
        ['bankItems', 288],
        ['equipment', 14],
        ['tradeMyOffer', 28],
        ['npcs', NPC_SCENE_CAP],
        ['locs', LOC_SCENE_CAP],
        ['players', PLAYER_SCENE_CAP],
        ['groundItems', GROUND_SCENE_ROWS],
        ['tradeTheirOffer', 28],
        ['tradeSidePack', 28],
        ['shopInv', 28, 700],
        ['puzzleBoard', 28],
        ['mainSkillMultiItems', 28],
    ];
    const itemChecks = itemRows.map(([method, count, ...args]) =>
        rowsFast(method, count, ...args));
    const facadeRows = [
        ['compatQuestsAll', 69],
        ['compatShopStock', 40],
        ['compatTradeMyOffer', 28],
        ['compatTradeTheirOffer', 28],
    ];
    const facadeChecks = facadeRows.map(([method, count]) => rowsFast(method, count));
    const stats = Array.from({ length: read('skillCount') }, (_, index) =>
        read('stat', index));
    const chat = read('chat', 100);
    const inventoryList = inventoryItems();
    const equipmentList = equipmentItems();
    const skillNames = read('skillNames');
    const skillUsed = read('skillUsedMask');
    const chatTexts = read('chatModalTexts');
    const mainTexts = read('mainModalTexts');
    const npcBox = read('npcBox', NPC_SCENE_CAP - 1);
    const options = read('chatOptions');
    const buttons = read('modalButtons', 100);
    const trade = read('tradeConfirmOffers');
    const duel = read('duelOffers');
    const quests = read('questStatuses');
    const products = read('makeProducts');
    const productButtons = products.flatMap((product) => product.buttons);
    const stylePaths = __clientAdapterStylePaths();
    const lastPath = read('lastWalkPathWorld');
    const icon = read('itemIconPixels', 999999);
    const iconAgain = read('itemIconPixels', 999999);
    const close = read('modalCloseObservation');
    const checks = {
        items: itemChecks.every(Boolean),
        facadeLists: facadeChecks.every(Boolean),
        stats: stats.length === 25 && stats.length === read('skillCount')
            && stats.length > 0 && stats.every(fastObject),
        modals: fastObject(read('modals')),
        chatOptions: options.length === 288 && options.every(fastObject),
        npcBox: npcBox.length === 8 && npcBox.every(fastObject),
        npcBoxInput: globalThis.__rs2b0t_host.snapshot.npc_boxes.length === NPC_SCENE_CAP,
        chat: chat.length === 100 && chat.every(fastObject),
        runControls: fastObject(read('runControls')),
        hintTile: fastObject(read('hintTile')),
        toWorld: fastObject(read('toWorld', 1, 2)),
        toLocal: fastObject(read('toLocal', 3201, 3202)),
        mapBuildBase: fastObject(read('mapBuildBase')),
        lastWalkPath: lastPath.length === 104 * 104 && lastPath.every(fastObject),
        shownStyles: stylePaths.shown.length === 4 && stylePaths.shown.every(fastObject),
        hiddenStyles: stylePaths.hidden.length === 4 && stylePaths.hidden.every(fastObject),
        inventoryItems: rowsFastValue(inventoryList, 28),
        equipmentItems: rowsFastValue(equipmentList, 14),
        skillNames: skillNames.length === 25 && Object.isFrozen(skillNames),
        skillUsed: skillUsed.length === 25 && Object.isFrozen(skillUsed),
        chatModalTexts: chatTexts.length === 288 && Object.isFrozen(chatTexts),
        mainModalTexts: mainTexts.length === 288 && Object.isFrozen(mainTexts),
        tradeConfirmOffers: fastObject(trade)
            && trade.mine.length === 28 && trade.mine.every(fastObject)
            && trade.theirs.length === 28 && trade.theirs.every(fastObject),
        duelOffers: fastObject(duel)
            && duel.mine.length === 28 && duel.mine.every(fastObject)
            && duel.theirs.length === 28 && duel.theirs.every(fastObject),
        questStatuses: quests.length === 69 && quests.every(fastObject),
        makeProducts: products.length === 28 && products.every(fastObject),
        makeButtons: productButtons.length === 112 && productButtons.every(fastObject),
        modalCloseState: fastObject(globalThis.__rs2b0t_client_adapter_modal_session),
        modalClose: fastObject(close),
        overlayPoint: fastObject(read('overlayPosWorld', 3222, 3222, 0)),
        areaPoint: fastObject(read('projectAreaGameWorld', 3222, 3222, 0)),
        locBox: rowsFast('locBox', 8, { x: 3222, z: 3222, id: 900 }),
        itemIcon: fastObject(icon) && fastObject(iconAgain)
            && icon.width === 2 && icon.height === 2
            && icon.data instanceof Int32Array && icon.data.length === 4
            && icon.data[0] === 11 && icon.data[1] === 22
            && icon.data[2] === 33 && icon.data[3] === 44
            && icon.data !== iconAgain.data,
    };
    return JSON.stringify({
        ok: Object.values(checks).every(Boolean),
        checks,
        counts: {
            inventory: 28,
            bank: 288,
            inventoryItems: inventoryList.length,
            equipmentItems: equipmentList.length,
            sceneNpcs: read('npcs').length,
            scenePlayers: read('players').length,
            sceneLocs: read('locs').length,
            sceneGroundItems: read('groundItems').length,
            chatModalTexts: chatTexts.length,
            mainModalTexts: mainTexts.length,
            chat: chat.length,
            statCount: stats.length,
            npcBoxPoints: npcBox.length,
            npcBoxRows: globalThis.__rs2b0t_host.snapshot.npc_boxes.length,
            chatOptions: options.length,
            widgetButtons: buttons.length,
            questStatuses: quests.length,
            questFacadeRows: read('compatQuestsAll').length,
            shopStockFacadeRows: read('compatShopStock').length,
            tradeMyOfferFacadeRows: read('compatTradeMyOffer').length,
            tradeTheirOfferFacadeRows: read('compatTradeTheirOffer').length,
            products: products.length,
            makeButtons: productButtons.length,
            lastPath: lastPath.length,
        },
    });
};
globalThis.__clientAdapterCacheInvariants = () => {
    let snapshot = globalThis.__rs2b0t_host.snapshot;
    const arraysAndObjects = (value) => {
        const arrays = [];
        const objects = [];
        const seen = new Set();
        const visit = (item) => {
            if (item === null || typeof item !== 'object' || seen.has(item)) return;
            seen.add(item);
            if (Array.isArray(item)) {
                arrays.push(item);
                for (const child of item) visit(child);
            } else {
                objects.push(item);
                for (const key of Object.keys(item)) visit(item[key]);
            }
        };
        visit(value);
        return { arrays, objects };
    };
    const deepFrozen = (value, seen = new Set()) => {
        if (value === null || typeof value !== 'object' || seen.has(value)) return true;
        seen.add(value);
        if (!Object.isFrozen(value)) return false;
        return Object.keys(value).every((key) => deepFrozen(value[key], seen));
    };
    const strictWritesThrow = new Function('arrays', 'objects', `
        'use strict';
        const attempt = (operation) => {
            try { operation(); return false; }
            catch (error) { return error instanceof TypeError; }
        };
        return {
            push: arrays.length > 0 && arrays.every((list) => attempt(() => list.push(null))),
            index: arrays.length > 0 && arrays.every((list) => attempt(() => { list[0] = null; })),
            sort: arrays.length > 0 && arrays.every((list) => attempt(() => list.sort(() => 0))),
            rowField: objects.length === 0 || objects.every((row) => attempt(() => { row.count = 5; })),
            ops: objects.filter((row) => Array.isArray(row.ops))
                .every((row) => attempt(() => row.ops.push('x'))),
            cert: objects.length === 0 || objects.every((row) => attempt(() => { row.cert = 1; })),
        };
    `);
    const writesThrow = (value) => {
        const { arrays, objects } = arraysAndObjects(value);
        return Object.values(strictWritesThrow(arrays, objects)).every(Boolean);
    };
    const sameIdentity = (left, right) => {
        const leftArrays = arraysAndObjects(left).arrays;
        const rightArrays = arraysAndObjects(right).arrays;
        return left === right
            && leftArrays.length > 0
            && leftArrays.length === rightArrays.length
            && leftArrays.every((list, index) => list === rightArrays[index]);
    };
    const clonePost = (value) => {
        if (Array.isArray(value)) return value.map(clonePost);
        if (
            value !== null &&
            typeof value === 'object' &&
            Object.getPrototypeOf(value) === Object.prototype
        ) {
            const copy = {};
            for (const [key, child] of Object.entries(value)) copy[key] = clonePost(child);
            return copy;
        }
        return value;
    };
    const cases = [
        ['reader.inventory', () => read('inventory'), ['inv', 'iface']],
        ['reader.bankSideItems', () => read('bankSideItems'), ['inv', 'iface']],
        ['reader.bankItems', () => read('bankItems'), ['inv', 'iface']],
        ['reader.npcs', () => read('npcs'), ['npc', 'npc_info', 'player', 'player_info', 'scene', 'world']],
        ['reader.players', () => read('players'), ['player', 'player_info', 'scene', 'world']],
        ['reader.locs', () => read('locs'), ['scene', 'map_flag', 'world', 'player', 'player_info', 'npc_info']],
        ['reader.groundItems', () => read('groundItems'), ['scene', 'world', 'player', 'player_info', 'map_flag']],
        ['reader.equipment', () => read('equipment'), ['inv', 'iface']],
        ['reader.chatOptions', () => read('chatOptions'), ['iface', 'chat']],
        ['reader.chatModalTexts', () => read('chatModalTexts'), ['iface', 'chat']],
        ['reader.chat(100)', () => read('chat', 100), ['chat']],
        ['reader.shopInv(700)', () => read('shopInv', 700), ['iface', 'inv']],
        ['reader.puzzleBoard', () => read('puzzleBoard'), ['iface', 'inv']],
        ['reader.duelOffers', () => read('duelOffers'), ['iface', 'inv']],
        ['reader.tradeMyOffer', () => read('tradeMyOffer'), ['iface', 'inv']],
        ['reader.tradeTheirOffer', () => read('tradeTheirOffer'), ['iface', 'inv']],
        ['reader.tradeSidePack', () => read('tradeSidePack'), ['iface', 'inv']],
        ['reader.tradeConfirmOffers', () => read('tradeConfirmOffers'), ['iface', 'inv']],
        ['reader.mainModalTexts', () => read('mainModalTexts'), ['iface']],
        ['reader.modalButtons(100)', () => read('modalButtons', 100), ['iface']],
        ['reader.selectButtonLabelsByVarp', () => read('selectButtonLabelsByVarp', 900, 43), ['iface', 'varp']],
        ['reader.mainSkillMultiItems', () => read('mainSkillMultiItems'), ['iface', 'inv']],
        ['reader.questStatuses', () => read('questStatuses'), ['iface', 'varp']],
        ['reader.makeProducts', () => read('makeProducts'), ['iface', 'chat']],
        ['Inventory.items', () => inventoryItems(), ['inv']],
        ['Equipment.items', () => equipmentItems(), ['inv']],
        ['Quests.all', () => read('compatQuestsAll'), ['iface', 'varp']],
        ['Shop.stock', () => read('compatShopStock'), ['iface', 'inv']],
        ['Trade.myOffer', () => read('compatTradeMyOffer'), ['iface', 'inv']],
        ['Trade.theirOffer', () => read('compatTradeTheirOffer'), ['iface', 'inv']],
    ];
    const results = cases.map(([name, get, families]) => {
        const first = get();
        const again = get();
        return {
            name,
            get,
            families,
            first,
            sameArray: sameIdentity(first, again),
            frozen: deepFrozen(first),
            writesThrow: writesThrow(first),
        };
    });
    const staticCases = ['skillNames', 'skillUsedMask'].map((method) => {
        const first = read(method);
        const again = read(method);
        return {
            method,
            first,
            sameArray: sameIdentity(first, again),
            frozen: deepFrozen(first),
            writesThrow: writesThrow(first),
        };
    });

    // A later posted snapshot with fresh row arrays but unchanged generations
    // must reuse each reader's exact cached V8 identity.
    snapshot = clonePost(snapshot);
    globalThis.__rs2b0t_host.snapshot = snapshot;
    for (const result of results) {
        result.sameAcrossPost = sameIdentity(result.first, result.get());
    }
    for (const result of staticCases) {
        result.sameAcrossPost = sameIdentity(result.first, read(result.method));
    }
    for (const result of results) {
        for (const family of result.families) {
            snapshot.client_adapter.read_generations[family] += 1;
        }
        result.newAfterGenerationMove = result.get() !== result.first;
    }

    const dynamicResults = results.map((result) => ({
        name: result.name,
        sameArray: result.sameArray,
        sameAcrossPost: result.sameAcrossPost,
        frozen: result.frozen,
        writesThrow: result.writesThrow,
        newAfterGenerationMove: result.newAfterGenerationMove,
    }));
    const staticLists = staticCases.map((result) => ({
        name: `reader.${result.method}`,
        sameArray: result.sameArray,
        sameAcrossPost: result.sameAcrossPost,
        frozen: result.frozen,
        writesThrow: result.writesThrow,
        source: 'static skill catalogue; no game generation',
    }));
    const failures = [...dynamicResults, ...staticLists].filter((result) =>
        !result.sameArray || !result.sameAcrossPost || !result.frozen || !result.writesThrow
            || ('newAfterGenerationMove' in result && !result.newAfterGenerationMove));
    return JSON.stringify({
        ok: failures.length === 0,
        failures,
        count: dynamicResults.length,
        names: dynamicResults.map((result) => result.name),
        staticLists,
    });
};
globalThis.__clientAdapterRepeat = (count) => {
    let kept = 0;
    for (let iteration = 0; iteration < count; iteration++) {
        kept += read('inventory').length;
        kept += read('bankSideItems').length;
        kept += read('bankItems').length;
        kept += read('equipment').length;
        kept += read('npcs').length;
        kept += read('players').length;
        kept += read('locs').length;
        kept += read('groundItems').length;
        kept += read('tradeMyOffer').length;
        kept += read('tradeTheirOffer').length;
        kept += read('tradeSidePack').length;
        kept += read('shopInv', 700).length;
        kept += read('puzzleBoard').length;
        kept += read('mainSkillMultiItems').length;
        kept += read('chat', 100).length;
        kept += read('chatOptions').length;
        kept += read('chatModalTexts').length;
        kept += read('mainModalTexts').length;
        kept += read('modalButtons', 100).length;
        kept += read('selectButtonLabelsByVarp', 900, 43).length;
        kept += read('questStatuses').length;
        kept += read('makeProducts').length;
        kept += read('compatQuestsAll').length;
        kept += read('compatShopStock').length;
        kept += read('compatTradeMyOffer').length;
        kept += read('compatTradeTheirOffer').length;
        kept += read('skillNames').length;
        kept += read('skillUsedMask').length;
        kept += inventoryItems().length;
        kept += equipmentItems().length;
        kept += read('lastWalkPathWorld').length;
        kept += read('stat', iteration % read('skillCount')).effective;
        kept += read('modals').main;
        kept += read('runControls').onComId;
        kept += read('toWorld', 1, 2).x;
        kept += read('toLocal', 3201, 3202).lx;
        kept += read('mapBuildBase').x;
        kept += read('tradeConfirmOffers').mine.length;
        kept += read('duelOffers').mine.length;
        kept += read('modalCloseObservation').generation;
        kept += read('overlayPosWorld', 3222, 3222, 0).x;
        kept += read('projectAreaGameWorld', 3222, 3222, 0).x;
        kept += read('locBox', { x: 3222, z: 3222, id: 900 }).length;
        kept += read('itemIconPixels', 999999).width;
    }
    return kept;
};
"#;

fn seed_item_icon() -> Arc<Cache> {
    let cache = Arc::new(Cache::default());
    api::snapshot::set_client_adapter_cache_for_tests(Arc::clone(&cache));
    let mut sprite = Pix32::new(2, 2);
    sprite.ohi = -1;
    sprite.data.copy_from_slice(&[11, 22, 33, 44]);
    ModelStore::instance()
        .lock()
        .expect("model store")
        .put_sprite(Arc::new(sprite), 999_999);
    cache
}

fn setup() -> (Runtime, Arc<Cache>) {
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    super::super::install(
        &mut runtime,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    crate::inventory_v8::install(&mut runtime).unwrap();
    let cache = seed_item_icon();
    runtime.eval::<()>(SETUP).unwrap();
    (runtime, cache)
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

fn run_in_child(environment: &str, test_name: &str) -> bool {
    if std::env::var_os(environment).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", test_name, "--nocapture"])
        .env(environment, "1")
        .output()
        .expect("run isolated V8 test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    print!("{stdout}");
    false
}

#[test]
fn client_adapter_objects_have_fast_properties() {
    if !run_in_child(FAST_CHILD_ENV, FAST_TEST_NAME) {
        return;
    }
    let (mut runtime, _cache) = setup();
    let diagnostics: Value =
        serde_json::from_str(&runtime.eval::<String>("__clientAdapterFast()").unwrap())
            .expect("ClientAdapter fast-properties diagnostics");
    assert_eq!(
        diagnostics["ok"],
        json!(true),
        "ClientAdapter native objects and maximum row fixtures must be fast: {diagnostics}"
    );
    assert_eq!(diagnostics["counts"]["inventory"], 28);
    assert_eq!(diagnostics["counts"]["inventoryItems"], 28);
    assert_eq!(diagnostics["counts"]["equipmentItems"], 14);
    assert_eq!(diagnostics["counts"]["sceneNpcs"], 255);
    assert_eq!(diagnostics["counts"]["scenePlayers"], 2047);
    assert_eq!(diagnostics["counts"]["sceneLocs"], 104 * 104 * 4);
    assert_eq!(diagnostics["counts"]["sceneGroundItems"], 2048);
    assert_eq!(diagnostics["counts"]["chatModalTexts"], 288);
    assert_eq!(diagnostics["counts"]["mainModalTexts"], 288);
    assert_eq!(diagnostics["counts"]["bank"], 288);
    assert_eq!(diagnostics["counts"]["chat"], 100);
    assert_eq!(diagnostics["counts"]["npcBoxPoints"], 8);
    assert_eq!(diagnostics["counts"]["statCount"], 25);
    assert_eq!(diagnostics["counts"]["npcBoxRows"], 255);
    assert_eq!(diagnostics["counts"]["widgetButtons"], 288);
    assert_eq!(diagnostics["counts"]["questStatuses"], 69);
    assert_eq!(diagnostics["counts"]["products"], 28);
    assert_eq!(diagnostics["counts"]["makeButtons"], 112);
    assert_eq!(diagnostics["counts"]["lastPath"], 104 * 104);
    assert_eq!(diagnostics["counts"]["questFacadeRows"], 69);
    assert_eq!(diagnostics["counts"]["shopStockFacadeRows"], 40);
    assert_eq!(diagnostics["counts"]["tradeMyOfferFacadeRows"], 28);
    assert_eq!(diagnostics["counts"]["tradeTheirOfferFacadeRows"], 28);
}

#[test]
fn generation_backed_lists_reuse_frozen_identity_across_posts_and_generation_moves() {
    if !run_in_child(CACHE_CHILD_ENV, CACHE_TEST_NAME) {
        return;
    }
    let (mut runtime, _cache) = setup();
    let diagnostics: Value = serde_json::from_str(
        &runtime
            .eval::<String>("__clientAdapterCacheInvariants()")
            .unwrap(),
    )
    .expect("reader list cache diagnostics");
    assert_eq!(
        diagnostics["ok"],
        json!(true),
        "cached reader arrays must be shared and frozen only for one generation: {diagnostics}"
    );
    assert_eq!(diagnostics["count"], 30);
}

#[test]
fn client_adapter_objects_have_bounded_v8_heap() {
    if !run_in_child(HEAP_CHILD_ENV, HEAP_TEST_NAME) {
        return;
    }
    let (mut runtime, _cache) = setup();
    let warmup = runtime.eval::<usize>("__clientAdapterRepeat(5)").unwrap();
    assert!(warmup > 0);
    let baseline = collected_used_heap(&mut runtime);
    const REPEATS: usize = 20;
    let repeated = runtime
        .eval::<usize>(&format!("__clientAdapterRepeat({REPEATS})"))
        .unwrap();
    assert!(repeated > warmup);
    let after = collected_used_heap(&mut runtime);
    let growth = after.saturating_sub(baseline);
    let limit = 64 * 1024;
    println!(
        "ClientAdapter collected V8 heap rows/iteration: inventory=28 bank=288 \
         scene-npcs=255 npc-box-points=8 chat=100 modal-buttons=288 \
         facade-quests=69 shop-stock=40 trade-mine=28 trade-theirs=28 \
         products=28x4 path=10816 repeats={REPEATS}; \
         baseline={baseline} after={after} growth={growth} limit={limit}"
    );
    assert!(
        growth <= limit,
        "collected V8 heap grew {growth} bytes; limit {limit}"
    );
}

#[test]
fn combat_style_labels_filter_on_both_tab_paths() {
    if !run_in_child(STYLE_CHILD_ENV, STYLE_TEST_NAME) {
        return;
    }
    let (mut runtime, _cache) = setup();
    let paths: Value = serde_json::from_str(
        &runtime
            .eval::<String>("JSON.stringify(__clientAdapterStylePaths())")
            .unwrap(),
    )
    .expect("combat style label paths");
    let accepted = json!([
        { "mode": 1, "label": "(Accurate)" },
        { "mode": 2, "label": "(Aggressive)" },
        { "mode": 3, "label": "(Controlled)" },
        { "mode": 4, "label": "(Defensive)" },
    ]);
    assert_eq!(paths["shown"], accepted, "shown-tab widgets filter Punch");
    assert_eq!(
        paths["hidden"], accepted,
        "hidden-tab fallback filters Punch"
    );
}
