// Task 10: Load isolate (out-of-tree only). `JsLibrary::load(path)` reads a
// JS file, classifies it, validates it compiles in a throwaway Runtime (the
// compile Runtime is dropped before `load()` returns), registers the card
// (same name overwrites; compiled picker names are reserved), and persists
// `{name, path}` to the store. `LoadIsolate::spawn` is the ONLY place a V8
// Runtime is created for a JS card — the panel calls it from Start, never
// from Load. API takes a filesystem path / String source; nothing here
// `include_str!`s a script tree.

use std::path::PathBuf;
use std::sync::mpsc;

use api::interact::Driver;
use api::prot::Out;
use script::ctx::ScriptCtx;
use script::load::{JsLibrary, LoadIsolate, LoadShape};
use script::{CompiledId, SlotScript};

// The brief's native fixture: exported `tick` that counts on `api`.
const NATIVE_TICK: &str = "export function tick(api) { api._n = (api._n||0)+1 }";

// The brief's compat fixture: defineBot config with a `create()` bot.
const COMPAT_FIXTURE: &str =
    r#"export default defineBot({ name: "t", create() { return new (class { loop() {} }) } })"#;

/// Accept-everything driver stub (same shape as the other script tests:
/// the crate's `test_support` module is unit-test-only, so integration
/// tests copy the stub).
#[derive(Default)]
struct NullDriver {
    out: OutSink,
}

impl Driver for NullDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

#[derive(Default)]
struct OutSink;

impl Out for OutSink {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}

/// Unique temp dir per test binary (existing 274bot convention), plus a
/// per-test scratch subdir so parallel tests never share files.
fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-script-load-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(dir: &std::path::Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    path
}

// (1) Loading a native `tick` noop file adds a JS card under the file stem.
#[test]
fn js_library_load_native_tick_file_adds_card() {
    let dir = scratch("adds_card");
    let path = write_file(&dir, "tickbot.js", NATIVE_TICK);
    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));

    let card = lib.load(&path).expect("native tick file loads");
    assert_eq!(card.name, "tickbot");
    assert_eq!(card.path, path);
    assert_eq!(card.shape, LoadShape::NativeTick);
    assert_eq!(card.source, NATIVE_TICK);

    assert_eq!(lib.cards().len(), 1);
    assert_eq!(lib.cards()[0].name, "tickbot");
}

// (2) A second load whose name matches replaces path/source (no duplicate
// picker entry), and the persisted store holds one `{name, path}` record.
#[test]
fn js_library_same_name_replaces_path_and_source() {
    let dir = scratch("same_name");
    let store = dir.join("js-scripts.json");
    let a = write_file(&dir, "t.js", NATIVE_TICK);
    let sub = scratch("same_name_b");
    let b = write_file(&sub, "t.js", "export function tick(api) { api._n = 99 }");
    let mut lib = JsLibrary::new(store.clone());

    lib.load(&a).unwrap();
    assert_eq!(lib.cards().len(), 1);
    assert_eq!(lib.cards()[0].path, a);

    let card = lib.load(&b).unwrap();
    assert_eq!(card.name, "t"); // same stem keeps the picker name
    assert_eq!(card.path, b);
    assert_ne!(card.source, NATIVE_TICK);
    assert_eq!(lib.cards().len(), 1, "same name overwrites, never appends");

    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
    assert_eq!(stored.as_array().unwrap().len(), 1);
    assert_eq!(stored[0]["name"], "t");
    assert_eq!(stored[0]["path"], b.to_string_lossy().to_string());
}

// (3) A file named after a compiled picker id is reserved.
#[test]
fn js_library_reserved_walk_to_errors() {
    let dir = scratch("reserved");
    let path = write_file(&dir, "WalkTo.js", NATIVE_TICK);
    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));

    let err = lib.load(&path).expect_err("WalkTo is a compiled picker id");
    assert!(err.contains("reserved"), "{err}");
    assert!(lib.cards().is_empty());
}

// (4) Non-bot shapes and unreadable files are rejected at Load.
#[test]
fn js_library_rejects_non_bot_shape_and_missing_file() {
    let dir = scratch("rejects");
    let plain = write_file(&dir, "plain.js", "const x = 1 + 1;");
    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));

    let err = lib.load(&plain).expect_err("not a bot shape");
    assert!(err.contains("shape") || err.contains("bot"), "{err}");
    assert!(lib.cards().is_empty());

    let missing = dir.join("nope.js");
    assert!(lib.load(&missing).is_err());
}

