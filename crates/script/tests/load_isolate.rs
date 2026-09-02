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
use std::thread;
use std::time::{Duration, Instant};

use api::interact::Driver;
use api::prot::Out;
use script::ctx::ScriptCtx;
use script::load::{JsLibrary, LoadIsolate, LoadShape};
use script::{CompiledId, ScriptSource, SlotScript};

// The brief's native fixture: exported `tick` that counts on its own
// global (the `api` object is host-owned: `api.tick` is the only member,
// every other read/set throws `not v1`).
const NATIVE_TICK: &str =
    "export function tick(api) { globalThis.__rs_n = (globalThis.__rs_n || 0) + 1 }";

// The brief's compat fixture: defineBot config with a `create()` bot.
const COMPAT_FIXTURE: &str =
    r#"export default defineBot({ name: "t", create() { return new (class { loop() {} }) } })"#;

// The catalog shape: a TS file with a typed default-export LoopingBot
// subclass. Transpiled at Load and at isolate spawn (types gone).
const CLASS_TS_FIXTURE: &str = r#"
export default class Burier extends LoopingBot {
    override loopDelay = 600;
    private n: number = 0;
    override loop() { this.n += 1; }
}
"#;

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

/// Hermetic library: store + cache under the same scratch dir (never
/// the operator's `~/.274bot`).
fn test_library(dir: &std::path::Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

// Task 9b: the snapshot posted into an isolate is a FlatBuffers blob, not
// a JSON string — a test posting JSON would smuggle the forbidden parse
// path back in. These helpers build the blob through the same encoder the
// host uses (`script::isolate_fb::encode_snapshot`).
fn post_snapshot_input(iso: &LoadIsolate, input: &script::isolate_fb::SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

/// Throw-shaped isolate lines: tick errors, not-v1, Error. Ordinary
/// `this.log` (BoneBurier status) is allowed.
fn is_throw_shaped_log(line: &str) -> bool {
    line.contains("not v1")
        || line.contains("Error")
        || line.contains("interrupted slow tick")
        || (line.starts_with("tick ") && line.contains(':'))
}

/// The empty fail-closed snapshot; tests override the fields they post.
fn base_snapshot<'a>() -> script::isolate_fb::SnapshotInput<'a> {
    script::isolate_fb::SnapshotInput {
        tick: 1,
        here: None,
        ingame: true,
        inv: &[],
        inv_size: 0,
        stats: &[],
        booths: &[],
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
    }
}

// (1) Loading a native `tick` noop file adds a JS card under the file stem.
#[test]
fn js_library_load_native_tick_file_adds_card() {
    let dir = scratch("adds_card");
    let path = write_file(&dir, "tickbot.js", NATIVE_TICK);
    let mut lib = test_library(&dir);

    let card = lib.load(&path).expect("native tick file loads");
    assert_eq!(card.name, "tickbot");
    assert_eq!(card.path, path);
    assert_eq!(card.shape, LoadShape::NativeTick);
    assert_eq!(card.origin, NATIVE_TICK);

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
    let b = write_file(
        &sub,
        "t.js",
        "export function tick(api) { globalThis.__rs_n = 99 }",
    );
    let mut lib = test_library(&dir);

    lib.load(&a).unwrap();
    assert_eq!(lib.cards().len(), 1);
    assert_eq!(lib.cards()[0].path, a);

    let card = lib.load(&b).unwrap();
    assert_eq!(card.name, "t"); // same stem keeps the picker name
    assert_eq!(card.path, b);
    assert_ne!(card.origin, NATIVE_TICK);
    assert_eq!(lib.cards().len(), 1, "same name overwrites, never appends");

    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
    assert_eq!(stored.as_array().unwrap().len(), 1);
    assert_eq!(stored[0]["name"], "t");
    assert_eq!(stored[0]["path"], b.to_string_lossy().to_string());
}

// (3) WalkTo is the only reserved picker id: it is host nav, never a JS
// card.
#[test]
fn js_library_reserved_walk_to_errors() {
    let dir = scratch("reserved");
    let path = write_file(&dir, "WalkTo.js", NATIVE_TICK);
    let mut lib = test_library(&dir);

    let err = lib.load(&path).expect_err("WalkTo is reserved at Load");
    assert!(err.contains("reserved"), "{err}");
    assert!(lib.cards().is_empty());
}

// (3b) Abandoned compiled smoke names are no longer reserved: a file named
// BoneBurier.ts loads Ok under its stem.
#[test]
fn js_library_load_bone_burier_named_file_is_ok() {
    let dir = scratch("bone_burier");
    let path = write_file(&dir, "BoneBurier.ts", CLASS_TS_FIXTURE);
    let mut lib = test_library(&dir);

    let card = lib.load(&path).expect("BoneBurier.ts is not reserved");
    assert_eq!(card.name, "BoneBurier");
    assert_eq!(card.shape, LoadShape::CompatClass);
    assert_eq!(lib.cards().len(), 1);

    let native = write_file(&dir, "BoneBurier.js", NATIVE_TICK);
    let card = lib.load(&native).expect("BoneBurier.js is not reserved");
    assert_eq!(card.name, "BoneBurier");
    assert_eq!(card.shape, LoadShape::NativeTick);
    assert_eq!(lib.cards().len(), 1, "same name replaces the card");
}

// (4) Non-bot shapes and unreadable files are rejected at Load.
#[test]
fn js_library_rejects_non_bot_shape_and_missing_file() {
    let dir = scratch("rejects");
    let plain = write_file(&dir, "plain.js", "const x = 1 + 1;");
    let mut lib = test_library(&dir);

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
        let mut lib = test_library(&dir);
        lib.load(&path).unwrap();
    }
    let mut lib = test_library(&dir);
    lib.restore().unwrap();
    assert_eq!(lib.cards().len(), 1);
    assert_eq!(lib.cards()[0].name, "tickbot");
    assert_eq!(lib.cards()[0].shape, LoadShape::NativeTick);
    assert_eq!(lib.cards()[0].source, ScriptSource::File);
    assert_eq!(lib.cards()[0].origin, NATIVE_TICK);
}

// (5) The brief's native fixture: spawn ticks the JS through the `api`
// object, `probe` reads `_n` back, `join` stops the thread.
#[test]
fn isolate_spawn_ticks_probe_and_joins() {
    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
        .expect("spawn native isolate");
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    let n = iso.probe("__rs_n").expect("native counter readable");
    assert_eq!(n, 3, "three ticks reached the JS tick function");
    iso.join();
}

// (5b) Compat defineBot fixture: the injected shim lets the module load,
// and ticks run `create()`'s `loop()` without throwing.
#[test]
fn isolate_spawn_compat_fixture_ticks_and_joins() {
    let iso = LoadIsolate::spawn(COMPAT_FIXTURE.to_string(), LoadShape::CompatDefineBot, vec![])
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
    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.pause();
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    let n = iso.probe("__rs_n").unwrap();
    assert_eq!(n, 1, "paused ticks do not run");
    iso.resume();
    iso.on_game_tick(4);
    let n = iso.probe("__rs_n").unwrap();
    assert_eq!(n, 2, "resume re-arms tick dispatch");
    iso.join();
}

// (5d) A second spawn after join works (isolate lifecycle is not one-shot).
#[test]
fn isolate_join_returns_and_isolates_are_reusable() {
    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.join();

    let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(2);
    let n = iso.probe("__rs_n").unwrap();
    assert_eq!(n, 1, "fresh isolate, fresh api");
    iso.join();
}

// `this.log` lands on the isolate log (host handle `log[]` forwarded
// after the tick) so BOT_DEBUG / the panel can see script-side lines.
#[test]
fn isolate_forwards_this_log() {
    let src = "export default class T extends LoopingBot { loop() { this.log('bury ok'); } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().any(|l| l.contains("bury ok")),
        "this.log must reach drain_logs: {logs:?}"
    );
    iso.join();
}

