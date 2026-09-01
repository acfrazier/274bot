//! JS Load: shape detection, the picker library of loaded JS cards, and the
//! out-of-tree `LoadIsolate` (rustyscript V8 on its own thread).
//!
//! Loading (`JsLibrary::load`) only reads, classifies, validates the source
//! in a throwaway Runtime (dropped before `load()` returns), registers the
//! card, and persists `{name, path}`. The isolate is spawned **only** on
//! Start of a JS card (`LoadIsolate::spawn`); nothing here `include_str!`s
//! a script tree. 0.1.5 listed TS is an operator `$RS2B0T` path.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use crate::rs2b0t_registry::{parse_registry, persist_rs2b0t_root_at, script_file_path};

/// Which loader a JS source belongs to.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LoadShape {
    /// Old rs2b0t `defineBot(...)` / manifest-flagged source.
    CompatDefineBot,
    /// Catalog shape: default-export `LoopingBot`/`TaskBot`/`TreeBot`
    /// subclass (TS, transpiled at Load and at isolate spawn).
    CompatClass,
    /// Modern source exporting a `tick` function.
    NativeTick,
    /// Not a recognized bot shape.
    Reject,
}

/// Classify a JS source by marker scan. Compat markers win over the
/// native `tick` export when a source carries both, and a default-export
/// `LoopingBot`/`TaskBot`/`TreeBot` subclass (the catalog shape) also
/// wins over the native `tick` export.
pub fn detect_shape(source: &str) -> LoadShape {
    if source.contains("defineBot(") || source.contains("__rs2b0tManifest") {
        LoadShape::CompatDefineBot
    } else if source.contains("export default class")
        && ["LoopingBot", "TaskBot", "TreeBot"]
            .iter()
            .any(|base| source.contains(&format!("extends {base}")))
    {
        LoadShape::CompatClass
    } else if source.contains("export function tick")
        || source.contains("export async function tick")
    {
        LoadShape::NativeTick
    } else {
        LoadShape::Reject
    }
}

/// Strip TypeScript from `source` (types, `private`/`override` markers,
/// type-only imports) and re-emit as a JavaScript module V8 can parse.
/// Plain JS passes through unchanged in behaviour. A parse failure means
/// the source is not readable TypeScript/JavaScript.
#[cfg(feature = "load")]
pub fn transpile_ts(source: &str) -> Result<String, String> {
    let specifier = deno_ast::ModuleSpecifier::parse("file:///bot.ts")
        .map_err(|e| format!("ts specifier: {e}"))?;
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: source.to_string().into(),
        media_type: deno_ast::MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|e| format!("ts parse: {e}"))?;
    let emitted = parsed
        .transpile(
            &deno_ast::TranspileOptions::default(),
            &deno_ast::TranspileModuleOptions::default(),
            &deno_ast::EmitOptions {
                source_map: deno_ast::SourceMapOption::None,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?
        .into_source();
    Ok(emitted.text)
}

/// A loaded JS bot: the picker name (file stem), its source path, the
/// loader shape, and the source text captured at Load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsCard {
    pub name: String,
    pub path: PathBuf,
    pub shape: LoadShape,
    pub source: String,
}

/// Default persisted library path (`~/.274bot/js-scripts.json`).
pub fn default_js_store() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/js-scripts.json")),
        Err(_) => PathBuf::from(".274bot/js-scripts.json"),
    }
}

/// One persisted library record: only the name and the source path (the
/// source itself is re-read from disk on restore).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoreEntry {
    name: String,
    path: String,
}

/// The out-of-tree JS library: the picker cards for loaded files, persisted
/// to `store`, plus cards filled from the `$RS2B0T` registry. Same `name`
/// (file stem or register name) overwrites; only WalkTo is reserved;
/// non-bot shapes are rejected at Load.
pub struct JsLibrary {
    store: PathBuf,
    cards: Vec<JsCard>,
}

impl JsLibrary {
    pub fn new(store: PathBuf) -> Self {
        JsLibrary {
            store,
            cards: Vec::new(),
        }
    }

