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

const SETUP: &str = r#"
const itemRows = (count, idBase = 100) => Array.from({ length: count }, (_, slot) => ({
    slot,
    id: idBase + slot,
    name: `Item ${slot}`,
    count: slot + 1,
    component_id: 3214,
    ops: ['Use', 'Drop'],
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
    widgetItems(3542, 12),
    widgetItems(3538, 12),
    widgetItems(3532, 12),
    widgetItems(3539, 12),
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
        inv: itemRows(28),
        inv_size: 28,
        bank: itemRows(288),
        bank_side: itemRows(28),
        equipment: itemRows(14),
        trade_mine: itemRows(12),
        trade_theirs: itemRows(12),
        trade_side: itemRows(28),
        main_make_items: itemRows(28),
        puzzle_board: { component_id: 7000, size: 28, items: itemRows(28) },
        stats: Array.from({ length: 25 }, (_, index) => ({
            effective: index + 1,
            base: index + 1,
            xp: (index + 1) * 100,
        })),
        npc_boxes: [{
            index: 7,
            points: Array.from({ length: 8 }, (_, index) => ({ x: index * 2, y: index * 2 + 1 })),
        }],
        chat_options: Array.from({ length: 5 }, (_, index) => ({
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
        chat_modal_texts: { root: 123, texts: ['Chat'] },
        main_modal_texts: { root: 6575, texts: ['Main'] },
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
globalThis.__clientAdapterStylePaths = () => {
    const facts = globalThis.__rs2b0t_host.snapshot.client_adapter;
    const shown = read('selectButtonLabelsByVarp', 900, 43);
    const widgets = facts.widgets;
    facts.widgets = widgets.filter((row) => row.root_component_id !== 900);
    const hidden = read('selectButtonLabelsByVarp', 900, 43);
    facts.widgets = widgets;
    return { shown, hidden };
};
globalThis.__clientAdapterFast = () => {
    const itemRows = [
        ['inventory', 28],
        ['bankSideItems', 28],
        ['bankItems', 288],
        ['equipment', 14],
        ['tradeMyOffer', 12],
        ['tradeTheirOffer', 12],
        ['tradeSidePack', 28],
        ['shopInv', 28, 700],
        ['puzzleBoard', 28],
        ['mainSkillMultiItems', 28],
    ];
    const itemChecks = itemRows.map(([method, count, ...args]) =>
        rowsFast(method, count, ...args));
    const stats = Array.from({ length: read('skillCount') }, (_, index) =>
        read('stat', index));
    const chat = read('chat', 100);
    const npcBox = read('npcBox', 7);
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
        stats: stats.length === read('skillCount') && stats.length > 0 && stats.every(fastObject),
        modals: fastObject(read('modals')),
        chatOptions: options.length === 5 && options.every(fastObject),
        npcBox: npcBox.length === 8 && npcBox.every(fastObject),
        chat: chat.length === 100 && chat.every(fastObject),
        runControls: fastObject(read('runControls')),
        hintTile: fastObject(read('hintTile')),
        toWorld: fastObject(read('toWorld', 1, 2)),
        toLocal: fastObject(read('toLocal', 3201, 3202)),
        mapBuildBase: fastObject(read('mapBuildBase')),
        lastWalkPath: lastPath.length === 104 * 104 && lastPath.every(fastObject),
        shownStyles: stylePaths.shown.length === 4 && stylePaths.shown.every(fastObject),
        hiddenStyles: stylePaths.hidden.length === 4 && stylePaths.hidden.every(fastObject),
        modalButtons: buttons.length === 288 && buttons.every(fastObject),
        tradeConfirmOffers: fastObject(trade)
            && trade.mine.length === 24 && trade.mine.every(fastObject)
            && trade.theirs.length === 24 && trade.theirs.every(fastObject),
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
            chat: chat.length,
            npcBoxPoints: npcBox.length,
            chatOptions: options.length,
            widgetButtons: buttons.length,
            questStatuses: quests.length,
            products: products.length,
            makeButtons: productButtons.length,
            lastPath: lastPath.length,
        },
    });
};
globalThis.__clientAdapterRepeat = (count) => {
    let kept = 0;
    for (let iteration = 0; iteration < count; iteration++) {
        kept += read('inventory').length;
        kept += read('bankSideItems').length;
        kept += read('bankItems').length;
        kept += read('equipment').length;
        kept += read('tradeMyOffer').length;
        kept += read('tradeTheirOffer').length;
        kept += read('tradeSidePack').length;
        kept += read('shopInv', 700).length;
        kept += read('puzzleBoard').length;
        kept += read('mainSkillMultiItems').length;
        kept += read('chat', 100).length;
        kept += read('chatOptions').length;
        kept += read('npcBox', 7).length;
        kept += read('modalButtons', 100).length;
        kept += read('questStatuses').length;
        kept += read('makeProducts').length;
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
        kept += __clientAdapterStylePaths().shown.length;
        kept += __clientAdapterStylePaths().hidden.length;
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
    assert_eq!(diagnostics["counts"]["bank"], 288);
    assert_eq!(diagnostics["counts"]["chat"], 100);
    assert_eq!(diagnostics["counts"]["npcBoxPoints"], 8);
    assert_eq!(diagnostics["counts"]["widgetButtons"], 288);
    assert_eq!(diagnostics["counts"]["questStatuses"], 69);
    assert_eq!(diagnostics["counts"]["products"], 28);
    assert_eq!(diagnostics["counts"]["makeButtons"], 112);
    assert_eq!(diagnostics["counts"]["lastPath"], 104 * 104);
}

#[test]
fn client_adapter_objects_have_bounded_v8_heap() {
    if !run_in_child(HEAP_CHILD_ENV, HEAP_TEST_NAME) {
        return;
    }
    let (mut runtime, _cache) = setup();
    let warmup = runtime
        .eval::<usize>("__clientAdapterRepeat(5)")
        .unwrap();
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
        "ClientAdapter collected V8 heap rows/iteration: inventory=28 bank=288 chat=100 \
         modal-buttons=288 quests=69 products=28x4 path=10816 repeats={REPEATS}; \
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
    assert_eq!(paths["hidden"], accepted, "hidden-tab fallback filters Punch");
}
