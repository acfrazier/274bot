use super::*;
use rustyscript::RuntimeOptions;

const SHAPE_TEST_NAME: &str = "gather_catalog_v8::tests::every_table_object_has_fast_properties_from_its_cached_shape";
const CHILD_ENV: &str = "SCRIPT_GATHER_CATALOG_SHAPE_TEST_CHILD";

const SETUP: &str = r#"
globalThis.__Tile = class Tile {
    constructor(x, z, level) {
        this.x = x;
        this.z = z;
        this.level = level;
    }
};
globalThis.__g = (op, ...args) => __rs2b0t_gather_catalog(op, ...args);
"#;

/// Materializes every table, walks every object it reaches and checks the contract the shapes exist for.
const CHECK: &str = r#"
globalThis.__check = expected => {
    const hasFast = new Function('value', 'return %HasFastProperties(value)');
    const T = __Tile;
    const g = __g;
    const fail = message => { throw new Error(message); };
    const need = (condition, message) => { if (!condition) fail(message); };
    const seen = new Set();
    const walk = (value, label) => {
        if (value === null || typeof value !== 'object') return;
        if (seen.has(value)) return;
        seen.add(value);
        if (value instanceof Set) { for (const item of value) walk(item, label); return; }
        need(hasFast(value), `${label} does not have fast properties`);
        if (Array.isArray(value)) { value.forEach((item, index) => walk(item, `${label}[${index}]`)); return; }
        need(value instanceof T || Object.getPrototypeOf(value) === Object.prototype, `${label} prototype changed`);
        for (const key of Reflect.ownKeys(value)) {
            const descriptor = Object.getOwnPropertyDescriptor(value, key);
            need('value' in descriptor && descriptor.writable && descriptor.enumerable && descriptor.configurable,
                `${label}.${String(key)} descriptor changed`);
            walk(value[key], `${label}.${String(key)}`);
        }
    };
    const tables = { wood: g(0, T), mining: g(1, T), fishing: g(2, T) };
    for (const [name, rows] of Object.entries(tables)) {
        need(Array.isArray(rows) && rows.length === expected[name].length, `${name} row count`);
        rows.forEach((row, index) => {
            need(JSON.stringify(Reflect.ownKeys(row)) === JSON.stringify(expected[name][index]), `${name}[${index}] own-key order`);
            for (const tileKey of ['spot', 'bankStand', 'rangeStand']) {
                if (tileKey in row) need(row[tileKey] instanceof T, `${name}[${index}].${tileKey} is not a Tile`);
            }
            for (const key of ['sweep', 'avoidSpots']) {
                if (row[key] !== undefined) row[key].forEach(tile => need(tile instanceof T, `${name}[${index}].${key} holds a non-Tile`));
            }
        });
        walk(rows, name);
        need(g(name === 'wood' ? 0 : name === 'mining' ? 1 : 2, T) === rows, `${name} table is not returned by identity`);
    }
    const surfaces = g(13, T);
    walk(surfaces, 'surfaces');
    need(g(13, T) === surfaces, 'cook surfaces are not returned by identity');
    const catherby = tables.fishing.find(row => row.name === 'Catherby');
    need(catherby.rangeStand === surfaces[0].stand, 'rangeStand is the cook surface stand Tile');
    need(tables.mining.find(row => row.name === 'Al Kharid Mine').bankStand !==
        tables.mining.find(row => row.name === 'Barbarian Village').bankStand, 'distinct banks are distinct Tiles');
    need(tables.wood[0].bankStand === tables.wood[1].bankStand, 'rows of one bank share its Tile within a table');
    const methods = g(4, T);
    walk(methods, 'methods');
    need(g(4, T) === methods, 'methods are not returned by identity');
    for (const op of [5, 6, 3]) {
        const vendor = g(op, T);
        walk(vendor, `vendor ${op}`);
        need(g(op, T) === vendor, `vendor ${op} is not returned by identity`);
    }
    walk(g(7, T), 'labels');
    walk(g(11, T), 'costs');
    need(g(12, T) instanceof Set && g(10, T) === undefined, 'sets and the unavailable whirlpool ids');
    globalThis.__walked = seen.size;
    return true;
};
globalThis.__walkedCount = () => globalThis.__walked;
globalThis.__repeat = count => {
    for (let index = 0; index < count; index++) {
        globalThis.__last = [__g(0, __Tile), __g(1, __Tile), __g(2, __Tile), __g(4, __Tile), __g(13, __Tile)];
    }
};
"#;

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    cb::install_ops(&mut runtime).unwrap();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