    /// Read the persisted `{name, path}` list back into cards. Sources are
    /// re-read from disk and re-classified; entries whose file is gone or
    /// that no longer look like a bot are dropped. A missing store is not
    /// an error (first run).
    pub fn restore(&mut self) -> Result<(), String> {
        let raw = match std::fs::read_to_string(&self.store) {
            Ok(raw) => raw,
            Err(_) => return Ok(()),
        };
        let entries: Vec<StoreEntry> =
            serde_json::from_str(&raw).map_err(|e| format!("js-scripts.json: {e}"))?;
        self.cards.clear();
        for entry in entries {
            let path = PathBuf::from(&entry.path);
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            if detect_shape(&source) == LoadShape::Reject {
                continue;
            }
            if is_reserved(&entry.name) {
                continue;
            }
            self.cards.push(JsCard {
                name: entry.name,
                path,
                shape: detect_shape(&source),
                source,
            });
        }
        Ok(())
    }

    /// Register a JS bot from a filesystem path. Reads the source, derives
    /// the picker name from the file stem, classifies, rejects reserved
    /// ids (WalkTo only), validates the source compiles in a throwaway
    /// Runtime (dropped before this returns), then registers and persists.
    /// A second load with the same name replaces the previous card.
    pub fn load(&mut self, path: &Path) -> Result<JsCard, String> {
        let source =
            std::fs::read_to_string(path).map_err(|e| format!("load {}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("load {}: no file stem", path.display()))?
            .to_string();
        if is_reserved(&name) {
            return Err(format!("reserved: {name}"));
        }
        let shape = detect_shape(&source);
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        #[cfg(feature = "load")]
        {
            // Transpile at Load (types gone) so the throwaway Runtime
            // validates the JS V8 will actually parse.
            let js = transpile_ts(&source).map_err(|e| format!("{name}: {e}"))?;
            isolate::validate_compiles(&js, shape).map_err(|e| format!("{name}: {e}"))?;
        }
        let card = JsCard {
            name,
            path: path.to_path_buf(),
            shape,
            source,
        };
        // Transactional register: build the would-be list, persist it, and
        // only then commit, so a failed write leaves the library untouched.
        let new_cards: Vec<JsCard> = self
            .cards
            .iter()
            .filter(|c| c.name != card.name)
            .cloned()
            .chain(std::iter::once(card.clone()))
            .collect();
        self.persist_entries(
            &new_cards
                .iter()
                .map(|c| StoreEntry {
                    name: c.name.clone(),
                    path: c.path.to_string_lossy().to_string(),
                })
                .collect::<Vec<_>>(),
        )?;
        self.cards = new_cards;
        Ok(card)
    }

    /// All registered cards, in load order (a re-load moves the card to
    /// the back; the picker shows them after the compiled ids).
    pub fn cards(&self) -> &[JsCard] {
        &self.cards
    }

    /// Fill the library from the `$RS2B0T` catalog: statically parse
    /// `root/src/bot/scripts/index.ts` and register each script as a card
    /// under its register name (which may differ from the folder). Sources
    /// are read and classified only — no transpile, no V8 Runtime, no
    /// isolate. Reserved names (WalkTo) and non-bot shapes are skipped.
    /// The first successful parse persists `root` to `path_file` so later
    /// boots find the catalog without `$RS2B0T` set. Returns the number of
    /// cards registered.
    pub fn register_rs2b0t(&mut self, root: &Path, path_file: &Path) -> Result<usize, String> {
        let index = crate::rs2b0t_registry::registry_index_path(root);
        let index_ts = std::fs::read_to_string(&index)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let cards = parse_registry(&index_ts)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut n = 0;
        for card in &cards {
            if is_reserved(&card.name) {
                continue;
            }
            let path = script_file_path(root, &card.rel_path);
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let shape = detect_shape(&source);
            if shape == LoadShape::Reject {
                continue;
            }
            self.cards.retain(|c| c.name != card.name);
            self.cards.push(JsCard {
                name: card.name.clone(),
                path,
                shape,
                source,
            });
            n += 1;
        }
        let _ = persist_rs2b0t_root_at(root, path_file); // first successful parse records the path
        Ok(n)
    }

    /// The card registered under `name`, if any.
    pub fn get(&self, name: &str) -> Option<&JsCard> {
        self.cards.iter().find(|c| c.name == name)
    }

    /// Write the current `{name, path}` list to the store, creating the
    /// parent directory. Errors propagate so a load that cannot persist is
    /// reported instead of silently lost.
    pub fn persist(&self) -> Result<(), String> {
        self.persist_entries(
            &self
                .cards
                .iter()
                .map(|c| StoreEntry {
                    name: c.name.clone(),
                    path: c.path.to_string_lossy().to_string(),
                })
                .collect::<Vec<_>>(),
        )
    }

    fn persist_entries(&self, entries: &[StoreEntry]) -> Result<(), String> {
        if let Some(parent) = self.store.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("js-scripts.json: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(entries).map_err(|e| format!("js-scripts.json: {e}"))?;
        std::fs::write(&self.store, json).map_err(|e| format!("js-scripts.json: {e}"))
    }
}

/// True when `name` collides with a reserved picker id. Only WalkTo is
/// reserved: it is host nav, never a JS card. The abandoned rust-first
/// smokes (`BoneBurier` …) are free again — the shim catalog Loads them.
pub fn is_reserved(name: &str) -> bool {
    name == "WalkTo"
}

/// A picker selection: a compiled id or a loaded JS card (by name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSel {
    Compiled(crate::registry::CompiledId),
    Loaded(String),
}

impl ScriptSel {
    /// The label the picker shows and Start keys on.
    pub fn label(&self) -> String {
        match self {
            ScriptSel::Compiled(id) => id.0.to_string(),
            ScriptSel::Loaded(name) => name.clone(),
        }
    }
}

#[cfg(feature = "load")]
mod isolate {
    use super::*;
    use rustyscript::{json_args, Runtime, RuntimeOptions};
    use std::thread::JoinHandle;

    /// Per-tick budget: ticks taking longer than this are interrupted and
    /// logged, and stale ticks are skipped.
    const SLOW_TICK: Duration = Duration::from_millis(50);
    /// Hard stop for yielding JS (rustyscript `RuntimeOptions.timeout`).
    const RUNTIME_TIMEOUT: Duration = Duration::from_millis(50);
    /// How long `join` waits for the isolate thread after Stop + terminate
    /// before abandoning it: a stuck isolate must never freeze the caller.
    const JOIN_TIMEOUT: Duration = Duration::from_secs(2);
    /// Heap cap for the isolate (~64 MB, the brief's number).
    const MAX_HEAP: usize = 64 * 1024 * 1024;

    enum IsolateCmd {
        Tick(u64),
        Pause,
        Resume,
        Probe(String, Sender<Result<serde_json::Value, String>>),
        Stop,
    }

    enum ThreadMsg {
        Log(String),
        /// The highest tick the thread has fully processed (ran or skipped).
        Completed(u64),
    }

    /// One JS bot running in its own rustyscript/V8 isolate. Spawned only
    /// on Start; the Runtime lives on the thread and is reached through a
    /// command channel, so the host never blocks on JS. Ticks run on
    /// observed game-tick edges; stale ticks are skipped.
    pub struct LoadIsolate {
        tx: Sender<IsolateCmd>,
        rx: Mutex<Receiver<ThreadMsg>>,
        logs: Mutex<Vec<String>>,
        handle: Option<JoinHandle<()>>,
        /// Thread-safe handle used to terminate a runaway tick from this
        /// side of the channel. The terminate stays armed until the isolate
        /// thread has returned from the tick and clears it there (a cancel
        /// from this side would race the interrupt and make it a no-op).
        terminate: v8::IsolateHandle,
        /// The tick currently being dispatched and when it was sent; the
        /// thread clears it when the tick completes.
        in_flight: Mutex<Option<(u64, Instant)>>,
    }

    impl LoadIsolate {
        /// Spawn the isolate thread: init V8, create the Runtime (heap
        /// capped, per-op timeout), wire the module, then return a handle.
        /// The source is transpiled first (types gone) — plain JS passes
        /// through unchanged in behaviour — so both JS and TS cards run
        /// the same V8-parseable text. Fails with a message when the
        /// source cannot be wired.
        pub fn spawn(source: String, shape: LoadShape) -> Result<Self, String> {
            ensure_platform();
            let js = transpile_ts(&source)?;
            let (tx, rx) = mpsc::channel::<IsolateCmd>();
            let (msg_tx, msg_rx) = mpsc::channel::<ThreadMsg>();
            let (setup_tx, setup_rx) = mpsc::channel::<Result<v8::IsolateHandle, String>>();
            let handle = std::thread::Builder::new()
                .name("js-isolate".into())
                .spawn(move || isolate_main(js, shape, rx, msg_tx, setup_tx))
                .map_err(|e| format!("isolate thread: {e}"))?;
            let terminate = match setup_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(handle)) => handle,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(format!("isolate init: {e}")),
            };
            Ok(LoadIsolate {
                tx,
                rx: Mutex::new(msg_rx),
                logs: Mutex::new(Vec::new()),
                handle: Some(handle),
                terminate,
                in_flight: Mutex::new(None),
            })
        }

        /// Dispatch one observed game tick to the isolate. The previous
        /// tick is checked against the budget: still running past
        /// [`SLOW_TICK`] is interrupted and logged, and its stale ticks are
        /// skipped.
        pub fn on_game_tick(&self, snap_tick: u64) {
            self.pump_logs();
            let interrupted = {
                let mut in_flight = self.in_flight.lock().unwrap();
                // The previous tick is still in flight (no `Completed`
                // folded yet) past the budget: interrupt it.
                let over = in_flight
                    .as_ref()
                    .filter(|(_, started)| started.elapsed() > SLOW_TICK)
                    .map(|(tick, started)| (*tick, started.elapsed()));
                *in_flight = Some((snap_tick, Instant::now()));
                over
            };
            if let Some((tick, elapsed)) = interrupted {
                // Leave the terminate armed until the isolate thread has
                // returned from the tick (it cancels there); an immediate
                // cancel would race the interrupt and make this a no-op.
                self.terminate.terminate_execution();
                // `in_flight` was released before this lock, so the lock
                // order (never `in_flight` -> `logs`) holds everywhere.
                self.logs
                    .lock()
                    .unwrap()
                    .push(format!("interrupted slow tick {tick} ({elapsed:?})"));
            }
            let _ = self.tx.send(IsolateCmd::Tick(snap_tick));
        }

        /// Park tick dispatch. A runaway tick is interrupted first so the
        /// thread returns to the command loop.
        pub fn pause(&self) {
            self.pump_logs();
            let over = self
                .in_flight
                .lock()
                .unwrap()
                .map(|(_, started)| started.elapsed() > SLOW_TICK)
                .unwrap_or(false);
            if over {
                // No cancel here: the isolate thread clears the terminate
                // itself once it has returned from the interrupted tick.
                self.terminate.terminate_execution();
            }
            let _ = self.tx.send(IsolateCmd::Pause);
        }

        /// Re-arm tick dispatch after [`LoadIsolate::pause`].
        pub fn resume(&self) {
            let _ = self.tx.send(IsolateCmd::Resume);
        }

        /// Evaluate `expr` in the isolate's global scope and return its
        /// JSON value (test/status read-back; e.g. `"__rs_bot.n"`).
        pub fn probe(&self, expr: &str) -> Result<serde_json::Value, String> {
            let (tx, rx) = mpsc::channel::<Result<serde_json::Value, String>>();
            self.tx
                .send(IsolateCmd::Probe(expr.to_string(), tx))
                .map_err(|e| e.to_string())?;
            rx.recv_timeout(Duration::from_secs(10))
                .map_err(|e| format!("probe: {e}"))?
        }

        /// Drain the isolate's log lines (tick errors, slow/interrupted
        /// ticks).
        pub fn drain_logs(&self) -> Vec<String> {
            self.pump_logs();
            std::mem::take(&mut *self.logs.lock().unwrap())
        }

        /// Stop the isolate: tell the thread to exit, interrupt any running
        /// JS so the join cannot hang, and wait for the thread (the Runtime
        /// is dropped there). The wait is bounded by [`Self::JOIN_TIMEOUT`]:
        /// a stuck isolate is abandoned (thread detached) so Stop can never
        /// freeze the panel.
        pub fn join(mut self) {
            let _ = self.tx.send(IsolateCmd::Stop);
            self.terminate.terminate_execution();
            if let Some(handle) = self.handle.take() {
                let deadline = Instant::now() + JOIN_TIMEOUT;
                while !handle.is_finished() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if handle.is_finished() {
                    let _ = handle.join();
                }
                // else: abandoned — dropping the handle detaches the thread,
                // which exits on its own once the interrupt lands.
            }
        }

        /// Fold completed ticks and thread log lines into local state.
        /// `logs` and `in_flight` are never held together: the thread also
        /// takes them in this same order (`logs` -> `in_flight` would let a
        /// slow-tick interrupt deadlock against `on_game_tick`), so each
        /// message is folded under its own lock.
        fn pump_logs(&self) {
            let mut msgs = Vec::new();
            {
                let rx = self.rx.lock().unwrap();
                while let Ok(msg) = rx.try_recv() {
                    msgs.push(msg);
                }
            }
            let mut clear_through: Option<u64> = None;
            for msg in msgs {
                match msg {
                    ThreadMsg::Log(line) => self.logs.lock().unwrap().push(line),
                    ThreadMsg::Completed(up_to) => {
                        clear_through = Some(clear_through.map_or(up_to, |c| c.max(up_to)));
                    }
                }
            }
            if let Some(up_to) = clear_through {
                let mut in_flight = self.in_flight.lock().unwrap();
                if in_flight.map(|(t, _)| t <= up_to).unwrap_or(false) {
                    *in_flight = None;
                }
            }
        }
    }