// Task 12 fix 3 — `LoopingBot.on` stores IPC subscriptions; `chat.message`
// fires when posted `chat_text` changes. Unknown names subscribe and never
// fire; `on` itself does not throw (Thiever/ChickenKiller add ContinueDialog
// after `this.on`).
#[test]
fn isolate_looping_bot_on_does_not_throw_and_taskbot_still_adds() {
    let src = r#"
export default class T extends TaskBot {
    onStart() {
        this.on('chat.message', () => {});
        this.on('skill.xp', () => {});
        this.add({ validate() { return false; }, execute() {} });
        globalThis.__added = this._tasks.length;
    }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let added = iso.probe("__added").unwrap();
    assert_eq!(added, 1, "this.on must not throw before this.add");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !is_throw_shaped_log(l)),
        "this.on must not throw: {logs:?}"
    );
    iso.join();
}

#[test]
fn isolate_looping_bot_on_fires_chat_message_when_chat_text_changes() {
    let src = r#"
export default class T extends LoopingBot {
    onStart() {
        this.on('chat.message', (e) => { globalThis.__chat = e.text; });
    }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.chat_text = Some("You have been stunned.");
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1");
    let text = iso.probe("__chat").unwrap();
    assert_eq!(
        text.as_str(),
        Some("You have been stunned."),
        "chat.message fires with posted chat_text"
    );
    iso.join();
}

// (5e) A throwing tick is logged, not fatal.
#[test]
fn isolate_logs_tick_errors() {
    let src = "export function tick(api) { throw new Error('boom') }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_n"); // round-trip: the tick finished first
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
    let src = "export function tick(api) { globalThis.__rs_n = (globalThis.__rs_n||0)+1; if (globalThis.__rs_n === 1) { while(true){} } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::NativeTick, vec![]).unwrap();
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
        .probe("__rs_n")
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
    let iso = LoadIsolate::spawn("export function tick(api) { while(true){} }".to_string(), LoadShape::NativeTick, vec![])
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
    slot.start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
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
    slot.start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
        .unwrap();
    let err = slot
        .start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
        .expect_err("already active");
    assert!(err.contains("active"), "{err}");
    slot.stop();
    assert!(slot
        .start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
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
    slot.start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
        .unwrap();
    assert!(slot.start_compiled(Box::new(Noop)).is_err());
    slot.stop();

    slot.start_compiled(Box::new(Noop)).unwrap();
    assert!(slot
        .start_load(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![])
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
        let iso = LoadIsolate::spawn(NATIVE_TICK.to_string(), LoadShape::NativeTick, vec![]).unwrap();
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
    let mut lib = test_library(&dir);
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

// (8) The catalog shape loads: a TS file with a typed default-export
// `LoopingBot` subclass is `CompatClass`, transpiles at Load (types gone)
// and validates in the throwaway compile Runtime.
#[test]
fn js_library_loads_compat_class_ts() {
    let dir = scratch("compat_class");
    let path = write_file(&dir, "Burier.ts", CLASS_TS_FIXTURE);
    let mut lib = test_library(&dir);

    let card = lib.load(&path).expect("compat class TS loads");
    assert_eq!(card.shape, LoadShape::CompatClass);
    assert_eq!(card.name, "Burier");
    assert_eq!(lib.cards().len(), 1);
}

// (8b) `transpile_ts` strips TS-only syntax so V8 can parse the output.
#[test]
fn transpile_ts_strips_types_and_v8_can_parse_it() {
    let js = script::load::transpile_ts(
        "export default class X extends LoopingBot { private n: number = 0 }",
    )
    .expect("transpiles");
    assert!(!js.contains(": number"), "type annotation gone: {js}");
    assert!(!js.contains("private"), "private marker gone: {js}");
}

// (8c) The catalog shape spawns: the isolate instantiated the default
// export and `loop()` runs on ticks (the instance is probed back).
#[test]
fn isolate_spawn_compat_class_ticks_and_joins() {
    let js = script::transpile_ts(CLASS_TS_FIXTURE).expect("transpile class fixture");
    let iso = LoadIsolate::spawn(js, LoadShape::CompatClass, vec![]).expect("spawn compat class isolate");
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let n = iso.probe("__rs_bot.n").expect("instance readable");
    assert_eq!(n, 2, "two ticks reached the class loop");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("error")),
        "class loop ran clean: {logs:?}"
    );
    iso.join();
}

// Task 3 — import remap: the relative `../../api/...` imports resolve to
// our Game module (not the rs2b0t tree); ticks run clean, and
// `Game.teleport` throws `not v1` (logged, never fatal).
#[test]
fn isolate_remaps_api_imports_to_our_game_and_teleport_throws() {
    let src = "import { Game } from '../../api/game/Game.js'; export default class T extends LoopingBot { loop() { Game.ingame(); } }";
    let iso =
        LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).expect("remapped import loads");
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let _ = iso.probe("__rs_bot"); // round-trip: both ticks finished first
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("error")),
        "remapped ticks ran clean: {logs:?}"
    );
    iso.join();

    // Game.teleport is a real member that throws at runtime.
    let src = "import { Game } from '../../api/game/Game.js'; export default class T extends LoopingBot { loop() { Game.teleport('Lumbridge'); } }";
    let iso =
        LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).expect("teleport bot loads");
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_bot");
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("not v1") && l.contains("Game.teleport")),
        "teleport throws not v1: {logs:?}"
    );
    iso.join();
}

