use super::*;
use client::io::ClientRevision;
use rustyscript::{Runtime, RuntimeOptions};

const CHILD_ENV: &str = "SCRIPT_SELECTED_FACTS_SHAPE_TEST_CHILD";
const TEST_NAME: &str =
    "materialize::selected_facts_v8::tests::catalog_rows_keep_fast_properties_and_resolver_heap_is_bounded";
const MUTATION_CHILD_ENV: &str = "SCRIPT_SELECTED_FACTS_CATALOG_MUTATION_CHILD";
const MUTATION_TEST_NAME: &str =
    "materialize::selected_facts_v8::tests::catalog_ignores_script_mutation_of_the_record_array";
const SETTER_CHILD_ENV: &str = "SCRIPT_SELECTED_FACTS_CATALOG_SETTER_CHILD";
const SETTER_TEST_NAME: &str =
    "materialize::selected_facts_v8::tests::catalog_rebuild_ignores_an_array_prototype_setter";
const REWRITTEN_ID_CHILD_ENV: &str = "SCRIPT_SELECTED_FACTS_CATALOG_REWRITTEN_ID_CHILD";
const REWRITTEN_ID_TEST_NAME: &str =
    "materialize::selected_facts_v8::tests::catalog_rejects_a_rewritten_row_id";
/// Round-3 review R3-2 probe: install an index accessor on `Array.prototype`,
/// empty the cached row array so the next `obj-records` call rebuilds it, and
/// rebuild with the accessor in place.
const SETTER_REBUILD: &str = r#"
(() => {
    Object.defineProperty(Array.prototype, '0', {
        configurable: true,
        get() { return 'evil' },
        set() {},
    });
    facts('obj-records').length = 0;
    globalThis.__rows = facts('obj-records');
    globalThis.__everyRowIsPresent = () => {
        if (__rows.length !== __recordCount) return false;
        for (let index = 0; index < __rows.length; index++) {
            if (!Object.prototype.hasOwnProperty.call(__rows, index)) return false;
            const row = __rows[index];
            if (!row || typeof row !== 'object' || row.id === undefined) return false;
        }
        return true;
    };
    globalThis.__setterCatalogWorks = () => {
        const catalog = facts('obj-catalog');
        const rows = [...catalog.byId.values()];
        return catalog.byId.size === __recordCount
            && rows.every(row => row && catalog.byId.get(row.id) === row);
    };
    return __rows.length;
})()
"#;
/// Round-3 review R3-3 probe: rewrite the `id` of one listed row, then ask for
/// the catalog again. `''` means the call returned without an error.
const REWRITTEN_ID_REWRITE: &str = r#"
(() => {
    const catalog = facts('obj-catalog');
    const row = catalog.items[0];
    if (!row || row.id === undefined) return 'no listed row';
    row.id = -999;
    try {
        facts('obj-catalog');
    } catch (error) {
        return String(error);
    }
    return '';
})()
"#;
/// True while every `byId` entry maps a row to its own id and every listed
/// item is one of those rows.
const CATALOG_CHECK: &str = r#"
(() => {
    const catalog = facts('obj-catalog');
    const rows = [...catalog.byId.values()];
    return catalog.byId.size === globalThis.__recordCount
        && rows.every(row => row && row.id !== undefined && catalog.byId.get(row.id) === row)
        && catalog.items.every(row => catalog.byId.get(row.id) === row);
})()
"#;

const SETUP: &str = r#"
const facts = (...args) => globalThis.__rs2b0t_selected_facts(...args);
globalThis.__catalog = null;
globalThis.__recordRows = null;
globalThis.__buildCatalog = () => {
    globalThis.__recordRows = facts('obj-records');
    globalThis.__catalog = facts('obj-catalog');
    return __catalog.byId.size + __catalog.aliases.size;
};
globalThis.__resolve = (query, exactOnly = false) =>
    facts('obj-resolve-by-name', __catalog, query, exactOnly);
globalThis.__fast = () => {
    const hasFast = new Function('value', 'return %HasFastProperties(value)');
    const resolved = __resolve('green dragonhide');
    const catalogRecords = [...__catalog.byId.values()];
    const recordKeys = 'id,name,cost,stackable,members,equippable,certlink,certtemplate,stackVariant';
    return hasFast(__catalog)
        && Object.keys(__catalog).join(',') === 'byId,items,aliases'
        && __recordRows.length === catalogRecords.length
        && __recordRows.every(
            (row, index) => row === catalogRecords[index]
                && hasFast(row)
                && Object.keys(row).join(',') === recordKeys
        )
        && __catalog.items.every(row => __catalog.byId.get(row.id) === row)
        && [...__catalog.aliases.values()].every(
            row => hasFast(row) && Object.keys(row).join(',') === 'words,label'
        )
        && resolved.every(hasFast)
        && resolved.length === 1
        && resolved[0] === __catalog.byId.get(1753);
};
globalThis.__retainedResolutions = [];
globalThis.__repeat = count => {
    globalThis.__retainedResolutions = [];
    for (let index = 0; index < count; index++) {
        globalThis.__retainedResolutions.push(__resolve('green dragonhide'));
    }
    return globalThis.__retainedResolutions.length;
};
"#;