// (4b) Restoring from the persisted store re-reads the file and the shape.
#[test]
fn js_library_restore_reads_persisted_paths() {
    let dir = scratch("restore");
    let store = dir.join("js-scripts.json");
    let path = write_file(&dir, "tickbot.js", NATIVE_TICK);
    {
        let mut lib = JsLibrary::new(store.clone());
        lib.load(&path).unwrap();
    }
    let mut lib = JsLibrary::new(store.clone());
    lib.restore().unwrap();
    assert_eq!(lib.cards().len(), 1);
    assert_eq!(lib.cards()[0].name, "tickbot");
    assert_eq!(lib.cards()[0].shape, LoadShape::NativeTick);
    assert_eq!(lib.cards()[0].source, NATIVE_TICK);
}

// (5) The brief's native fixture: spawn ticks the JS through the `api`
// object, `probe` reads `_n` back, `join` stops the thread.
#[test]
fn isolate_spawn_ticks_probe_and_joins() {
    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .expect("spawn native isolate");
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    let n = iso.probe("__rs_api._n").expect("api object readable");
    assert_eq!(n, 3, "three ticks reached the JS tick function");
    iso.join();
}

// (5b) Compat defineBot fixture: the injected shim lets the module load,
// and ticks run `create()`'s `loop()` without throwing.
#[test]
fn isolate_spawn_compat_fixture_ticks_and_joins() {
    let iso = LoadIsolate::spawn(COMPAT_FIXTURE.to_string(), LoadShape::CompatDefineBot)
        .expect("spawn compat isolate");
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let _ = iso.probe("__rs_tick"); // round-trip: both ticks finished first
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("error")),
        "compat loop ran clean: {logs:?}"
    );
    iso.join();
}

// (5c) Pause ignores ticks; resume continues; join returns.
#[test]
fn isolate_pause_ignores_ticks_and_resume_continues() {
    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick).unwrap();
    iso.on_game_tick(1);
    iso.pause();
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    let n = iso.probe("__rs_api._n").unwrap();
    assert_eq!(n, 1, "paused ticks do not run");
    iso.resume();
    iso.on_game_tick(4);
    let n = iso.probe("__rs_api._n").unwrap();
    assert_eq!(n, 2, "resume re-arms tick dispatch");
    iso.join();
}

// (5d) A second spawn after join works (isolate lifecycle is not one-shot).
#[test]
fn isolate_join_returns_and_isolates_are_reusable() {
    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick).unwrap();
    iso.on_game_tick(1);
    iso.join();

    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick).unwrap();
    iso.on_game_tick(2);
    let n = iso.probe("__rs_api._n").unwrap();
    assert_eq!(n, 1, "fresh isolate, fresh api");
    iso.join();
}

// (5e) A throwing tick is logged, not fatal.
#[test]
fn isolate_logs_tick_errors() {
    let src = "export function tick(api) { throw new Error('boom') }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::NativeTick).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_api._n"); // round-trip: the tick finished first
    let logs = iso.drain_logs();
    assert!(
        logs.iter().any(|l| l.contains("boom")),
        "error surfaced: {logs:?}"
    );
    iso.on_game_tick(2); // still alive
    iso.join();
}

// (5f) A runaway tick is interrupted by the budget terminate (armed, never
// cancelled from the host), and the isolate stays usable afterwards.
#[test]
fn slow_tick_is_interrupted_and_isolate_survives() {
    // The first tick spins forever; later ticks count.
    let src =
        "export function tick(api) { api._n = (api._n||0)+1; if (api._n === 1) { while(true){} } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::NativeTick).unwrap();
    iso.on_game_tick(1);
    // Let the thread enter the spin; pause then arms a terminate for the
    // over-budget tick (no immediate cancel), and resume re-arms dispatch.
    std::thread::sleep(std::time::Duration::from_millis(80));
    iso.pause();
    iso.resume();
    // The interrupted tick unwinds on the thread; this tick and the probe
    // round-trip only when the terminate was cleared after the tick's
    // frames unwound (a host-side cancel would race and never interrupt).
    iso.on_game_tick(2);
    let n = iso
        .probe("__rs_api._n")
        .expect("isolate must stay usable after an interrupted tick");
    assert_eq!(n, 2, "the post-interrupt tick reached the JS");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().any(|l| l.contains("interrupted slow tick")),
        "the budget interrupt must be logged: {logs:?}"
    );
    iso.join();
}