// Task 3 — the `@rs2b0t/api` bare specifier remaps to the same shim.
#[test]
fn isolate_rs2b0t_api_bare_import_resolves_to_our_shim() {
    let src = "import { Game } from '@rs2b0t/api'; export default class T extends LoopingBot { loop() { Game.ingame(); } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![])
        .expect("bare @rs2b0t/api import loads");
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_bot");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("error")),
        "bare @rs2b0t/api ticks ran clean: {logs:?}"
    );
    iso.join();
}

// Task 3 — Paint.begin(title/row/gap/end) records a ScriptPaint on the
// host handle (no canvas); the widget methods throw `not v1`.
#[test]
fn isolate_paint_begin_records_script_paint() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    loop() {
        const p = Paint.begin(null, { accent: '#f3e6a2' });
        p.title('BoneBurier — digging');
        p.row('Runtime: 1.2m', 'Buried: 3');
        p.gap();
        p.end();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).expect("paint bot loads");
    iso.on_game_tick(1);
    let value = iso
        .probe("__rs2b0t_host.paint")
        .expect("paint record readable");
    let paint: script::shim::ScriptPaint =
        serde_json::from_value(value).expect("decodes as ScriptPaint");
    assert_eq!(paint.title.as_deref(), Some("BoneBurier — digging"));
    assert_eq!(paint.accent.as_deref(), Some("#f3e6a2"));
    assert_eq!(paint.lines, vec!["Runtime: 1.2m | Buried: 3", ""]);
    iso.join();
}

// Task 13 — the host-side paint accessor returns the forwarded frame
// without a probe round-trip (the isolate thread sends it after the tick).
#[test]
fn isolate_paint_accessor_returns_the_recorded_frame() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    loop() {
        const p = Paint.begin(null, { accent: '#f3e6a2' });
        p.title('BoneBurier — digging');
        p.row('Runtime: 1.2m', 'Buried: 3');
        p.end();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).expect("paint bot loads");
    iso.on_game_tick(1);
    // The probe round-trips after the tick, so the forwarded Paint
    // message is in the channel by the time it returns.
    let _ = iso.probe("0");
    let paint = iso.paint().expect("paint forwarded");
    assert_eq!(paint.title.as_deref(), Some("BoneBurier — digging"));
    assert_eq!(paint.accent.as_deref(), Some("#f3e6a2"));
    assert_eq!(paint.lines, vec!["Runtime: 1.2m | Buried: 3"]);
    iso.join();
}

// Task 3 — reader.worldTile / inventorySize and actions.closeModal are
// thin stubs (no snapshot this tag: null / 0 / false); missing members
// throw `not v1`.
#[test]
fn isolate_reader_and_actions_expose_thin_stubs_and_throw_on_missing() {
    let src = r#"
import { reader, actions } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            tile: reader.worldTile(),
            inv: reader.inventorySize(),
            closed: actions.closeModal(),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("stub reads back");
    assert_eq!(value["tile"], serde_json::Value::Null, "no snapshot tile");
    assert_eq!(value["inv"], 0, "no snapshot inventory");
    assert_eq!(value["closed"], false, "no open modal");
    iso.join();

    let src = "import { reader } from '../../adapter/ClientAdapter.js'; export default class T extends LoopingBot { loop() { reader.bankOpen(); } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_bot");
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("not v1") && l.contains("reader.bankOpen")),
        "missing reader member throws: {logs:?}"
    );
    iso.join();
}

// Task 3 — ScriptRunner.stop stops the isolate: the stop flag breaks the
// tick loop on the isolate thread (like IsolateCmd::Stop), the Runtime is
// dropped, and the host sees a dead isolate.
#[test]
fn isolate_script_runner_stop_stops_the_isolate() {
    let src = r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__rs_n = (globalThis.__rs_n || 0) + 1;
        ScriptRunner.stop('done');
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    // The flag is read inside the tick; the thread logs and breaks. Poll
    // for the log (a probe round-trip would race the thread exit).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let logs = loop {
        let logs = iso.drain_logs();
        if logs.iter().any(|l| l.contains("stop")) || std::time::Instant::now() > deadline {
            break logs;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(
        logs.iter().any(|l| l.contains("stop")),
        "the stop hook must be logged: {logs:?}"
    );
    // The isolate stopped itself: it no longer answers probes (the thread
    // exited, so the channel is closed). A live Runtime would keep
    // answering — this is the regression the fix guards.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if iso.probe("1 + 1").is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the isolate must stop answering probes after ScriptRunner.stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    iso.join();
}

// Task 3 — TaskBot.loop runs the first task whose validate() passes (the
// rs2b0t priority order), via onStart's this.add(...).
#[test]
fn isolate_task_bot_loop_runs_first_passing_validate() {
    let src = r#"
import { TaskBot } from '../../api/bot/Bot.js';
export default class T extends TaskBot {
    onStart() {
        this.runs = [];
        this.add(
            { validate: () => false, execute: () => this.runs.push('first') },
            { validate: () => true, execute: () => this.runs.push('second') },
        );
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let runs = iso.probe("__rs_bot.runs").expect("task runs readable");
    assert_eq!(
        runs,
        serde_json::json!(["second", "second"]),
        "only the passing task executes, once per tick"
    );
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("error")),
        "task loop ran clean: {logs:?}"
    );
    iso.join();
}

// Task 4 — Execution.delayUntil parks the loop: `loop()` awaits a cond on
// `Game.tick()`; posted ticks pump the wait (the isolate does NOT call
// `loop()` again while parked); the third `loop` runs only after the cond
// cleared.
#[test]
fn isolate_execution_delay_until_parks_loop_until_cond() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
        if (globalThis.__rs_loops === 1) {
            globalThis.__rs_ok = null;
            globalThis.__rs_ok = await Execution.delayUntil(() => Game.tick() >= 3, 6000);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(
        loops, 1,
        "parked: loop must not re-enter while the wait is active"
    );
    let ok = iso.probe("__rs_ok").unwrap();
    assert_eq!(ok, serde_json::Value::Null, "delayUntil not settled yet");
    iso.on_game_tick(3); // cond true: the wait clears, loop 1 finishes
    let ok = iso.probe("__rs_ok").unwrap();
    assert_eq!(ok, true, "delayUntil resolved true on the posted tick");
    iso.on_game_tick(4); // not parked any more: the third loop runs
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(loops, 2, "loop re-enters after the park cleared");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("error")),
        "delayUntil ran clean: {logs:?}"
    );
    iso.join();
}