fn setup() -> Runtime {
    crate::runtime::platform::bootstrap_test_v8_platform();
    crate::scene::supply_v2::configure(Some(
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

fn in_child(env: &str, name: &str, body: impl FnOnce()) {
    if std::env::var_os(env).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", name, "--nocapture"])
            .env(env, "1")
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
    if !api::test_support::game_data_available() {
        return;
    }
    in_child(CHILD_ENV, TEST_NAME, || {
        let mut runtime = setup();
        let baseline = collected_used_heap(&mut runtime);
        let catalog_rows = runtime.eval::<usize>("__buildCatalog()").unwrap();
        assert!(catalog_rows > 0);
        let after_catalog = collected_used_heap(&mut runtime);
        let catalog_growth = after_catalog.saturating_sub(baseline);
        // 4,436 catalog rows grew 863,808 B on aubaine; 25% headroom covers V8 variance.
        let catalog_limit = 863_808 + 863_808 / 4;
        println!("selected-facts V8 heap catalog_rows={catalog_rows} baseline={baseline} after={after_catalog} growth={catalog_growth} limit={catalog_limit}");
        assert!(
            catalog_growth <= catalog_limit,
            "collected V8 heap grew {catalog_growth} bytes for {catalog_rows} catalog rows; limit {catalog_limit}"
        );
        assert!(
            runtime.eval::<bool>("__fast()").unwrap(),
            "obj-records and obj-catalog share fast record rows and preserve object identity"
        );

        assert_eq!(runtime.eval::<usize>("__repeat(1)").unwrap(), 1);
        let baseline = collected_used_heap(&mut runtime);
        let iterations = 1_000;
        let repeat = format!("__repeat({iterations})");
        assert_eq!(runtime.eval::<usize>(&repeat).unwrap(), iterations);
        assert_eq!(
            runtime
                .eval::<usize>("__retainedResolutions.length")
                .unwrap(),
            iterations,
            "the resolver heap measurement retains every result array"
        );
        let after = collected_used_heap(&mut runtime);
        let growth = after.saturating_sub(baseline);
        // 1,000 retained result arrays grew 68,480 B on aubaine; 25% headroom covers V8 variance.
        let limit = 68_480 + 68_480 / 4;
        println!("catalog resolver V8 heap retained_result_arrays={iterations} baseline={baseline} after={after} growth={growth} limit={limit}");
        assert!(
            growth <= limit,
            "collected V8 heap grew {growth} bytes for {iterations} retained resolver results; limit {limit}"
        );
    });
}

/// Round-2 review N1: `obj-catalog` and the script-visible `obj-records` array
/// share their row objects, so a script that reorders or empties the array it
/// received must not change the catalog.
#[test]
fn catalog_ignores_script_mutation_of_the_record_array() {
    if !api::test_support::game_data_available() {
        return;
    }
    in_child(MUTATION_CHILD_ENV, MUTATION_TEST_NAME, || {
        let mut runtime = setup();
        let rows = runtime
            .eval::<usize>("globalThis.__recordCount = facts('obj-records').length")
            .unwrap();
        assert!(rows > 0, "the selected data has obj rows");
        assert!(
            runtime.eval::<bool>(CATALOG_CHECK).unwrap(),
            "the catalog keys every row by its own id"
        );
        runtime
            .eval::<()>("facts('obj-records').reverse()")
            .unwrap();
        assert!(
            runtime.eval::<bool>(CATALOG_CHECK).unwrap(),
            "a reversed row array does not change the catalog"
        );
        runtime
            .eval::<()>("facts('obj-records').length = 0")
            .unwrap();
        assert!(
            runtime.eval::<bool>(CATALOG_CHECK).unwrap(),
            "an emptied row array does not change the catalog"
        );
    });
}

/// Round-3 review R3-2: the rebuild writes own elements, so an index setter
/// that a script put on `Array.prototype` cannot drop a row.
#[test]
fn catalog_rebuild_ignores_an_array_prototype_setter() {
    if !api::test_support::game_data_available() {
        return;
    }
    in_child(SETTER_CHILD_ENV, SETTER_TEST_NAME, || {
        let mut runtime = setup();
        let rows = runtime
            .eval::<usize>("globalThis.__recordCount = facts('obj-records').length")
            .unwrap();
        assert!(rows > 0, "the selected data has obj rows");
        assert!(
            runtime.eval::<bool>(CATALOG_CHECK).unwrap(),
            "the catalog keys every row by its own id"
        );
        assert_eq!(
            runtime.eval::<usize>(SETTER_REBUILD).unwrap(),
            rows,
            "the rebuild keeps the row count"
        );
        assert!(
            runtime.eval::<bool>("__everyRowIsPresent()").unwrap(),
            "an Array.prototype index setter drops no row from the rebuild"
        );
        assert!(
            runtime.eval::<bool>("__setterCatalogWorks()").unwrap(),
            "obj-catalog still keys every row with the setter installed"
        );
        runtime.eval::<()>("delete Array.prototype[0]").unwrap();
        assert!(
            runtime.eval::<bool>(CATALOG_CHECK).unwrap(),
            "obj-catalog builds its items after the accessor is gone"
        );
    });
}

/// Round-3 review R3-3: a script that rewrites a listed row's `id` gets an
/// error from `obj-catalog`, not an `undefined` entry in `items`.
#[test]
fn catalog_rejects_a_rewritten_row_id() {
    if !api::test_support::game_data_available() {
        return;
    }
    in_child(REWRITTEN_ID_CHILD_ENV, REWRITTEN_ID_TEST_NAME, || {
        let mut runtime = setup();
        let rows = runtime
            .eval::<usize>("facts('obj-records').length")
            .unwrap();
        assert!(rows > 0, "the selected data has obj rows");
        let error = runtime.eval::<String>(REWRITTEN_ID_REWRITE).unwrap();
        assert!(
            error.contains("not a row"),
            "obj-catalog rejects a listed id that no longer resolves; got {error:?}"
        );
    });
}