fn expected_keys() -> String {
    let keys = |rows: &[&[CampKey]]| {
        rows.iter()
            .map(|keys| keys.iter().map(|key| key.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let wood: Vec<&[CampKey]> = gather_camps::woodcutting_camps().iter().map(|camp| camp.keys).collect();
    let mining: Vec<&[CampKey]> = gather_camps::mining_camps().iter().map(|camp| camp.keys).collect();
    let fishing: Vec<&[CampKey]> = gather_camps::fishing_rows().iter().map(|row| row.keys.as_slice()).collect();
    serde_json::json!({ "wood": keys(&wood), "mining": keys(&mining), "fishing": keys(&fishing) }).to_string()
}

/// A fast-property check needs `%HasFastProperties`, which is a process-wide V8 flag, so the check runs in its
/// own child process.
fn in_child(name: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("unit-test executable path"))
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated V8-native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}");
        assert!(
            stdout.contains(&format!("test {name} ... ok")) && stdout.contains("1 passed"),
            "isolated test did not run exactly once\nstdout: {stdout}\nstderr: {stderr}"
        );
        return;
    }
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    body();
}

#[test]
fn every_table_object_has_fast_properties_from_its_cached_shape() {
    in_child(SHAPE_TEST_NAME, || {
        let mut runtime = setup();
        runtime.eval::<()>(CHECK).unwrap();
        // No JavaScript warm-up runs before the first object is built: the check is the first use.
        runtime
            .eval::<()>(&format!("globalThis.__expected = {};", expected_keys()))
            .unwrap();
        assert!(runtime.eval::<bool>("__check(__expected)").unwrap());
        let shapes = {
            let scope = runtime.deno_runtime().handle_scope();
            let cache = scope.get_slot::<Shared>().cloned().expect("catalog installed");
            let count = cache.borrow().shapes.len();
            count
        };
        assert!(shapes >= 10, "shapes are cached per own-key list: {shapes}");
        assert!(shapes < 64, "the shape cache stays bounded by the table schema: {shapes}");
        assert!(runtime.eval::<bool>("__check(__expected)").unwrap());
        let again = {
            let scope = runtime.deno_runtime().handle_scope();
            let cache = scope.get_slot::<Shared>().cloned().expect("catalog installed");
            let count = cache.borrow().shapes.len();
            count
        };
        assert_eq!(again, shapes, "a second read adds no shape");
    });
}

#[test]
fn materialized_tables_bound_the_collected_v8_heap_and_repeat_reads_add_nothing() {
    let mut runtime = setup();
    runtime.eval::<()>(CHECK).unwrap();
    runtime.eval::<()>("globalThis.__last = null;").unwrap();
    let baseline = collected_used_heap(&mut runtime);
    runtime
        .eval::<()>(&format!("globalThis.__expected = {};", expected_keys()))
        .unwrap_or_else(|error| panic!("{error}"));
    // The shape check itself needs natives syntax; the heap measure materializes through the same calls.
    runtime
        .eval::<()>("globalThis.__repeat(1); globalThis.__last = null;")
        .unwrap();
    let populated = collected_used_heap(&mut runtime);
    runtime.eval::<()>("globalThis.__repeat(1000); globalThis.__last = null;").unwrap();
    let after_repeat = collected_used_heap(&mut runtime);
    let objects = {
        let rows = gather_camps::woodcutting_camps().len()
            + gather_camps::mining_camps().len()
            + gather_camps::fishing_rows().len();
        // Per camp row: the row, its spot, bank stand, resources array, and room for sweep, obstacles, vendor and
        // notes; plus the cook surfaces, the methods with their gear, and the vendors.
        rows * 8 + cook::FISH_CAMP_COOK_SURFACES.len() * 5 + fishing_gear::FISHING_METHODS.len() * 6 + 16
    };
    // `cold_native_entity_rows_bound_collected_v8_heap_growth` caps collected growth at 4 MiB for 4,096 rows,
    // so each retained object is charged 1 KiB.
    let limit = objects * 1024;
    let growth = populated.saturating_sub(baseline);
    println!(
        "Gather catalog collected V8 heap: objects<={objects} baseline={baseline} populated={populated} \
         after_repeat={after_repeat} growth={growth} limit={limit}"
    );
    assert!(growth <= limit, "the tables grew collected V8 used heap by {growth} bytes; limit {limit}");
    assert!(
        after_repeat.saturating_sub(populated) <= 64 * 1024,
        "1,000 repeat reads grew collected V8 used heap from {populated} to {after_repeat}: a table is not returned by identity"
    );
}

#[test]
fn shapes_exist_only_after_the_first_object_of_a_key_list_is_built() {
    let mut runtime = setup();
    let count = |runtime: &mut Runtime| {
        let scope = runtime.deno_runtime().handle_scope();
        let cache = scope.get_slot::<Shared>().cloned().expect("catalog installed");
        let count = cache.borrow().shapes.len();
        count
    };
    assert_eq!(count(&mut runtime), 0);
    runtime.eval::<()>("__g(4, __Tile)").unwrap();
    assert_eq!(count(&mut runtime), 2, "method rows and gear pieces: one template each");
    runtime.eval::<()>("__g(4, __Tile)").unwrap();
    assert_eq!(count(&mut runtime), 2);
}