// Task 4 — guardian hold freezes loop() AND the parked cond: ticks posted
// while `__rs2b0t_host.hold` is set neither run loop() nor settle the
// wait; after the hold lifts, the pump resumes and the next loop runs.
#[test]
fn isolate_hold_freezes_loop_and_parked_conds() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
        if (globalThis.__rs_loops === 1) {
            await Execution.delayUntil(() => Game.tick() >= 3, 6000);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_loops").unwrap(), 1, "first loop parked");
    let mut snap = base_snapshot();
    snap.hold = true;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    iso.on_game_tick(4);
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(loops, 1, "hold freezes loop: count does not increase");
    snap.hold = false;
    snap.tick = 5;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(5); // cond true now (tick 5 >= 3): loop 1 finishes
    iso.on_game_tick(6); // loop 2 runs
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(loops, 2, "loop resumes after the hold lifts");
    iso.join();
}

// Fix round — Guardian hold from a posted FlatBuffer freezes loop() but
// still calls onPaint. Do NOT poke `__rs2b0t_host.hold`; the blob's
// `hold: true` must mirror onto the host flag and the tick must paint.
#[test]
fn isolate_hold_from_blob_freezes_loop_but_still_paints() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
    }
    onPaint() {
        globalThis.__rs_paints = (globalThis.__rs_paints || 0) + 1;
        const p = Paint.begin();
        p.title('held');
        p.row('status');
        p.end();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_loops").unwrap(), 1, "first loop runs");
    let paints_after_first = iso.probe("__rs_paints").unwrap();
    assert_eq!(paints_after_first, 1, "first tick paints");
    // Post hold via the FlatBuffer — never poke `__rs2b0t_host.hold`.
    let mut snap = base_snapshot();
    snap.hold = true;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(loops, 1, "posted hold freezes loop: count does not increase");
    let paints: i64 = iso.probe("__rs_paints").unwrap().as_i64().unwrap();
    assert!(
        paints >= 3,
        "onPaint still runs while held (paints={paints})"
    );
    let frame = iso.paint().expect("paint forwarded while held");
    assert_eq!(frame.title.as_deref(), Some("held"));
    assert!(frame.lines.iter().any(|l| l == "status"));
    iso.join();
}

// SEC-004 — while the posted blob has hold:true, JS cannot unfreeze by
// writing __rs2b0t_host.hold = false: loop() stays frozen; onPaint may run.
#[test]
fn isolate_js_cannot_clear_host_hold() {
    let src = r#"
export default class T extends LoopingBot {
    loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
    }
    onPaint() {
        globalThis.__rs_paints = (globalThis.__rs_paints || 0) + 1;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.hold = true;
    snap.tick = 1;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("globalThis.__rs_loops || 0").unwrap(),
        0,
        "held first tick must not run loop()"
    );
    let paints: i64 = iso.probe("__rs_paints").unwrap().as_i64().unwrap();
    assert_eq!(paints, 1, "onPaint runs while held");
    // JS tries to clear hold; the blob still has hold:true (unchanged delta).
    iso.probe("__rs2b0t_host.hold = false").unwrap();
    assert_eq!(
        iso.probe("__rs2b0t_host.hold").unwrap(),
        false,
        "JS assignment landed (the exploit under test)"
    );
    iso.on_game_tick(2);
    iso.on_game_tick(3);
    assert_eq!(
        iso.probe("globalThis.__rs_loops || 0").unwrap(),
        0,
        "loop() must not run after JS cleared hold while blob holds"
    );
    let paints: i64 = iso.probe("__rs_paints").unwrap().as_i64().unwrap();
    assert!(
        paints >= 3,
        "onPaint still runs while blob hold is true (paints={paints})"
    );
    iso.join();
}

// Task 4 — Execution.delayTicks(n) parks for n posted ticks (dueTick =
// current + n), then the loop continues and re-enters on the next tick.
#[test]
fn isolate_execution_delay_ticks_parks_n_ticks() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
        if (globalThis.__rs_loops === 1) {
            await Execution.delayTicks(2);
            globalThis.__rs_done = true;
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let done = iso.probe("__rs_done");
    assert!(
        done.is_err(),
        "two posted ticks are not enough for delayTicks(2)"
    );
    iso.on_game_tick(3); // dueTick reached: the wait clears
    let done = iso.probe("__rs_done").unwrap();
    assert_eq!(done, true, "delayTicks(2) settled after two posted ticks");
    iso.on_game_tick(4);
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(loops, 2, "loop re-enters after delayTicks");
    iso.join();
}

// Task 4 — Execution.delay(ms) parks on wall-clock time (isolate time):
// after the wait elapses the next posted tick settles it.
#[test]
fn isolate_execution_delay_parks_on_wall_clock() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
        if (globalThis.__rs_loops === 1) {
            await Execution.delay(100);
            globalThis.__rs_done = true;
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    std::thread::sleep(std::time::Duration::from_millis(140));
    iso.on_game_tick(2); // wall clock elapsed: the wait settles
    let done = iso.probe("__rs_done").unwrap();
    assert_eq!(
        done, true,
        "delay(100) settled after the wall clock elapsed"
    );
    iso.on_game_tick(3);
    let loops = iso.probe("__rs_loops").unwrap();
    assert_eq!(loops, 2, "loop re-enters after delay");
    iso.join();
}

