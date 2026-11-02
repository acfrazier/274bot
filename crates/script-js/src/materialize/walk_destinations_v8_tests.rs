use super::*;
use rustyscript::RuntimeOptions;

const SHAPE_TEST_NAME: &str =
    "walk_destinations_v8::tests::every_destination_uses_cached_fast_shapes";
const CHILD_ENV: &str = "SCRIPT_WALK_DESTINATIONS_SHAPE_TEST_CHILD";

const SETUP: &str = r#"
globalThis.__Tile = class Tile {
    constructor(x, z, level) {
        this.x = x;
        this.z = z;
        this.level = level;
    }
};
globalThis.__catalog = () => __rs2b0t_walk_catalog(__Tile);
"#;

fn setup() -> Runtime {
    crate::platform::bootstrap_test_v8_platform();
    let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
    cb::install_ops(&mut runtime).unwrap();
    install(&mut runtime).unwrap();
    runtime.eval::<()>(SETUP).unwrap();
    runtime
}

fn expected_catalog() -> String {
    serde_json::json!(script::native::walk::walk_destinations()
        .iter()
        .map(|destination| {
            (
                destination.name,
                destination.x,
                destination.z,
                destination.level,
            )
        })
        .collect::<Vec<_>>())
    .to_string()
}

fn collected_used_heap(runtime: &mut Runtime) -> usize {
    let isolate = runtime.deno_runtime().v8_isolate();
    isolate.low_memory_notification();
    isolate.low_memory_notification();
    isolate.get_heap_statistics().used_heap_size()
}

/// V8 native syntax is process-wide, so the shape check runs in an isolated child process.
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
fn every_destination_uses_cached_fast_shapes() {
    in_child(SHAPE_TEST_NAME, || {
        let mut runtime = setup();
        let expected = expected_catalog();
        runtime
            .eval::<()>(&format!("globalThis.__expected = {expected};"))
            .unwrap();
        runtime
            .eval::<()>(
                r#"
                const hasFast = new Function('value', 'return %HasFastProperties(value)');
                globalThis.__check = expected => {
                    const [destinations, options] = __catalog();
                    if (!hasFast(destinations) || !hasFast(options)) return false;
                    if (destinations.length !== expected.length || options.length !== expected.length) return false;
                    for (let index = 0; index < expected.length; index++) {
                        const [name, x, z, level] = expected[index];
                        const row = destinations[index];
                        if (!hasFast(row) || !hasFast(row.tile)) return false;
                        if (JSON.stringify(Reflect.ownKeys(row)) !== '["name","tile"]') return false;
                        if (row.name !== name || !(row.tile instanceof __Tile)) return false;
                        if (row.tile.x !== x || row.tile.z !== z || row.tile.level !== level) return false;
                        if (options[index] !== name) return false;
                    }
                    return __catalog() === __catalog();
                };
                "#,
            )
            .unwrap();
        assert!(runtime.eval::<bool>("__check(__expected)").unwrap());
        let state = {
            let scope = runtime.deno_runtime().handle_scope();
            let shared = scope
                .get_slot::<Shared>()
                .cloned()
                .expect("catalog installed");
            let state = shared.borrow();
            (state.row_shape.is_some(), state.values.is_some())
        };
        assert_eq!(
            state,
            (true, true),
            "the shape and catalog are cached after first use"
        );
    });
}

#[test]
fn repeated_reads_return_the_same_bounded_catalog() {
    let mut runtime = setup();
    let baseline = collected_used_heap(&mut runtime);
    let expected: Vec<_> = script::native::walk::walk_destinations().iter().collect();
    runtime
        .eval::<()>("globalThis.__first = __catalog();")
        .unwrap();
    let populated = collected_used_heap(&mut runtime);
    runtime
        .eval::<()>("for (let index = 0; index < 1000; index++) globalThis.__last = __catalog();")
        .unwrap();
    let after_repeat = collected_used_heap(&mut runtime);
    let object_bound = expected.len() * 2 + 3;
    let growth = populated.saturating_sub(baseline);
    let limit = object_bound * 1024;
    println!(
        "Walk destination collected V8 heap: rows={} objects<={object_bound} baseline={baseline} populated={populated} after_repeat={after_repeat} growth={growth} limit={limit}",
        expected.len()
    );
    assert!(
        growth <= limit,
        "walk destinations grew collected V8 heap by {growth} bytes; limit {limit}"
    );
    assert!(
        after_repeat.saturating_sub(populated) <= 16 * 1024,
        "1,000 repeat reads grew collected V8 heap from {populated} to {after_repeat}: catalog is not cached"
    );
    assert!(runtime
        .eval::<bool>("globalThis.__first === globalThis.__last")
        .unwrap());
}
#[test]
fn loaded_declared_surface_materializes_walk_catalog() {
    const TEST_NAME: &str =
        "walk_destinations_v8::tests::loaded_declared_surface_materializes_walk_catalog";
    if std::env::var_os(CHILD_ENV).is_some() {
        crate::platform::bootstrap_test_v8_platform();
        CALLBACK_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
        let isolate = crate::LoadIsolate::spawn(
            "import './declared_surface.js'; export default class T extends LoopingBot { loop() {} }"
                .into(),
            crate::LoadShape::CompatClass,
            vec![],
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let ready = loop {
            match isolate.poll_ready() {
                crate::Ready::Pending if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                ready => break ready,
            }
        };
        let callbacks = CALLBACK_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        isolate.join();
        assert_eq!(ready, crate::Ready::Ready);
        assert_eq!(
            callbacks, 1,
            "the imported declared surface imports WalkDestinations"
        );
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("run isolated declared-surface walk catalog test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated declared-surface walk catalog test failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains(&format!("test {TEST_NAME} ... ok")) && stdout.contains("1 passed"),
        "isolated test did not run exactly once\nstdout: {stdout}\nstderr: {stderr}"
    );
}