    impl Drop for LoadIsolate {
        fn drop(&mut self) {
            // Best-effort: unblock a stuck tick and close the channel; the
            // thread exits and drops its Runtime by itself (no join here,
            // and no cancel — the thread clears the terminate once the tick
            // has returned).
            let _ = self.tx.send(IsolateCmd::Stop);
            self.terminate.terminate_execution();
        }
    }

    /// Initialize the V8 platform once, on the caller's thread (Start and
    /// Load both run here, and the isolate threads are spawned by it).
    fn ensure_platform() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            rustyscript::init_platform(1, true);
        });
    }

    /// The isolate thread: create the Runtime, wire the module, hand the
    /// thread-safe isolate handle back, then run the tick loop.
    fn isolate_main(
        source: String,
        shape: LoadShape,
        cmds: Receiver<IsolateCmd>,
        out: Sender<ThreadMsg>,
        setup: Sender<Result<v8::IsolateHandle, String>>,
    ) {
        let mut runtime = match Runtime::new(RuntimeOptions {
            timeout: RUNTIME_TIMEOUT,
            max_heap_size: Some(MAX_HEAP),
            ..Default::default()
        }) {
            Ok(runtime) => runtime,
            Err(e) => {
                let _ = setup.send(Err(format!("js engine init: {e}")));
                return;
            }
        };
        if let Err(e) = wire_runtime(&mut runtime, &source, shape) {
            let _ = setup.send(Err(e));
            return;
        }
        let terminate = runtime.deno_runtime().v8_isolate().thread_safe_handle();
        let _ = setup.send(Ok(terminate));
        tick_loop(runtime, cmds, out);
    }

    /// Load `source` into `runtime` as a module and wire the global tick
    /// entry. Native sources export `tick(api)`; compat sources
    /// default-export a `defineBot` config and tick through `create()`'s
    /// `loop()`, and the catalog shape default-exports a
    /// `LoopingBot`/`TaskBot`/`TreeBot` subclass that is instantiated and
    /// ticked through its `loop()`.
    ///
    /// The shim prelude and the extra rs2b0t-named modules are wired first
    /// so relative `../../api/...` imports and the remapped
    /// `@rs2b0t/api` bundle resolve to our modules; an import that does
    /// not name a shim module (e.g. `../../api/bank/Banking.js` before
    /// its task) fails the load honestly.
    fn wire_runtime(runtime: &mut Runtime, source: &str, shape: LoadShape) -> Result<(), String> {
        if shape == LoadShape::Reject {
            return Err("not a bot shape".to_string());
        }
        let source = crate::shim::remap_rs2b0t_api(source);
        runtime
            .eval::<()>(crate::shim::PRELUDE)
            .map_err(|e| format!("shim: {e}"))?;
        let bot = rustyscript::Module::new(crate::shim::BOT_MODULE, source);
        let main = match shape {
            LoadShape::NativeTick => {
                rustyscript::Module::new(crate::shim::MAIN_MODULE, NATIVE_MAIN)
            }
            LoadShape::CompatDefineBot => {
                rustyscript::Module::new(crate::shim::MAIN_MODULE, COMPAT_MAIN)
            }
            LoadShape::CompatClass => {
                rustyscript::Module::new(crate::shim::MAIN_MODULE, COMPAT_CLASS_MAIN)
            }
            LoadShape::Reject => unreachable!("rejected above"),
        };
        // Side modules load in order, so the shim modules (which the bot
        // imports) must precede the bot's own module.
        let mut side = crate::shim::shim_modules();
        side.push(bot);
        let side: Vec<&rustyscript::Module> = side.iter().collect();
        // Loading evaluates the modules, so a missing `tick`/default
        // export, an unresolvable import, or a syntax error surfaces
        // here, before any tick runs.
        runtime
            .load_modules(&main, side)
            .map_err(|e| format!("load: {e}"))?;
        Ok(())
    }

    /// Validate a source in a throwaway Runtime, dropped before `load()`
    /// returns.
    pub(crate) fn validate_compiles(source: &str, shape: LoadShape) -> Result<(), String> {
        ensure_platform();
        let mut runtime = Runtime::new(RuntimeOptions {
            timeout: Duration::from_secs(2),
            max_heap_size: Some(MAX_HEAP),
            ..Default::default()
        })
        .map_err(|e| format!("js engine init: {e}"))?;
        let result = wire_runtime(&mut runtime, source, shape);
        drop(runtime); // compile Runtime must not outlive load()
        result
    }

    /// Native wrapper: re-export the module's `tick` behind a global that
    /// receives the tick number and a persistent `api` object. `api` is a
    /// Proxy: the host owns it (`api.tick` is set each tick); reading or
    /// writing any other member throws `not v1` — a script stashes its own
    /// state elsewhere, never in host-owned slots.
    const NATIVE_MAIN: &str = r#"