// Task 4 — delayUntil with a never-true cond resolves false on timeout.
#[test]
fn isolate_execution_delay_until_times_out_false() {
    let src = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1;
        if (globalThis.__rs_loops === 1) {
            globalThis.__rs_ok = null;
            globalThis.__rs_ok = await Execution.delayUntil(() => false, 100);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    std::thread::sleep(std::time::Duration::from_millis(140));
    iso.on_game_tick(2); // timeout elapsed: the wait resolves false
    let ok = iso.probe("__rs_ok").unwrap();
    assert_eq!(ok, false, "delayUntil times out to false");
    iso.join();
}

// Task 9b — a posted FlatBuffer snapshot is what Game/Inventory/Skills/
// EventSignal read: `Inventory.count` sums the inv rows by name
// (case-insensitive), `EventSignal.pending()` is hold OR ours as posted,
// `Skills.xp`/`index` scan the stats rows, and `Game.tile()`/`ingame()`
// read the posted `here`/`ingame`. Only the fields the host posts — no
// World clone. The blob is FlatBuffers (never a JSON string).
#[test]
fn isolate_reads_posted_fb_snapshot_blob() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Skills } from '../../api/skills/Skills.js';
import { EventSignal } from '../../api/execution/EventSignal.js';
export default class T extends LoopingBot {
    capture() {
        globalThis.__probe = {
            bones: Inventory.count('Bones'),
            drag: Inventory.count('Dragon bones'),
            pending: EventSignal.pending(),
            ignored: EventSignal.ignoredRandoms(),
            xp: Skills.xp('prayer'),
            idx: Skills.index('prayer'),
            tile: Game.tile(),
            ingame: Game.ingame(),
        };
    }
    loop() { this.capture(); }
    // Guardian hold still calls onPaint — keep the probe fresh while held.
    onPaint() { this.capture(); }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.tick = 1;
    snap.here = Some(script::isolate_fb::TileInput {
        x: 3200,
        z: 3200,
        level: 0,
    });
    snap.ingame = true;
    snap.inv = &[(Some("Bones"), 2)];
    snap.stats = &[script::isolate_fb::StatInput {
        index: 5,
        name: "prayer",
        xp: 1300,
        level: 10,
    }];
    snap.hold = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("posted snapshot reads back");
    assert_eq!(
        value["bones"], 2,
        "Inventory.count('Bones') sums the posted inv row"
    );
    assert_eq!(value["drag"], 0, "a name never posted fails closed to 0");
    assert_eq!(
        value["pending"], true,
        "EventSignal.pending() is hold as posted"
    );
    assert_eq!(
        value["ignored"],
        serde_json::json!([]),
        "no posted ignore list defaults to []"
    );
    assert_eq!(value["xp"], 1300, "Skills.xp reads the posted stats row");
    assert_eq!(
        value["idx"], 0,
        "Skills.index finds the posted stat by name"
    );
    assert_eq!(
        value["tile"]["x"], 3200,
        "Game.tile() reads the posted here tile"
    );
    assert_eq!(value["ingame"], true, "Game.ingame() reads the posted flag");
    iso.join();
}

