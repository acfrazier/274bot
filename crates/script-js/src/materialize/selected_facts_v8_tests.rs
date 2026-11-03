use super::*;
use client::io::ClientRevision;
use rustyscript::{Runtime, RuntimeOptions};

const CHILD_ENV: &str = "SCRIPT_SELECTED_FACTS_SHAPE_TEST_CHILD";
const TEST_NAME: &str = "selected_facts_v8::tests::catalog_rows_keep_fast_properties_and_resolver_heap_is_bounded";

const SETUP: &str = r#"
const facts = (...args) => globalThis.__rs2b0t_selected_facts(...args);
globalThis.__catalog = null;
globalThis.__buildCatalog = () => {
    globalThis.__catalog = facts('obj-catalog');
    return __catalog.byId.size + __catalog.aliases.size;
};
globalThis.__resolve = (query, exactOnly = false) =>
    facts('obj-resolve-by-name', __catalog, query, exactOnly);
globalThis.__fast = () => {
    const hasFast = new Function('value', 'return %HasFastProperties(value)');
    const resolved = __resolve('green dragonhide');
    const recordKeys = 'id,name,cost,stackable,members,equippable,certlink,certtemplate,stackVariant';
    return hasFast(__catalog)
        && Object.keys(__catalog).join(',') === 'byId,items,aliases'
        && [...__catalog.byId.values()].every(
            row => hasFast(row) && Object.keys(row).join(',') === recordKeys
        )
        && __catalog.items.every(row => __catalog.byId.get(row.id) === row)
        && [...__catalog.aliases.values()].every(
            row => hasFast(row) && Object.keys(row).join(',') === 'words,label'
        )
        && resolved.every(hasFast)
        && resolved.length === 1
        && resolved[0] === __catalog.byId.get(1753);
};
globalThis.__repeat = count => {
    let found = 0;
    for (let index = 0; index < count; index++) {
        found += __resolve('green dragonhide').length;
    }
    return found;
};
"#;

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    crate::supply_v2::configure(Some(
        api::game_data::for_revision(ClientRevision::R289).expect("selected game data"),
    ));
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    install(&mut runtime, Arc::new(AtomicBool::new(false))).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

fn in_child(body: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated V8 shape test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    body();
}

#[test]
fn catalog_rows_keep_fast_properties_and_resolver_heap_is_bounded() {
    in_child(|| {
        let mut runtime = setup();
        let baseline = collected_used_heap(&mut runtime);
        let catalog_rows = runtime.eval::<usize>("__buildCatalog()").unwrap();
        assert!(catalog_rows > 0);
        let after_catalog = collected_used_heap(&mut runtime);
        let catalog_growth = after_catalog.saturating_sub(baseline);
        let catalog_limit = catalog_rows * 512 + 64 * 1024;
        println!("selected-facts V8 heap catalog_rows={catalog_rows} baseline={baseline} after={after_catalog} growth={catalog_growth} limit={catalog_limit}");
        assert!(
            catalog_growth <= catalog_limit,
            "collected V8 heap grew {catalog_growth} bytes for {catalog_rows} rows; limit {catalog_limit}"
        );
        assert!(
            runtime.eval::<bool>("__fast()").unwrap(),
            "catalog records, alias rows and resolved values have fast properties"
        );

        let found = runtime.eval::<usize>("__repeat(1)").unwrap();
        assert_eq!(found, 1);
        let baseline = collected_used_heap(&mut runtime);
        let iterations = 1_000;
        assert_eq!(
            runtime
                .eval::<usize>(&format!("__repeat({iterations})"))
                .unwrap(),
            iterations
        );
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        let limit = 64 * iterations + 16_384;
        println!("catalog resolver V8 heap resolver_rows={iterations} baseline={baseline} after={after} growth={growth} limit={limit}");
        assert!(
            growth <= limit,
            "collected V8 heap grew {growth} bytes; limit {limit}"
        );
    });
}
