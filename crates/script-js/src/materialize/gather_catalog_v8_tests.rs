use super::*;
use rustyscript::RuntimeOptions;

const SHAPE_TEST_NAME: &str =
    "materialize::gather_catalog_v8::tests::every_table_object_has_fast_properties_from_its_cached_shape";
const CHILD_ENV: &str = "SCRIPT_GATHER_CATALOG_SHAPE_TEST_CHILD";

const SETUP: &str = r#"
globalThis.__Tile = class Tile {
    constructor(x, z, level) {
        this.x = x;
        this.z = z;
        this.level = level;
    }
};
// Tables take the `Tile` class after the op; members (op 20 and up) take their own arguments only.
globalThis.__g = (op, tile, ...rest) =>
    op < 20 ? __rs2b0t_gather_catalog(op, tile, ...rest) : __rs2b0t_gather_catalog(op, ...rest);
globalThis.__custom = { gear: [
    { name: 'Fishing bait', min: 1, restock: 1000, note: 'x' },
    { restock: 2000, name: 'Feather', min: 1 },
    { name: 'Small fishing net', min: 1, restock: 1 },
] };
/// One thunk per member that materializes a script value, each over realistic arguments; `__origin` stands
/// beside a selected `Range`.
globalThis.__ops = () => {
    const T = __Tile;
    const g = __g;
    const origin = __origin;
    const none = () => 0;
    const stocked = () => 50;
    const rich = name => (name === 'Coins' ? 1000000 : 0);
    const world = { heldCount: none, invCount: rich, bankCount: rich };
    const near = { x: 2830, z: 3430 };
    const methods = g(4, T);
    const method = methods.find(m => m.gear.some(piece => piece.restock > 1));
    const bait = method.gear.find(piece => piece.restock > 1);
    const mining = g(1, T);
    const labelled = mining.find(row => row.recommendedCombat != null) ?? mining[0];
    const everyMethod = run => () => methods.map(m => run(m));
    return {
        woodcutting: () => g(0, T),
        mining: () => g(1, T),
        fishing: () => g(2, T),
        shilo: () => g(3, T),
        methods: () => g(4, T),
        gerrant: () => g(5, T),
        harry: () => g(6, T),
        miningLabels: () => g(7, T),
        methodOptions: () => g(8, T),
        gearNames: () => g(9, T),
        whirlpool: () => g(10, T),
        shopCosts: () => g(11, T),
        gerrantOnly: () => g(12, T),
        cookSurfaces: () => g(13, T),
        miningLabel: () => g(20, T, labelled),
        resolveMethod: () => g(21, T, method.name),
        keepNames: () => g(22, T, method),
        gearLabel: () => g(23, T, method),
        hasGear: () => g(24, T, method, none),
        missingGear: () => g(25, T, method, none),
        restockPlan: () => g(26, T, method, none, stocked),
        spotMatches: () => g(27, T, [method.op, method.pair], method),
        shopCost: () => g(28, T, bait.name),
        vendorFor: () => g(29, T, bait.name, near),
        isBait: () => g(30, T, bait),
        withBait: () => g(31, T, method, 500),
        withBaitCustom: () => g(31, T, __custom, 500),
        planBuys: () => g(32, T, method, world, { near, baitQty: 500 }),
        planAcquire: () => g(33, T, method, world, { near }),
        plansCost: () => g(34, T, g(32, T, method, world)),
        shopCart: () => g(35, T, method, world, { baitQty: 500 }),
        shopableMissing: () => g(36, T, method.gear, none),
        cookSurfaceFor: () => g(40, T, 'Catherby', 'bank'),
        resolveCookCurated: () => g(41, T, 'Catherby', origin, 5, 'bank'),
        resolveCookNearest: () => g(41, T, 'Nowhere', origin, 5, 'bank'),
        everyMethodRestock: everyMethod(m => g(26, T, m, none, stocked)),
        everyMethodWithBait: everyMethod(m => g(31, T, m, 500)),
        everyMethodPlans: everyMethod(m => [g(32, T, m, world, { near }), g(33, T, m, world), g(35, T, m, world)]),
    };
};
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
    need(g(12, T) instanceof Set && g(10, T) instanceof Set, 'sets, including the selected whirlpool ids');
    // Every member that materializes a script value: the result and everything it reaches has fast properties.
    const ops = __ops();
    for (const [label, run] of Object.entries(ops)) {
        const result = run();
        need(result !== undefined && result !== null, `${label} produced nothing`);
        walk(result, `op ${label}`);
    }
    const baited = g(31, T, __custom, 500);
    need(JSON.stringify(Reflect.ownKeys(baited.gear[0])) === '["name","min","restock","note"]' && baited.gear[0].restock === 500,
        'a bait copy keeps the piece key order and takes the target');
    need(JSON.stringify(Reflect.ownKeys(baited.gear[1])) === '["restock","name","min"]' && baited.gear[1].restock === 500,
        'a present restock keeps its place in the copy');
    need(baited.gear[0] !== __custom.gear[0] && __custom.gear[0].restock === 1000 && baited.gear[2] === __custom.gear[2],
        'a bait piece is copied, any other piece is the caller object');
    need(g(41, T, 'Catherby', __origin, 5, 'bank') === g(40, T, 'Catherby', 'bank') && g(40, T, 'Catherby', 'bank') !== null,
        'the curated cook surface is the cached object');
    globalThis.__walked = seen.size;
    return true;
};
globalThis.__walkedCount = () => globalThis.__walked;
globalThis.__repeat = count => {
    const runs = Object.values(__ops());
    for (let index = 0; index < count; index++) {
        for (const run of runs) globalThis.__last = run();
    }
};
globalThis.__labels = () => Object.keys(__ops());
globalThis.__repeatOp = (label, count) => {
    const run = __ops()[label];
    for (let index = 0; index < count; index++) globalThis.__last = run();
};
"#;