// OPT-007: the isolate thread forwards ignoredRandoms after each tick;
// the host reads the cache without a probe recv_timeout round-trip.
#[test]
fn ignored_randoms_cache_fills_without_probe() {
    let src = r#"
export default class T extends LoopingBot {
    ignoredRandoms() { return ['swarm']; }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let deadline = Instant::now() + Duration::from_millis(500);
    let list = loop {
        let list = iso.ignored_randoms();
        if list == ["swarm"] {
            break list;
        }
        if Instant::now() >= deadline {
            panic!(
                "ignored_randoms cache expected ['swarm'], got {list:?} (probe path?)"
            );
        }
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(list, vec!["swarm".to_string()]);
    // Park the isolate on tick 2; cache read must not probe.
    let src_block = r#"
export default class T extends LoopingBot {
    ignoredRandoms() { return ['swarm']; }
    loop() { if (globalThis.__rs_tick >= 2) { while (true) {} } }
}
"#;
    let iso_block = LoadIsolate::spawn(src_block.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso_block.on_game_tick(1);
    thread::sleep(Duration::from_millis(50));
    iso_block.on_game_tick(2);
    thread::sleep(Duration::from_millis(20));
    let block_list = iso_block.ignored_randoms();
    assert_eq!(
        block_list,
        vec!["swarm".to_string()],
        "cached list readable while a later tick is in flight"
    );
    iso.join();
    iso_block.join();
}

// Task 12 — `EventSignal.ignoredRandoms()` reads the bot instance (the
// rs2b0t `setIgnoredRandoms` source): the class shape's `__rs_bot` and
// the defineBot shape's created instance. The host reads the same list
// through the knock path to skip act on those names.
#[test]
fn event_signal_ignored_randoms_reads_the_bot_instance() {
    let src = r#"
import { EventSignal } from '../../api/execution/EventSignal.js';
export default class T extends LoopingBot {
    ignoredRandoms() { return ['swarm', 'rock golem']; }
    loop() {
        globalThis.__probe = EventSignal.ignoredRandoms();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso
        .probe("__probe")
        .expect("instance ignore list reads back");
    assert_eq!(
        value,
        serde_json::json!(["swarm", "rock golem"]),
        "EventSignal.ignoredRandoms() is the bot instance's method"
    );
    iso.join();
}

#[test]
fn event_signal_ignored_randoms_reads_a_definebot_instance() {
    let src = r#"
import { EventSignal } from '../../api/execution/EventSignal.js';
export default defineBot({
    name: 'ignorer',
    create() {
        return {
            ignoredRandoms: () => ['maze'],
            loop() { globalThis.__probe = EventSignal.ignoredRandoms(); },
        };
    },
});
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatDefineBot, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("defineBot instance reads back");
    assert_eq!(
        value,
        serde_json::json!(["maze"]),
        "the defineBot-created instance is the ignoredRandoms source"
    );
    iso.join();
}

// Task 9c — snapshot deltas: host-play posts `tick` every post and only
// the fields that changed vs the last post (keyframe on Start). The
// isolate merges the delta onto the last JS snapshot object — an omitted
// table keeps its prior value, it never clears to empty. The first post
// (keyframe) carries every field.
#[test]
fn isolate_snapshot_delta_merges_onto_last_js_snapshot() {
    let src = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
import { EventSignal } from '../../api/execution/EventSignal.js';
export default class T extends LoopingBot {
    capture() {
        globalThis.__probe = {
            bones: Inventory.count('Bones'),
            pending: EventSignal.pending(),
        };
    }
    loop() { this.capture(); }
    // Guardian hold still calls onPaint — pending flips while held.
    onPaint() { this.capture(); }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();

    // First post is the keyframe: the inv table is present.
    let mut snap = base_snapshot();
    snap.inv = &[(Some("Bones"), 2)];
    let (keyframe, fp1) = script::isolate_fb::encode_snapshot_delta(None, &snap, false);
    let kf = script::isolate_fb::decode_snapshot(&keyframe).expect("keyframe decodes");
    assert!(kf.has_inv(), "keyframe carries the inv table");
    iso.post_snapshot(keyframe);
    iso.on_game_tick(1);

    // Second post: inv unchanged -> the buffer has no inv table, and the
    // isolate keeps the last JS rows (Inventory.count still 2).
    let (delta, fp2) = script::isolate_fb::encode_snapshot_delta(Some(&fp1), &snap, false);
    let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
    assert!(!view.has_inv(), "second buffer has no inv table");
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(
        value["bones"], 2,
        "Inventory.count still 2 after an inv-less delta"
    );

    // Third post: inv count 1 -> the inv table comes back and count reads 1.
    snap.inv = &[(Some("Bones"), 1)];
    let (delta, fp3) = script::isolate_fb::encode_snapshot_delta(Some(&fp2), &snap, false);
    assert!(
        script::isolate_fb::decode_snapshot(&delta)
            .unwrap()
            .has_inv(),
        "changed inv is carried"
    );
    iso.post_snapshot(delta);
    iso.on_game_tick(3);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["bones"], 1, "Inventory.count reads the posted 1");

    // Hold flip without an inv change -> hold present, inv still omitted,
    // EventSignal.pending() flips true while the inv rows are untouched.
    snap.hold = true;
    let (delta, _fp4) = script::isolate_fb::encode_snapshot_delta(Some(&fp3), &snap, false);
    let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
    assert!(!view.has_inv(), "inv omitted by the hold-only delta");
    assert!(view.hold(), "hold flip is carried");
    iso.post_snapshot(delta);
    iso.on_game_tick(4);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["pending"], true, "hold flip reads pending true");
    assert_eq!(value["bones"], 1, "inv untouched by the hold-only delta");
    iso.join();
}

// Task 9b — EventSignal.pending() is hold OR ours, as the host posted them
// (the FlatBuffer blob carries both flags).
#[test]
fn isolate_event_signal_pending_tracks_hold_or_ours() {
    let src = r#"
import { EventSignal } from '../../api/execution/EventSignal.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = EventSignal.pending();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.ours = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value, true, "ours alone makes pending() true");
    iso.join();
}

// Task 5 — without a posted snapshot every read fails closed: count 0,
// pending false, xp 0, index -1. Missing members throw `not v1` — never a
// fake value.
#[test]
fn isolate_snapshot_reads_fail_closed_without_a_post() {
    let src = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
import { Skills } from '../../api/skills/Skills.js';
import { EventSignal } from '../../api/execution/EventSignal.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            bones: Inventory.count('Bones'),
            pending: EventSignal.pending(),
            xp: Skills.xp('prayer'),
            idx: Skills.index('prayer'),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["bones"], 0, "no posted snapshot: count 0");
    assert_eq!(value["pending"], false, "no posted snapshot: not pending");
    assert_eq!(value["xp"], 0, "no posted snapshot: xp 0");
    assert_eq!(value["idx"], -1, "no posted snapshot: index -1");
    iso.join();

    let src = "import { Inventory } from '../../api/inventory/Inventory.js'; export default class T extends LoopingBot { loop() { globalThis.__probe = Inventory.first('Bones'); } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert!(
        value.is_null(),
        "no posted snapshot: first fails closed to null"
    );
    iso.join();
}

// The live BoneBurier path: `Inventory.first` returns the held item and
// its `interact('Bury')` queues a held-item request for the host to
// dispatch — never a throw.
#[test]
fn isolate_inventory_first_queues_a_held_interact() {
    let src = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    loop() {
        const bones = Inventory.first('Bones');
        globalThis.__probe = bones ? bones.interact('Bury') : 'none';
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.inv = &[(Some("Bones"), 5)];
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value, true, "interact accepts the queue");
    let reqs = iso.drain_interacts();
    assert_eq!(
        reqs,
        vec![script::shim::InteractReq::Held {
            name: "Bones".into(),
            action: "Bury".into()
        }],
        "first().interact('Bury') queues the held op"
    );
    iso.join();
}

// The BoneBurier onStart gate: `reader.inventorySize()` mirrors the inv
// tab slot count the host posts (0 while tutorial-locked, 28 once bound).
#[test]
fn isolate_reader_inventory_size_mirrors_the_posted_tab_slot_count() {
    let src = r#"
import { reader } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = reader.inventorySize();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.inv_size = 28;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(
        value, 28,
        "reader.inventorySize() reads the posted slot count"
    );
    iso.join();
}

// The live BoneBurier gold probe: when `$RS2B0T` points at a real rs2b0t
// checkout, load the actual BoneBurier card and drive it against a
// seeded snapshot (Bones in the inv, inv tab bound, Prayer stats). The
// script's onStart must settle and its loop must queue a held Bury —
// the exact shim path the live `script_bone_burier` scenario runs.
#[test]
fn real_bone_burier_queues_bury_when_seeded() {
    let Some(root) = script::rs2b0t_root() else {
        eprintln!("skip: $RS2B0T not set");
        return;
    };
    let path = root.join("src/bot/scripts/BoneBurier/BoneBurier.ts");
    let Ok(source) = std::fs::read_to_string(&path) else {
        eprintln!("skip: no BoneBurier.ts at {path:?}");
        return;
    };
    if !source.contains("Bury") {
        eprintln!("skip: BoneBurier.ts is not a burier implementation");
        return;
    }
    let shape = script::detect_shape(&source);
    assert_eq!(
        shape,
        script::LoadShape::CompatClass,
        "BoneBurier is a class card"
    );
    let js = script::transpile_ts(&source).expect("transpile BoneBurier.ts");
    let iso = LoadIsolate::spawn(js, shape, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.ingame = true;
    snap.here = Some(script::isolate_fb::TileInput {
        x: 3220,
        z: 3220,
        level: 0,
    });
    let inv = [(Some("Bones"), 5)];
    snap.inv = &inv;
    snap.inv_size = 28;
    let stats = [script::isolate_fb::StatInput {
        index: 5,
        name: "Prayer",
        xp: 31,
        level: 1,
    }];
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    for n in 1..=8 {
        iso.on_game_tick(n);
    }
    // Sync barrier: the tick commands are fire-and-forget; the probe
    // round-trips so the thread has processed them before the drains.
    let _ = iso.probe("1 + 1");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !is_throw_shaped_log(l)),
        "the real BoneBurier must not throw on a seeded snapshot: {logs:?}"
    );
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter().any(|r| matches!(
            r,
            script::shim::InteractReq::Held { name, action }
                if name == "Bones" && action == "Bury"
        )),
        "the script must queue a held Bury, got {reqs:?}"
    );
    iso.join();
}

