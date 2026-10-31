use client::config::Cache;
use client::dash3d::store::ModelStore;
use client::graphics::Pix32;
use rustyscript::{Runtime, RuntimeOptions};
use std::sync::Arc;

const CHILD_ENV: &str = "SCRIPT_CLIENT_ADAPTER_V8_SHAPE_TEST_CHILD";
const TEST_NAME: &str = "selected_facts_v8::client_adapter_v8::tests::materialized_rows_have_fast_properties_and_bounded_v8_heap";

const SETUP: &str = r#"
globalThis.__rs2b0t_host = {
    tile: null,
    snapshot: {
        ingame: true,
        here: { x: 3200, z: 3200, level: 0 },
        client_adapter: {
            attached: true,
            session_generation: 7,
            modal_close_generation: 3,
            map_build_base: { x: 3200, z: 3200 },
            run_on_com_id: 91,
            run_off_com_id: 92,
            widgets: [
                {
                    component_id: 50,
                    root_component_id: 200,
                    parent_id: 200,
                    type_: 4,
                    text: 'Dragon Slayer',
                    colour: 0x55ff55,
                },
            ],
        },
        main_modal_id: 100,
        chat_modal_id: -1,
        side_modal_id: 300,
        side_tab_ifaces: [{ index: 2, id: 200 }],
        make_products: [
            {
                object_id: 2353,
                name: 'Steel bar',
                buttons: [{ qty: 1, comId: 501 }, { qty: 5, comId: 502 }],
            },
        ],
    },
};
const read = (method, ...args) =>
    __rs2b0t_selected_facts('client-adapter-reader', method, ...args);
const fast = new Function('value', 'return %HasFastProperties(value)');
globalThis.__clientAdapterFast = () => {
    const fastObject = (value) =>
        value !== null && typeof value === 'object' && fast(value);
    const modals = read('modals');
    const controls = read('runControls');
    const local = read('toLocal', 3201, 3202);
    const quests = read('questStatuses');
    const products = read('makeProducts');
    const close = read('modalCloseObservation');
    const iconA = read('itemIconPixels', 999999);
    const iconB = read('itemIconPixels', 999999);
    const questCount = Array.isArray(quests) ? quests.length : -1;
    const productCount = Array.isArray(products) ? products.length : -1;
    const buttons =
        productCount === 1 && Array.isArray(products[0].buttons)
            ? products[0].buttons
            : [];
    const modalsFast = fastObject(modals);
    const controlsFast = fastObject(controls);
    const localFast = fastObject(local);
    const closeFast = fastObject(close);
    const questFast = questCount === 1 && fastObject(quests[0]);
    const productFast = productCount === 1 && fastObject(products[0]);
    const buttonsFast = buttons.length === 2 && buttons.every(fastObject);
    const iconFast = fastObject(iconA);
    const iconData = iconFast && iconA.data instanceof Int32Array
        ? Array.from(iconA.data).join(',')
        : null;
    const ok = modalsFast
        && controlsFast
        && localFast
        && closeFast
        && questFast
        && productFast
        && buttonsFast
        && iconFast
        && iconA.width === 2
        && iconA.height === 2
        && iconA.data instanceof Int32Array
        && iconB.data instanceof Int32Array
        && iconA.data !== iconB.data
        && iconData === '11,22,33,44';
    return JSON.stringify({
        ok,
        modalsFast,
        controlsFast,
        localFast,
        closeFast,
        questCount,
        questFast,
        productCount,
        productFast,
        buttonCount: buttons.length,
        buttonsFast,
        iconFast,
        iconWidth: iconA && iconA.width,
        iconHeight: iconA && iconA.height,
        iconDataType: iconA && iconA.data && iconA.data.constructor.name,
        iconBDataType: iconB && iconB.data && iconB.data.constructor.name,
        distinctData: iconA && iconB && iconA.data !== iconB.data,
        iconData,
    });
};
globalThis.__clientAdapterRepeat = (count) => {
    let kept = 0;
    for (let i = 0; i < count; i++) {
        kept += read('modals').main;
        kept += read('runControls').onComId;
        kept += read('toLocal', 3201, 3202).lx;
        kept += read('questStatuses').length;
        kept += read('makeProducts')[0].buttons.length;
        kept += read('modalCloseObservation').generation;
        kept += read('itemIconPixels', 999999).data[0];
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

#[test]
fn materialized_rows_have_fast_properties_and_bounded_v8_heap() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated V8-native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }

    v8::V8::set_flags_from_string("--allow-natives-syntax");
    let (mut runtime, _cache) = setup();
    let diagnostics: serde_json::Value =
        serde_json::from_str(&runtime.eval::<String>("__clientAdapterFast()").unwrap())
            .expect("ClientAdapter diagnostics JSON");
    assert_eq!(
        diagnostics["ok"], true,
        "native modal, scene and product projections have fast properties: {diagnostics}"
    );
    runtime
        .eval::<usize>("__clientAdapterRepeat(1000)")
        .unwrap();
    let baseline = collected_used_heap(&mut runtime);
    assert_eq!(
        runtime
            .eval::<usize>("__clientAdapterRepeat(1000)")
            .unwrap(),
        209_000
    );
    let after = collected_used_heap(&mut runtime);
    let growth = after.saturating_sub(baseline);
    let limit = 16_384;
    println!("ClientAdapter V8 heap reads=7000 baseline={baseline} after={after} growth={growth} limit={limit}");
    assert!(
        growth <= limit,
        "collected V8 heap grew {growth} bytes; limit {limit}"
    );
}