// (5g) A tight `while(true){}` tick cannot hang Stop: `join` is bounded
// and returns even if the interrupt were somehow not delivered.
#[test]
fn join_bounds_a_runaway_tick() {
    let iso = LoadIsolate::spawn(
        "export function tick(api) { while(true){} }".to_string(),
        LoadShape::NativeTick,
    )
    .expect("spawn runaway isolate");
    iso.on_game_tick(1);
    std::thread::sleep(std::time::Duration::from_millis(80));
    let t0 = std::time::Instant::now();
    iso.join();
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(10),
        "join must be bounded on a runaway tick"
    );
}

// (6) SlotScript: the isolate is spawned only by the Start helper
// (`start_load`); `stop` joins the thread. Dispatch goes through the normal
// `on_game_tick` pump path.
#[test]
fn slot_start_load_ticks_and_stop_joins() {
    let mut slot = SlotScript::new();
    slot.start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .expect("start_load spawns the isolate");
    assert_eq!(slot.state(), script::RunState::Running);

    let mut driver = NullDriver::default();
    slot.on_game_tick(&mut ScriptCtx {
        driver: &mut driver,
        tick: 7,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
    });
    slot.on_game_tick(&mut ScriptCtx {
        driver: &mut driver,
        tick: 8,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
    });

    slot.pause();
    assert_eq!(slot.state(), script::RunState::Paused);
    slot.resume();
    assert_eq!(slot.state(), script::RunState::Running);

    slot.stop();
    assert_eq!(slot.state(), script::RunState::Idle);
}

// (6b) Start helper refuses while a script is already active.
#[test]
fn slot_start_load_refuses_while_active() {
    let mut slot = SlotScript::new();
    slot.start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .unwrap();
    let err = slot
        .start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .expect_err("already active");
    assert!(err.contains("active"), "{err}");
    slot.stop();
    assert!(slot
        .start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .is_ok());
    slot.stop();
}

// (6c) Compiled and Load are XOR: start_compiled is refused while a load
// isolate runs, and vice versa.
#[test]
fn slot_load_and_compiled_are_xor() {
    struct Noop;
    impl script::Script for Noop {
        fn name(&self) -> &str {
            "noop"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
    }
    let mut slot = SlotScript::new();
    slot.start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .unwrap();
    assert!(slot.start_compiled(Box::new(Noop)).is_err());
    slot.stop();

    slot.start_compiled(Box::new(Noop)).unwrap();
    assert!(slot
        .start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick)
        .is_err());
    slot.stop();
}

// (7) The spawn path is a channel/thread handle, not an in-memory runtime:
// `spawn` returns immediately with a handle, and a fresh spawn does not
// share state with a previous one (already covered above; here we pin the
// channel shape of the public API).
#[test]
fn isolate_spawn_returns_immediately_with_handle() {
    let (tx, rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let t = std::thread::spawn(move || {
        let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick).unwrap();
        tx.send(()).unwrap();
        let _ = done_rx.recv();
        iso.join();
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("spawn returns a handle quickly");
    done_tx.send(()).unwrap();
    t.join().unwrap();
}

// WalkTo stays reserved no matter what the source looks like.
#[test]
fn slot_start_load_reserved_name_via_compiled_ids_is_checked_at_load() {
    let dir = scratch("reserved_at_load");
    let path = write_file(&dir, "WalkTo.js", NATIVE_TICK);
    let mut lib = JsLibrary::new(dir.join("js-scripts.json"));
    assert!(lib.load(&path).is_err());
    assert!(CompiledId("WalkTo").0 == "WalkTo");
}

// Persist uses the default store path naming for the operator file.
#[test]
fn default_js_store_is_dot_274bot_json() {
    let p = script::load::default_js_store();
    let s = p.to_string_lossy().to_string();
    assert!(s.ends_with(".274bot/js-scripts.json"), "{s}");
}