// Task 3 — the native tick `api` is a Proxy: `api.tick` is set by the
// host and readable; every other member read or set throws `not v1`.
#[test]
fn isolate_native_tick_api_is_throw_on_missing_proxy() {
    let src = "export function tick(api) { globalThis.__rs_n = (globalThis.__rs_n || 0) + 1 }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let n = iso.probe("__rs_n").unwrap();
    assert_eq!(n, 2, "two ticks reached the JS tick function");
    let tick = iso.probe("__rs_api.tick").expect("api.tick is set");
    assert_eq!(tick, 2, "api.tick holds the dispatched tick number");
    assert!(
        iso.probe("__rs_api._n").is_err(),
        "reading an unknown api member throws"
    );
    assert!(
        iso.probe("__rs_api.missing = 1").is_err(),
        "setting an unknown api member throws"
    );
    iso.join();
}

// Task 7 — Banking shim: `Banking.open()` / `Bank.openNearest` /
// `Bank.openBooth` are thin names onto the host's nearest Use-quickly
// interact. No radius-1 JS router, no packed-stand walk.
#[test]
fn isolate_banking_open_queues_open_booth_without_walk() {
    let src = r#"
import { Banking } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__rs_ok = await Banking.open();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(script::isolate_fb::TileInput {
        x: 100,
        z: 100,
        level: 0,
    });
    snap.booths = &[script::isolate_fb::TileInput {
        x: 200,
        z: 100,
        level: 0,
    }];
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1"); // round-trip: the tick finished first
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::OpenBooth {
            x: 0,
            z: 0,
            level: 0
        }],
        "Banking.open queues open-booth; the host finds the nearest Use-quickly loc"
    );
    iso.join();
}

#[test]
fn isolate_bank_open_nearest_and_open_booth_queue_open_booth() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    loop() {
        Bank.openNearest('Bank booth', 'Use-quickly');
        Bank.openBooth({ x: 1, z: 1, level: 0 }, 'Bank booth', 'Use-quickly');
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(script::isolate_fb::TileInput {
        x: 100,
        z: 100,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1");
    assert_eq!(
        iso.drain_interacts(),
        vec![
            script::shim::InteractReq::OpenBooth {
                x: 0,
                z: 0,
                level: 0
            },
            script::shim::InteractReq::OpenBooth {
                x: 0,
                z: 0,
                level: 0
            },
        ],
        "openNearest / openBooth are thin names onto open-booth"
    );
    iso.join();
}

#[test]
fn isolate_banking_deposit_all_matching_records_bank_side_ops() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    loop() {
        Bank.depositAllMatching((name) => true);
        Bank.withdraw('Bones', 'all');
        Bank.withdraw('Lobster', 10);
        Bank.withdraw('Vial', 1);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(script::isolate_fb::TileInput {
        x: 100,
        z: 100,
        level: 0,
    });
    snap.bank = &[
        (Some("Bones"), 20),
        (Some("Lobster"), 30),
        (Some("Vial"), 1),
    ];
    snap.bank_side = &[(Some("Bones"), 3), (Some("Big bones"), 1)];
    snap.bank_open = true;
    snap.bank_loaded = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1"); // round-trip: the tick finished first
    assert_eq!(
        iso.drain_interacts(),
        vec![
            script::shim::InteractReq::Deposit {
                name: "Bones".into()
            },
            script::shim::InteractReq::Deposit {
                name: "Big bones".into()
            },
            script::shim::InteractReq::Withdraw {
                name: "Bones".into(),
                action: "Withdraw All".into()
            },
            script::shim::InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw 10".into()
            },
            script::shim::InteractReq::Withdraw {
                name: "Vial".into(),
                action: "Withdraw 1".into()
            },
        ],
        "depositAllMatching queues one Deposit-All per row; withdraw maps name + op"
    );
    iso.join();
}

// Task 7 — a bank deposit request never matches a name the 274 obj table
// does not know (the host resolves names through ObjNames): a blob row
// with a null name is skipped, and a missing member still throws.
#[test]
fn isolate_banking_deposit_skips_unknown_names_and_missing_members_throw() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    loop() {
        Bank.depositAllMatching(() => true);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(script::isolate_fb::TileInput {
        x: 100,
        z: 100,
        level: 0,
    });
    snap.bank_side = &[(None, 3)];
    snap.bank_open = true;
    snap.bank_loaded = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("1 + 1"); // round-trip: the tick finished first
    assert_eq!(
        iso.drain_interacts(),
        Vec::<script::shim::InteractReq>::new(),
        "a null-name row never queues a deposit"
    );
    iso.join();

    let src = "import { Banking } from '../../api/bank/Banking.js'; export default class T extends LoopingBot { loop() { Banking.close(); } }";
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_bot"); // round-trip: the tick finished first
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("not v1") && l.contains("Banking.close")),
        "missing Banking member throws: {logs:?}"
    );
    iso.join();
}