import { tick } from './bot.js';
const api = new Proxy({}, {
    get(target, prop) {
        if (typeof prop === 'symbol') return undefined;
        if (prop === 'tick') return target.tick;
        throw new Error('not v1: api.' + String(prop));
    },
    set(target, prop, value) {
        if (prop === 'tick') { target.tick = value; return true; }
        throw new Error('not v1: api.' + String(prop));
    },
});
globalThis.__rs_api = api;
globalThis.__rs_tick = (n) => { api.tick = n; return tick(api); };
"#;

    /// Compat wrapper: `create()` the bot instance, call `onStart` once,
    /// then `loop()` (and `onPaint` with the dummy ctx) every tick.
    const COMPAT_MAIN: &str = r#"
import bot from './bot.js';
const inst = (bot && typeof bot.create === 'function') ? bot.create() : (bot || null);
if (inst && typeof inst.onStart === 'function') { inst.onStart(); }
globalThis.__rs_tick = () => {
    if (!inst) return;
    if (typeof inst.loop === 'function') { inst.loop(); }
    if (typeof inst.onPaint === 'function') { inst.onPaint(globalThis.__dummy_ctx); }
};
"#;

    /// Compat class wrapper: instantiate the default-export
    /// `LoopingBot`/`TaskBot`/`TreeBot` subclass, call `onStart` once,
    /// then `loop()` (and `onPaint` with the dummy ctx) every tick. The
    /// instance is exposed as `__rs_bot` for probe read-back.
    const COMPAT_CLASS_MAIN: &str = r#"