/// The revision 289 selected content (its whirlpool NPCs and cook surfaces), configured on this thread, and an
/// origin one tile east of one of its ranges.
fn setup() -> Runtime {
    crate::runtime::platform::bootstrap_test_v8_platform();
    let data =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected game data");
    let range = data
        .cook_surfaces()
        .expect("cook surfaces")
        .iter()
        .find(|surface| surface.name == "Range")
        .expect("the selected content places a Range")
        .tile();
    crate::scene::supply_v2::configure(Some(data));
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    cb::install_ops(&mut runtime).unwrap();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
        .eval::<()>(&format!(
            "globalThis.__origin = new __Tile({}, {}, {});",
            range.x + 1,
            range.z,
            range.level
        ))
        .unwrap();
    runtime
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    // Twice: the second pass reclaims what the first left to concurrent marking.
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

fn expected_keys() -> String {
    let keys = |rows: &[&[CampKey]]| {
        rows.iter()
            .map(|keys| keys.iter().map(|key| key.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let wood: Vec<&[CampKey]> = gather_camps::woodcutting_camps()
        .expect("woodcutting camps")
        .iter()
        .map(|camp| camp.keys)
        .collect();
    let mining: Vec<&[CampKey]> = gather_camps::mining_camps()
        .expect("mining camps")
        .iter()
        .map(|camp| camp.keys)
        .collect();
    let fishing: Vec<&[CampKey]> = gather_camps::fishing_rows()
        .expect("fishing rows")
        .iter()
        .map(|row| row.keys.as_slice())
        .collect();
    serde_json::json!({ "wood": keys(&wood), "mining": keys(&mining), "fishing": keys(&fishing) })
        .to_string()
}

/// A fast-property check needs `%HasFastProperties`, which is a process-wide V8 flag, so the check runs in its
/// own child process.
fn in_child(name: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let output =
            std::process::Command::new(std::env::current_exe().expect("unit-test executable path"))
                .args(["--exact", name, "--nocapture"])
                .env(CHILD_ENV, "1")
                .output()
                .expect("run isolated V8-native-syntax test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "isolated V8 test failed\nstdout: {stdout}\nstderr: {stderr}"
        );
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
    if !api::test_support::embedded_game_data_available() {
        return;
    }
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
            let cache = scope
                .get_slot::<Shared>()
                .cloned()
                .expect("catalog installed");
            let count = cache.borrow().shapes.len();
            count
        };
        assert!(shapes >= 10, "shapes are cached per own-key list: {shapes}");
        assert!(
            shapes < 64,
            "the shape cache stays bounded by the table schema: {shapes}"
        );
        assert!(runtime.eval::<bool>("__check(__expected)").unwrap());
        let again = {
            let scope = runtime.deno_runtime().handle_scope();
            let cache = scope
                .get_slot::<Shared>()
                .cloned()
                .expect("catalog installed");
            let count = cache.borrow().shapes.len();
            count
        };
        assert_eq!(again, shapes, "a second read adds no shape");
    });
}

#[test]
fn materialized_tables_bound_the_collected_v8_heap_and_repeat_reads_add_nothing() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
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
    runtime
        .eval::<()>("globalThis.__repeat(1000); globalThis.__last = null;")
        .unwrap();
    let after_repeat = collected_used_heap(&mut runtime);
    let objects = {
        let rows = gather_camps::woodcutting_camps()
            .expect("woodcutting camps")
            .len()
            + gather_camps::mining_camps().expect("mining camps").len()
            + gather_camps::fishing_rows().expect("fishing rows").len();
        // Per camp row: the row, its spot, bank stand, resources array, and room for sweep, obstacles, vendor and
        // notes; plus the cook surfaces, the methods with their gear, and the vendors.
        rows * 8
            + cook::fish_camp_cook_surfaces()
                .expect("cook surfaces")
                .len()
                * 5
            + fishing_gear::fishing_methods()
                .expect("fishing methods")
                .len()
                * 6
            + 16
    };
    // `cold_native_entity_rows_bound_collected_v8_heap_growth` caps collected growth at 4 MiB for 4,096 rows,
    // so each retained object is charged 1 KiB.
    let limit = objects * 1024;
    let growth = populated.saturating_sub(baseline);
    println!(
        "Gather catalog collected V8 heap: objects<={objects} baseline={baseline} populated={populated} \
         after_repeat={after_repeat} growth={growth} limit={limit}"
    );
    assert!(
        growth <= limit,
        "the tables grew collected V8 used heap by {growth} bytes; limit {limit}"
    );
    assert!(
        after_repeat.saturating_sub(populated) <= 64 * 1024,
        "1,000 repeat reads grew collected V8 used heap from {populated} to {after_repeat}: a table is not returned by identity"
    );
    // Each operation alone: after 1,000 warming calls (the engine's tier-up allocates code and feedback on the
    // way, which is not retained rows), 1,000 more calls leave the collected heap where it was.
    let labels: Vec<String> = runtime.eval("__labels()").unwrap();
    // The 14 tables, the 17 members from `MINING_LABEL` to `SHOPABLE_MISSING`, the 2 cook members, and 5 variants.
    assert_eq!(
        labels.len(),
        38,
        "every dispatch operation has an exercise: {labels:?}"
    );
    for label in labels {
        runtime
            .eval::<()>(&format!(
                "globalThis.__repeatOp('{label}', 1000); globalThis.__last = null;"
            ))
            .unwrap();
        let before = collected_used_heap(&mut runtime);
        runtime
            .eval::<()>(&format!(
                "globalThis.__repeatOp('{label}', 1000); globalThis.__last = null;"
            ))
            .unwrap();
        let after = collected_used_heap(&mut runtime);
        let grown = after.saturating_sub(before);
        println!(
            "Gather catalog op {label}: before={before} after={after} growth_after_1000={grown}"
        );
        assert!(
            grown <= 16 * 1024,
            "{label}: 1,000 calls grew collected V8 used heap by {grown} bytes"
        );
    }
}