// Task 9 — kernel facades read posted snapshot rows and queue interact ops.
#[test]
fn kernel_facade_npcs_nearest_reads_posted_snapshot() {
    let src = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
export default class T extends LoopingBot {
    loop() {
        const nearest = Npcs.nearest(2);
        globalThis.__probe = nearest.map(n => ({ name: n.name, distance: n.distance() }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let attack = ["Attack".to_string()];
    let npcs = [
        script::isolate_fb::SceneEntityInput {
            index: 1,
            id: 41,
            name: Some("Chicken"),
            x: 3220,
            z: 3220,
            level: 0,
            distance: 5,
            health: 3,
            max_health: 3,
            in_combat: false,
            animating: false,
            actions: &attack,
        },
        script::isolate_fb::SceneEntityInput {
            index: 2,
            id: 41,
            name: Some("Chicken"),
            x: 3221,
            z: 3220,
            level: 0,
            distance: 2,
            health: 3,
            max_health: 3,
            in_combat: false,
            animating: false,
            actions: &attack,
        },
    ];
    let mut snap = base_snapshot();
    snap.npcs = &npcs;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(
        value,
        serde_json::json!([
            { "name": "Chicken", "distance": 2 },
            { "name": "Chicken", "distance": 5 }
        ]),
        "Npcs.nearest sorts by posted distance"
    );
    iso.join();
}

#[test]
fn kernel_facade_game_cast_on_item_queues_use_widget_on() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    async loop() {
        const item = Inventory.first('Steel platebody');
        globalThis.__probe = await Game.castOnItem('High level alchemy', item);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.inv = &[(Some("Steel platebody"), 1)];
    snap.combat_styles = &[script::isolate_fb::CombatStyleInput {
        mode: 0,
        label: "High level alchemy",
        component_id: 1234,
    }];
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("__probe");
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::UseWidgetOn {
            component_id: 1234,
            kind: "held".into(),
            target_name: Some("Steel platebody".into()),
            x: 0,
            z: 0,
            level: 0,
            index: None,
        }],
        "Game.castOnItem queues use-widget-on from posted spell label"
    );
    iso.join();
}

#[test]
fn kernel_facade_chat_dialog_continue_queues_continue_op() {
    let src = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__probe = await ChatDialog.continue();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.chat_open = true;
    snap.chat_continue = true;
    snap.chat_modal_id = 4882;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("__probe");
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::ContinueDialog],
        "ChatDialog.continue queues the continue interact op"
    );
    iso.join();
}

// Task 9 fix — Game.autoRetaliate is the gold export name (not autoRetaliateOn).
#[test]
fn kernel_facade_game_auto_retaliate_is_exported() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = typeof Game.autoRetaliate;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value, "function", "Game.autoRetaliate must be exported");
    let logs = iso.drain_logs();
    assert!(
        logs.iter().all(|l| !l.contains("not v1")),
        "autoRetaliate must not throw not v1: {logs:?}"
    );
    iso.join();
}

// Task 9 fix — Row FB has name+count only; countById cannot observe ids.
#[test]
fn kernel_facade_inventory_count_by_id_throws_not_v1() {
    let src = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    loop() {
        Inventory.countById(526);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.inv = &[(Some("Bones"), 5)];
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_bot");
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("not v1") && l.contains("Inventory.countById")),
        "countById must throw not v1 until ids exist: {logs:?}"
    );
    iso.join();
}

// Task 9 fix — item.useOn derives interact kind from the target entity type.
#[test]
fn kernel_facade_inventory_use_on_loc_queues_loc_kind() {
    let src = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
import { Locs } from '../../api/locs/Locs.js';
export default class T extends LoopingBot {
    loop() {
        const item = Inventory.first('Knife');
        const loc = Locs.query().nearest();
        globalThis.__probe = item ? item.useOn(loc) : false;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let use_actions = ["Use".to_string()];
    let locs = [script::isolate_fb::SceneEntityInput {
        index: 0,
        id: 873,
        name: Some("Tree"),
        x: 3220,
        z: 3220,
        level: 0,
        distance: 1,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &use_actions,
    }];
    let mut snap = base_snapshot();
    snap.inv = &[(Some("Knife"), 1)];
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value, true, "useOn accepts a loc target");
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::UseOn {
            name: "Knife".into(),
            kind: "loc".into(),
            target_name: Some("Tree".into()),
            x: 3220,
            z: 3220,
            level: 0,
            index: None,
        }],
        "useOn on a loc must not hardcode kind npc"
    );
    iso.join();
}

#[test]
fn kernel_facade_gold_import_paths_resolve() {
    let src = r#"
import Tile from '../../geometry/Tile.js';
import { Npcs } from '../../api/npcs/Npcs.js';
import { Locs } from '../../api/locs/Locs.js';
import { Players } from '../../api/players/Players.js';
import { GroundItems } from '../../api/grounditems/GroundItems.js';
import { Equipment } from '../../api/equipment/Equipment.js';
import { ContinueDialog } from '../../api/tasks/ContinueDialog.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { Reach } from '../../api/walking/Reach.js';
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
import { openOp } from '../../event/webwalk/walkOpening.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            tile: Tile.from({ x: 1, z: 2, level: 0 }).toString(),
            npcs: typeof Npcs.nearest,
            locs: typeof Locs.query,
            players: typeof Players.all,
            ground: typeof GroundItems.query,
            equip: typeof Equipment.contains,
            task: typeof ContinueDialog,
            walk: typeof Traversal.walkResilient,
            reach: typeof Reach.entityOp,
            reachability: typeof Reachability.canReach,
            openOp: typeof openOp,
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["tile"], "(1, 2, 0)");
    assert_eq!(value["npcs"], "function");
    assert_eq!(value["locs"], "function");
    assert_eq!(value["players"], "function");
    assert_eq!(value["ground"], "function");
    assert_eq!(value["equip"], "function");
    assert_eq!(value["task"], "function");
    assert_eq!(value["walk"], "function");
    assert_eq!(value["reach"], "function");
    assert_eq!(value["reachability"], "function");
    assert_eq!(value["openOp"], "function");
    iso.join();
}