import bot from './bot.js';
const inst = new bot();
globalThis.__rs_bot = inst;
if (inst && typeof inst.onStart === 'function') { inst.onStart(); }
globalThis.__rs_tick = () => {
    if (!inst) return;
    if (typeof inst.loop === 'function') { inst.loop(); }
    if (typeof inst.onPaint === 'function') { inst.onPaint(globalThis.__dummy_ctx); }
};
"#;

    /// The tick loop: commands are serialized on this thread; ticks run
    /// with a time budget, slow ticks are logged and stale queued ticks are
    /// skipped, and errors never kill the isolate.
    ///
    /// The stale-skip drain consumes commands with an explicit match so a
    /// non-Tick command (Pause/Resume/Probe/Stop) that arrives while ticks
    /// are queued is stashed for the next iteration instead of being
    /// dropped (a `while let Ok(IsolateCmd::Tick(..))` pattern would
    /// swallow it).
    fn tick_loop(mut runtime: Runtime, cmds: Receiver<IsolateCmd>, out: Sender<ThreadMsg>) {
        let mut paused = false;
        let mut pending: Option<IsolateCmd> = None;
        loop {
            let cmd = match pending.take() {
                Some(cmd) => cmd,
                None => match cmds.recv() {
                    Ok(cmd) => cmd,
                    Err(_) => break,
                },
            };
            match cmd {
                IsolateCmd::Tick(n) => {
                    if paused {
                        continue;
                    }
                    let start = Instant::now();
                    let result: Result<serde_json::Value, rustyscript::Error> =
                        runtime.call_function(None, "__rs_tick", json_args!(n));
                    // The host may have armed `terminate_execution` to
                    // interrupt a slow tick; clear it now that the tick's
                    // JS frames have fully unwound. This is the only cancel
                    // point — canceling from the host would race the
                    // interrupt and make it a no-op.
                    runtime
                        .deno_runtime()
                        .v8_isolate()
                        .cancel_terminate_execution();
                    let elapsed = start.elapsed();
                    if let Err(e) = result {
                        let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
                    }
                    // ScriptRunner.stop signal: the script flags the host
                    // handle. The isolate treats it like `IsolateCmd::Stop`
                    // — fold the completed tick, log the stop, and break the
                    // loop so the Runtime is dropped and the host's join
                    // returns. A later task may also surface the stop to the
                    // slot, but Stop is never parked on Execution wiring.
                    let stopped = runtime
                        .eval::<bool>(
                            "!!(globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.stopRequested)",
                        )
                        .unwrap_or(false);
                    if stopped {
                        let _ = out.send(ThreadMsg::Completed(n));
                        let _ = out.send(ThreadMsg::Log(format!(
                            "script requested stop on tick {n}; isolate stopping"
                        )));
                        break;
                    }
                    if elapsed > SLOW_TICK {
                        let _ = out.send(ThreadMsg::Log(format!("slow tick {n}: {elapsed:?}")));
                        // Skip stale queued ticks: a slow tick means the
                        // pump backed up, so only the newest matters.
                        let mut latest = n;
                        loop {
                            match cmds.try_recv() {
                                Ok(IsolateCmd::Tick(next)) => latest = next,
                                Ok(other) => {
                                    pending = Some(other);
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                        if latest != n {
                            let _ = out
                                .send(ThreadMsg::Log(format!("skipped stale ticks -> {latest}")));
                        }
                        let _ = out.send(ThreadMsg::Completed(latest));
                    } else {
                        let _ = out.send(ThreadMsg::Completed(n));
                    }
                }
                IsolateCmd::Pause => paused = true,
                IsolateCmd::Resume => paused = false,
                IsolateCmd::Probe(expr, reply) => {
                    let value: Result<serde_json::Value, String> =
                        runtime.eval(expr).map_err(|e| e.to_string());
                    let _ = reply.send(value);
                }
                IsolateCmd::Stop => break,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn shim_prelude_defines_globals_for_compat_fixture() {
            ensure_platform();
            let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
            runtime.eval::<()>(crate::shim::PRELUDE).unwrap();
            let t: bool = runtime.eval("typeof defineBot === 'function'").unwrap();
            assert!(t);
            let t: bool = runtime.eval("typeof TaskBot === 'function'").unwrap();
            assert!(t);
            let t: bool = runtime.eval("typeof TreeBot === 'function'").unwrap();
            assert!(t);
            let t: bool = runtime.eval("typeof LoopingBot === 'function'").unwrap();
            assert!(t);
            let t: bool = runtime.eval("typeof __rs2b0t_host === 'object'").unwrap();
            assert!(t);
            // defineBot validates { name, create } instead of no-op'ing.
            let err: bool = runtime
                .eval("(() => { try { defineBot({}); return false; } catch { return true; } })()")
                .unwrap();
            assert!(err, "defineBot throws without a name/create pair");
        }
    }
}

#[cfg(feature = "load")]
pub use isolate::LoadIsolate;