#[test]
fn shapes_exist_only_after_the_first_object_of_a_key_list_is_built() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
    let mut runtime = setup();
    let count = |runtime: &mut Runtime| {
        let scope = runtime.deno_runtime().handle_scope();
        let cache = scope
            .get_slot::<Shared>()
            .cloned()
            .expect("catalog installed");
        let count = cache.borrow().shapes.len();
        count
    };
    assert_eq!(count(&mut runtime), 0);
    runtime.eval::<()>("__g(4, __Tile)").unwrap();
    assert_eq!(
        count(&mut runtime),
        2,
        "method rows and gear pieces: one template each"
    );
    runtime.eval::<()>("__g(4, __Tile)").unwrap();
    assert_eq!(count(&mut runtime), 2);
}

fn cached_shapes(runtime: &mut Runtime) -> usize {
    let scope = runtime.deno_runtime().handle_scope();
    let cache = scope
        .get_slot::<Shared>()
        .cloned()
        .expect("catalog installed");
    let count = cache.borrow().shapes.len();
    count
}

/// `{ ...piece, restock: target }` for a piece carrying an index key, a symbol key or an own `__proto__`.
#[test]
fn bait_copies_follow_spread_semantics_for_index_symbol_and_proto_keys() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
    let mut runtime = setup();
    runtime
        .eval::<()>(
            r#"
(() => {
    const need = (condition, message) => { if (!condition) throw new Error(message); };
    const keys = value => JSON.stringify(Reflect.ownKeys(value).filter(key => typeof key === 'string'));
    const tag = Symbol('tag');
    const withIndex = { name: 'Feather', min: 1, restock: 100, 0: 'tag' };
    const withSymbol = { name: 'Feather', min: 1, restock: 100, [tag]: 'sym' };
    const withProto = JSON.parse('{"name":"Feather","min":1,"restock":100,"__proto__":{"x":1}}');
    const catalogShaped = { name: 'Feather', min: 1, restock: 100 };
    const absent = { name: 'Feather', min: 1 };
    const out = __g(31, __Tile, { gear: [withIndex, withSymbol, withProto, catalogShaped, absent] }, 500).gear;

    need(out[0] !== withIndex && keys(out[0]) === '["0","name","min","restock"]', `index key order ${keys(out[0])}`);
    need(out[0][0] === 'tag' && out[0].restock === 500 && withIndex.restock === 100, 'index key copied');
    need(out[1] !== withSymbol && keys(out[1]) === '["name","min","restock"]', `symbol string keys ${keys(out[1])}`);
    need(Reflect.ownKeys(out[1]).includes(tag) && out[1][tag] === 'sym' && out[1].restock === 500, 'symbol key copied');
    need(out[2] !== withProto && Object.getPrototypeOf(out[2]) === Object.prototype, 'own __proto__ did not set the prototype');
    const own = Object.getOwnPropertyDescriptor(out[2], '__proto__');
    need(own && own.value && own.value.x === 1 && own.enumerable && own.writable && own.configurable, 'own __proto__ is a data property');
    need(keys(out[2]) === '["name","min","restock","__proto__"]' && out[2].restock === 500, `proto key order ${keys(out[2])}`);
    need(out[3] !== catalogShaped && keys(out[3]) === '["name","min","restock"]' && out[3].restock === 500 && catalogShaped.restock === 100,
        'a catalog-shaped piece is copied');
    need(absent.restock === undefined, 'the caller piece is untouched');
})();
"#,
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Caller-specific key lists add nothing to the catalog: not a cached shape and not collected heap.
#[test]
fn caller_piece_shapes_leave_the_shape_cache_and_the_collected_heap_unchanged() {
    if !api::test_support::embedded_game_data_available() {
        return;
    }
    let mut runtime = setup();
    runtime
        .eval::<()>(
            r#"
globalThis.__serial = 0;
globalThis.__shaped = count => {
    for (let index = 0; index < count; index++) {
        const piece = { name: 'Feather', min: 1, restock: 100 };
        piece['extra' + globalThis.__serial++] = index;
        globalThis.__last = __g(31, __Tile, { gear: [piece] }, 500);
    }
};
__g(4, __Tile);
__shaped(1);
globalThis.__last = null;
"#,
        )
        .unwrap_or_else(|error| panic!("{error}"));
    let shapes = cached_shapes(&mut runtime);
    runtime
        .eval::<()>("__shaped(500); globalThis.__last = null;")
        .unwrap();
    assert_eq!(
        cached_shapes(&mut runtime),
        shapes,
        "500 distinct caller key lists added a cached shape"
    );
    let before = collected_used_heap(&mut runtime);
    runtime
        .eval::<()>("__shaped(2000); globalThis.__last = null;")
        .unwrap();
    let after = collected_used_heap(&mut runtime);
    let grown = after.saturating_sub(before);
    println!(
        "Gather catalog 2000 distinct caller shapes: before={before} after={after} growth={grown}"
    );
    assert_eq!(
        cached_shapes(&mut runtime),
        shapes,
        "2,000 more distinct caller key lists added a cached shape"
    );
    assert!(
        grown <= 64 * 1024,
        "2,000 distinct caller key lists grew collected V8 used heap by {grown} bytes"
    );
}
