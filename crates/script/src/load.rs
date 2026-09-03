//! JS Load: shape detection, the picker library of loaded JS cards, and the
//! out-of-tree `LoadIsolate` (rustyscript V8 on its own thread).
//!
//! Loading (`JsLibrary::load`) only reads, classifies, validates the source
//! in a throwaway Runtime (dropped before `load()` returns), registers the
//! card, and persists `{name, path}`. The isolate is spawned **only** on
//! Start of a JS card (`LoadIsolate::spawn`); nothing here `include_str!`s
//! a script tree. 0.1.5 listed TS is an operator `$RS2B0T` path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use crate::js_cache::{default_js_cache_root, CacheMeta, JsCache};
use crate::rs2b0t_registry::{
    parse_registry_with_sources, persist_rs2b0t_root_at, script_file_path, ScriptKind,
    ScriptSource, SettingDef,
};

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

/// A loaded JS bot: picker name, origin path, loader shape, origin text,
/// cached JS (SHA object), execution kind, provenance, and content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsCard {
    pub name: String,
    pub path: PathBuf,
    pub shape: LoadShape,
    pub origin: String,
    pub js: String,
    pub kind: ScriptKind,
    pub source: ScriptSource,
    pub sha256: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub settings_schema: Vec<SettingDef>,
}

/// Default persisted library path (`~/.274bot/js-scripts.json`).
pub fn default_js_store() -> PathBuf {
    crate::bot_file("js-scripts.json")
}

/// One persisted library record: only the name and the source path (the
/// source itself is re-read from disk on restore).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoreEntry {
    name: String,
    path: String,
}

/// The out-of-tree JS library: picker cards for loaded files and the
/// `$RS2B0T` catalog, persisted to `store`, with origin bytes cached
/// under `cache`. Same `(source, name)` overwrites; only WalkTo is
/// reserved; non-bot shapes are rejected at Load.
pub struct JsLibrary {
    store: PathBuf,
    cache: JsCache,
    cards: Vec<JsCard>,
}

impl JsLibrary {
    pub fn new(store: PathBuf) -> Self {
        Self::with_cache(store, default_js_cache_root())
    }

    /// Like [`JsLibrary::new`] but with an explicit JS cache root (tests
    /// must use a temp dir, never the operator's `~/.274bot`).
    pub fn with_cache(store: PathBuf, cache_root: PathBuf) -> Self {
        JsLibrary {
            store,
            cache: JsCache::new(cache_root),
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
            let Ok(origin) = std::fs::read_to_string(&path) else {
                continue;
            };
            if detect_shape(&origin) == LoadShape::Reject {
                continue;
            }
            if is_reserved(&entry.name) {
                continue;
            }
            let shape = detect_shape(&origin);
            let cached = match self.cache.get_or_transpile(
                &path,
                origin.as_bytes(),
                CacheMeta {
                    kind: shape_to_kind(shape),
                    source: ScriptSource::File,
                    shape: Some(shape_label(shape).into()),
                },
            ) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let settings_schema = crate::rs2b0t_registry::settings_schema_from_source(&origin);
            self.cards.push(JsCard {
                name: entry.name,
                path,
                shape,
                origin,
                js: cached.js,
                kind: shape_to_kind(shape),
                source: ScriptSource::File,
                sha256: cached.sha256,
                description: String::new(),
                category: String::new(),
                tags: Vec::new(),
                settings_schema,
            });
        }
        Ok(())
    }

    /// Register a JS bot from a filesystem path. Reads the origin, caches
    /// transpiled JS under `~/.274bot/js-cache`, and statically parses
    /// `export const SETTINGS` for the picker schema. V8 only runs on
    /// Start. A second load with the same `(ScriptSource::File, name)`
    /// replaces the previous card.
    pub fn load(&mut self, path: &Path) -> Result<JsCard, String> {
        let origin =
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
        let shape = detect_shape(&origin);
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        let cached = self
            .cache
            .get_or_transpile(
                path,
                origin.as_bytes(),
                CacheMeta {
                    kind: shape_to_kind(shape),
                    source: ScriptSource::File,
                    shape: Some(shape_label(shape).into()),
                },
            )
            .map_err(|e| format!("{name}: {e}"))?;
        let settings_schema = crate::rs2b0t_registry::settings_schema_from_source(&origin);
        let card = JsCard {
            name,
            path: path.to_path_buf(),
            shape,
            origin,
            js: cached.js,
            kind: shape_to_kind(shape),
            source: ScriptSource::File,
            sha256: cached.sha256,
            description: String::new(),
            category: String::new(),
            tags: Vec::new(),
            settings_schema,
        };
        let new_cards: Vec<JsCard> = self
            .cards
            .iter()
            .filter(|c| !(c.source == card.source && c.name == card.name))
            .cloned()
            .chain(std::iter::once(card.clone()))
            .collect();
        self.persist_file_cards(&new_cards)?;
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
        let registry_cards = parse_registry_with_sources(&index_ts, &HashMap::new())
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut sources = HashMap::new();
        for reg in &registry_cards {
            let Some(path) = script_file_path(root, &reg.rel_path) else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                sources.insert(reg.rel_path.clone(), text);
            }
        }
        let cards = parse_registry_with_sources(&index_ts, &sources)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut n = 0;
        for card in &cards {
            if is_reserved(&card.name) {
                continue;
            }
            let Some(path) = script_file_path(root, &card.rel_path) else {
                continue;
            };
            let Ok(origin) = std::fs::read_to_string(&path) else {
                continue;
            };
            let shape = detect_shape(&origin);
            if shape == LoadShape::Reject {
                continue;
            }
            // Origin/classify only — no transpile, no V8. Warmth is
            // [`JsLibrary::ensure_js`] on first click / Start / Transpile all.
            let sha256 = JsCache::origin_sha(origin.as_bytes());
            self.cards
                .retain(|c| !(c.source == ScriptSource::Catalog && c.name == card.name));
            self.cards.push(JsCard {
                name: card.name.clone(),
                path,
                shape,
                origin,
                js: String::new(),
                kind: card.kind,
                source: ScriptSource::Catalog,
                sha256,
                description: card.description.clone(),
                category: card.category.clone(),
                tags: card.tags.clone(),
                settings_schema: card.settings_schema.clone(),
            });
            n += 1;
        }
        let _ = persist_rs2b0t_root_at(root, path_file);
        Ok(n)
    }

    /// Re-read `card`'s origin from disk; when the SHA differs, fetch a new
    /// cached object and update that `(source, name)` card in place. Isolate
    /// respawn is the caller's job — the updated `js`/`sha256` are on the
    /// card when they do.
    pub fn refresh(&mut self, source: ScriptSource, name: &str) -> Result<(), String> {
        let idx = self
            .cards
            .iter()
            .position(|c| c.source == source && c.name == name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?;
        let path = self.cards[idx].path.clone();
        let origin = std::fs::read_to_string(&path)
            .map_err(|e| format!("refresh {}: {e}", path.display()))?;
        let shape = detect_shape(&origin);
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        let cached = self
            .cache
            .get_or_transpile(
                &path,
                origin.as_bytes(),
                CacheMeta {
                    kind: shape_to_kind(shape),
                    source,
                    shape: Some(shape_label(shape).into()),
                },
            )
            .map_err(|e| format!("{name}: {e}"))?;
        let card = &mut self.cards[idx];
        card.path = path;
        card.shape = shape;
        card.origin = origin;
        card.js = cached.js;
        card.kind = shape_to_kind(shape);
        card.sha256 = cached.sha256;
        Ok(())
    }

    /// The card registered under `(source, name)`, if any.
    pub fn get(&self, source: ScriptSource, name: &str) -> Option<&JsCard> {
        self.cards
            .iter()
            .find(|c| c.source == source && c.name == name)
    }

    /// First card with `name`. Prefer [`JsLibrary::get`] when `(source, name)`
    /// is known — the same stem may exist as both catalog and file cards.
    pub fn find_name(&self, name: &str) -> Option<&JsCard> {
        self.cards.iter().find(|c| c.name == name)
    }

    /// Write the current `{name, path}` list to the store, creating the
    /// parent directory. Errors propagate so a load that cannot persist is
    /// reported instead of silently lost.
    pub fn persist(&self) -> Result<(), String> {
        self.persist_file_cards(&self.cards)
    }

    fn persist_file_cards(&self, cards: &[JsCard]) -> Result<(), String> {
        let entries: Vec<StoreEntry> = cards
            .iter()
            .filter(|c| c.source == ScriptSource::File)
            .map(|c| StoreEntry {
                name: c.name.clone(),
                path: c.path.to_string_lossy().to_string(),
            })
            .collect();
        self.persist_entries(&entries)
    }

    fn persist_entries(&self, entries: &[StoreEntry]) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(entries).map_err(|e| format!("js-scripts.json: {e}"))?;
        vault::write_private_file(&self.store, json.as_bytes())
            .map_err(|e| format!("js-scripts.json: {e}"))
    }

    /// The SHA cache backing this library (Start sibling resolve).
    pub fn cache(&self) -> &JsCache {
        &self.cache
    }

    /// True when this card already holds transpiled JS (isolate-ready).
    pub fn js_is_ready(&self, source: ScriptSource, name: &str) -> bool {
        self.get(source, name).is_some_and(|c| !c.js.is_empty())
    }

    /// Catalog/file cards whose origin is not in the SHA cache yet.
    /// Cache hits are omitted — they are a disk read, not a transpile.
    pub fn cards_needing_transpile(&self) -> Vec<(ScriptSource, String)> {
        self.cards
            .iter()
            .filter(|c| c.js.is_empty() && !self.cache.is_cached(c.origin.as_bytes()))
            .map(|c| (c.source, c.name.clone()))
            .collect()
    }

    /// Fill `card.js` from the SHA cache (transpile on miss). Idempotent
    /// when already ready. Does not spawn V8 — that is Start.
    pub fn ensure_js(&mut self, source: ScriptSource, name: &str) -> Result<(), String> {
        if self.js_is_ready(source, name) {
            return Ok(());
        }
        self.refresh(source, name)
    }
}

/// Scan `source` for same-folder `./Name.js` import specifiers (quoted).
pub fn scan_same_folder_js_imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for quote in ['\'', '"'] {
        let needle = format!("{quote}./");
        let mut rest = source;
        while let Some(idx) = rest.find(&needle) {
            let after = &rest[idx + needle.len()..];
            if let Some(end) = after.find(quote) {
                let spec = &after[..end];
                if spec.ends_with(".js")
                    && !spec.contains("..")
                    && !spec.contains('/')
                    && !spec.contains('\\')
                {
                    let import = format!("./{spec}");
                    if !out.iter().any(|x| x == &import) {
                        out.push(import);
                    }
                }
            }
            rest = &rest[idx + 1..];
        }
    }
    out
}

/// Map a `./Foo.js` import from [`BOT_MODULE`] to the synthetic module URL
/// rustyscript resolves.
pub fn sibling_module_url(import_rel: &str) -> Option<String> {
    let name = import_rel.strip_prefix("./")?;
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(format!("/rs2b0t/bot/scripts/bot/{name}"))
}

/// Resolve a same-folder `./Foo.js` import beside `card_path`. The `.ts`
/// twin wins when the verbatim `.js` path is absent. Rejects `..` and
/// paths outside `card_dir` (Load has no catalog sandbox, but siblings
/// must stay beside the picked file).
pub fn resolve_sibling_path(card_dir: &Path, import_rel: &str) -> Option<PathBuf> {
    let rel = import_rel.strip_prefix("./")?;
    if rel.contains("..") {
        return None;
    }
    let verbatim = card_dir.join(rel);
    let candidate = if verbatim.is_file() {
        verbatim
    } else if let Some(stem) = rel.strip_suffix(".js") {
        let ts = card_dir.join(format!("{stem}.ts"));
        if ts.is_file() {
            ts
        } else {
            return None;
        }
    } else {
        return None;
    };
    canonical_under_dir(card_dir, &candidate)
}

fn canonical_under_dir(dir: &Path, path: &Path) -> Option<PathBuf> {
    if !path.starts_with(dir) {
        return None;
    }
    if let (Ok(canon_dir), Ok(canon_path)) = (dir.canonicalize(), path.canonicalize()) {
        if !canon_path.starts_with(&canon_dir) {
            return None;
        }
        return Some(canon_path);
    }
    Some(path.to_path_buf())
}

/// Same-folder `./Foo.js` imports beside `card_path`: read the `.ts` twin
/// (or `.js` origin), cache under `js-cache`, return `(module_url, js)` pairs
/// for extra rustyscript modules at Start.
pub fn resolve_sibling_modules(
    card_path: &Path,
    origin: &str,
    cache: &JsCache,
    meta: CacheMeta,
) -> Result<Vec<(String, String)>, String> {
    let card_dir = card_path
        .parent()
        .ok_or_else(|| format!("no parent dir for {}", card_path.display()))?;
    let mut out = Vec::new();
    for import_rel in scan_same_folder_js_imports(origin) {
        let Some(url) = sibling_module_url(&import_rel) else {
            continue;
        };
        let Some(path) = resolve_sibling_path(card_dir, &import_rel) else {
            continue;
        };
        let bytes = std::fs::read(&path).map_err(|e| format!("sibling {}: {e}", path.display()))?;
        let cached = cache.get_or_transpile(&path, &bytes, meta.clone())?;
        out.push((url, cached.js));
    }
    Ok(out)
}

/// True when `name` collides with a reserved picker id. Only WalkTo is
/// reserved: it is host nav, never a JS card. The abandoned rust-first
/// smokes (`BoneBurier` …) are free again — the shim catalog Loads them.
pub fn is_reserved(name: &str) -> bool {
    name == "WalkTo"
}

/// A picker selection: a compiled id or a loaded JS card by `(source, name)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSel {
    Compiled(crate::registry::CompiledId),
    Loaded(ScriptSource, String),
}

impl ScriptSel {
    /// The label the picker shows and Start keys on.
    pub fn label(&self) -> String {
        match self {
            ScriptSel::Compiled(id) => id.0.to_string(),
            ScriptSel::Loaded(_, name) => name.clone(),
        }
    }
}

fn shape_to_kind(shape: LoadShape) -> ScriptKind {
    match shape {
        LoadShape::NativeTick => ScriptKind::NativeTick,
        LoadShape::CompatDefineBot | LoadShape::CompatClass => ScriptKind::Compat,
        LoadShape::Reject => ScriptKind::Compat,
    }
}

fn shape_label(shape: LoadShape) -> &'static str {
    match shape {
        LoadShape::CompatDefineBot => "CompatDefineBot",
        LoadShape::CompatClass => "CompatClass",
        LoadShape::NativeTick => "NativeTick",
        LoadShape::Reject => "Reject",
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
        /// The host's FlatBuffer snapshot blob (schema: `crates/script/
        /// schema/isolate.fbs`), decoded on the isolate thread into the
        /// JS object the Game/Inventory/Skills/EventSignal shims read
        /// before the next dispatched tick. Never a JSON string.
        Snapshot(Vec<u8>),
        /// Merged operator settings JSON for the prelude's `this.settings.*`.
        Settings(String),
        Pause,
        Resume,
        Probe(String, Sender<Result<serde_json::Value, String>>),
        Stop,
    }

    enum ThreadMsg {
        Log(String),
        /// The tick's shim interact queue (`__rs2b0t_host.interact`), a
        /// FlatBuffer `InteractBatch` of [`crate::shim::InteractReq`]s
        /// forwarded after the tick's JS finished (parked or not).
        Interact(Vec<u8>),
        /// The tick's recorded paint frame (`Paint.begin` … `end()` on the
        /// host handle), a FlatBuffer `Paint` forwarded for the script
        /// paint views. The host reads the latest frame off the handle
        /// without a probe round-trip. Null frames are not forwarded — a
        /// script that stops painting keeps its last frame. Never a JSON
        /// value on this channel.
        Paint(Vec<u8>),
        /// The bot instance's `ignoredRandoms()` list, read on the isolate
        /// thread after the tick and cached on the host handle (no probe).
        IgnoredRandoms(Vec<String>),
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
        /// Interact requests forwarded by the tick thread (the shim
        /// `Bank`/`Banking` queue), drained by the host like logs.
        interacts: Mutex<Vec<crate::shim::InteractReq>>,
        /// The latest paint frame the tick thread forwarded (a
        /// [`crate::shim::ScriptPaint`] decoded off the host handle after
        /// each tick), read by the script paint views.
        paint: Mutex<Option<crate::shim::ScriptPaint>>,
        /// The bot instance's random-ignore list, forwarded by the tick
        /// thread after each tick (same source as the old probe path).
        ignored_randoms: Mutex<Vec<String>>,
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
        /// Spawn the isolate thread with already-cached JS (no transpile).
        /// Fails with a message when the source cannot be wired.
        pub fn spawn(
            js: String,
            shape: LoadShape,
            siblings: Vec<(String, String)>,
        ) -> Result<Self, String> {
            ensure_platform();
            let (tx, rx) = mpsc::channel::<IsolateCmd>();
            let (msg_tx, msg_rx) = mpsc::channel::<ThreadMsg>();
            let (setup_tx, setup_rx) = mpsc::channel::<Result<v8::IsolateHandle, String>>();
            let handle = std::thread::Builder::new()
                .name("js-isolate".into())
                .spawn(move || isolate_main(js, shape, siblings, rx, msg_tx, setup_tx))
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
                interacts: Mutex::new(Vec::new()),
                paint: Mutex::new(None),
                ignored_randoms: Mutex::new(Vec::new()),
                handle: Some(handle),
                terminate,
                in_flight: Mutex::new(None),
            })
        }

        /// Post the host's FlatBuffer snapshot blob into the isolate: the
        /// buffer is decoded on the isolate thread into the JS object on
        /// the host handle (`__rs2b0t_host.snapshot`) before the next
        /// dispatched tick, so the Game/Inventory/Skills/EventSignal shims
        /// read the fields the host observed this PLAYER_INFO. Only these
        /// fields are copied — no World clone. Commands are serialized on
        /// the isolate thread, so a post followed by
        /// [`LoadIsolate::on_game_tick`] reaches JS in that order.
        pub fn post_snapshot(&self, bytes: Vec<u8>) {
            let _ = self.tx.send(IsolateCmd::Snapshot(bytes));
        }

        /// Post the merged operator settings bag (schema defaults + panel/TUI
        /// overrides + optional scenario inject). The prelude's
        /// `this.settings.*` reads `__rs2b0t_host.settingsBag`.
        pub fn post_settings_bag(&self, bag: &serde_json::Map<String, serde_json::Value>) {
            let Ok(json) = serde_json::to_string(bag) else {
                return;
            };
            let _ = self.tx.send(IsolateCmd::Settings(json));
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

        /// The bot instance's random-ignore list (`inst.ignoredRandoms?.()`
        /// on `__rs_bot`, default `[]`): cached on the isolate thread
        /// after each tick (see [`ThreadMsg::IgnoredRandoms`]). A throwing /
        /// non-array method and a native `tick`-shaped card (no instance)
        /// fail closed to `[]`. No probe round-trip.
        pub fn ignored_randoms(&self) -> Vec<String> {
            self.pump_logs();
            self.ignored_randoms.lock().unwrap().clone()
        }

        /// Drain the isolate's log lines (tick errors, slow/interrupted
        /// ticks).
        pub fn drain_logs(&self) -> Vec<String> {
            self.pump_logs();
            std::mem::take(&mut *self.logs.lock().unwrap())
        }

        /// Drain the interact requests the tick's shim queued
        /// (`__rs2b0t_host.interact`), forwarded by the tick thread in
        /// tick order. The host dispatches them through the slot Driver;
        /// a malformed entry is logged and dropped, never fatal.
        pub fn drain_interacts(&self) -> Vec<crate::shim::InteractReq> {
            self.pump_logs();
            std::mem::take(&mut *self.interacts.lock().unwrap())
        }

        /// The latest recorded paint frame (the tick thread forwards the
        /// host handle's `paint` record after every tick that painted).
        /// `None` when the script has not painted yet. No probe
        /// round-trip — the host reads this every frame.
        pub fn paint(&self) -> Option<crate::shim::ScriptPaint> {
            self.pump_logs();
            self.paint.lock().unwrap().clone()
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
                    ThreadMsg::Interact(bytes) => {
                        // Decode the FlatBuffer interact batch (no JSON).
                        match crate::isolate_fb::decode_interact_batch(&bytes) {
                            Ok(reqs) => self.interacts.lock().unwrap().extend(reqs),
                            Err(e) => self.logs.lock().unwrap().push(format!("interact: {e}")),
                        }
                    }
                    ThreadMsg::Paint(bytes) => {
                        // Decode the FlatBuffer paint frame (no JSON).
                        match crate::isolate_fb::decode_paint(&bytes) {
                            Ok(paint) => {
                                let mut slot = self.paint.lock().unwrap();
                                if slot.as_ref() != Some(&paint) {
                                    *slot = Some(paint);
                                }
                            }
                            Err(e) => self.logs.lock().unwrap().push(format!("paint: {e}")),
                        }
                    }
                    ThreadMsg::IgnoredRandoms(list) => {
                        *self.ignored_randoms.lock().unwrap() = list;
                    }
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

    /// Read the bot instance's ignore list on the isolate thread (no probe).
    fn eval_ignored_randoms(runtime: &mut Runtime) -> Vec<String> {
        runtime
            .eval::<Vec<String>>(
                "(() => { const b = globalThis.__rs_bot; if (!b || typeof b.ignoredRandoms !== 'function') return []; const l = b.ignoredRandoms(); return Array.isArray(l) ? l.filter(x => typeof x === 'string') : []; })()",
            )
            .unwrap_or_default()
    }

    /// The isolate thread: create the Runtime, wire the module, hand the
    /// thread-safe isolate handle back, then run the tick loop.
    fn isolate_main(
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
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
        if let Err(e) = wire_runtime(&mut runtime, &source, shape, &siblings) {
            let _ = setup.send(Err(e));
            return;
        }
        let terminate = runtime.deno_runtime().v8_isolate().thread_safe_handle();
        let _ = setup.send(Ok(terminate));
        tick_loop(runtime, cmds, out);
    }

    /// Monotonic clock backing the prelude's `performance.now()` shim
    /// (rustyscript's default extensions define no `performance`). First
    /// call anchors at isolate-thread start.
    static CLOCK_START: OnceLock<Instant> = OnceLock::new();

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
    fn wire_runtime(
        runtime: &mut Runtime,
        source: &str,
        shape: LoadShape,
        siblings: &[(String, String)],
    ) -> Result<(), String> {
        if shape == LoadShape::Reject {
            return Err("not a bot shape".to_string());
        }
        let source = crate::shim::remap_rs2b0t_api(source);
        runtime
            .register_function(
                "__rs2b0t_now",
                |_args: &[rustyscript::serde_json::Value]| {
                    let start = CLOCK_START.get_or_init(Instant::now);
                    Ok(rustyscript::serde_json::Value::from(
                        start.elapsed().as_millis() as f64,
                    ))
                },
            )
            .map_err(|e| format!("register now: {e}"))?;
        runtime
            .eval::<()>(crate::shim::PRELUDE)
            .map_err(|e| format!("shim: {e}"))?;
        let bot = rustyscript::Module::new(crate::shim::BOT_MODULE, source);
        let main = match shape {
            LoadShape::NativeTick => {
                rustyscript::Module::new(crate::shim::MAIN_MODULE, NATIVE_MAIN)
            }
            LoadShape::CompatDefineBot => rustyscript::Module::new(
                crate::shim::MAIN_MODULE,
                format!("{COMPAT_MAIN}{COMPAT_RUNNER}"),
            ),
            LoadShape::CompatClass => rustyscript::Module::new(
                crate::shim::MAIN_MODULE,
                format!("{COMPAT_CLASS_MAIN}{COMPAT_RUNNER}"),
            ),
            LoadShape::Reject => unreachable!("rejected above"),
        };
        // Side modules load in order, so the shim modules (which the bot
        // imports) must precede the bot's own module.
        let mut side = crate::shim::shim_modules();
        for (url, src) in siblings {
            side.push(rustyscript::Module::new(url, src));
        }
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

    /// Compat wrapper: `create()` the bot instance, then the shared compat
    /// runner (onStart once, awaited, then loop/onPaint every tick). The
    /// instance is exposed as `__rs_bot` for probe read-back and the
    /// EventSignal/ignoredRandoms host read (same global the class shape
    /// uses).
    const COMPAT_MAIN: &str = r#"
import bot from './bot.js';
const inst = (bot && typeof bot.create === 'function') ? bot.create() : (bot || null);
globalThis.__rs_bot = inst;
"#;

    /// Compat class wrapper: instantiate the default-export
    /// `LoopingBot`/`TaskBot`/`TreeBot` subclass, then the shared compat
    /// runner. The instance is exposed as `__rs_bot` for probe read-back.
    const COMPAT_CLASS_MAIN: &str = r#"
import bot from './bot.js';
const inst = new bot();
globalThis.__rs_bot = inst;
"#;

    /// The shared compat tick runner (defineBot and class shapes): `onStart`
    /// once (awaited), then `loop()` (awaited), then `onPaint` with the
    /// dummy ctx. `__rs2b0t_tick_async` is async so an Execution wait parks
    /// the whole runner; `__rs_tick` is the synchronous entry the thread
    /// calls (it returns immediately — parked or not), and the wait is
    /// settled by `__rs2b0t_pump` on later posted ticks instead of a
    /// re-entrant `loop()`. Async errors (a cond that throws) land on the
    /// host handle's `lastError` for the thread to log.
    const COMPAT_RUNNER: &str = r#"
globalThis.__rs_tick = (n) => {
    if (!inst) return;
    globalThis.__rs2b0t_tick_async(n).catch((e) => {
        globalThis.__rs2b0t_host.lastError = String((e && e.message) || e);
    });
};
globalThis.__rs2b0t_tick_async = async (n) => {
    globalThis.__rs2b0t_host.tick = n;
    if (!globalThis.__rs2b0t_started) {
        globalThis.__rs2b0t_started = true;
        if (typeof inst.onStart === 'function') { await inst.onStart(); }
    }
    // IPC bus: posted chat_text changed → chat.message { text }.
    const text = (globalThis.__rs2b0t_host.snapshot || {}).chat_text;
    const t = (text == null || text === '') ? '' : String(text);
    if (t !== globalThis.__rs2b0t_last_chat) {
        globalThis.__rs2b0t_last_chat = t;
        const cbs = inst && inst._subs && inst._subs['chat.message'];
        if (t && cbs) {
            const ev = { text: t };
            for (let i = 0; i < cbs.length; i++) {
                try { cbs[i](ev); } catch (_) {}
            }
        }
    }
    if (typeof inst.loop === 'function') { await inst.loop(); }
    if (typeof inst.onPaint === 'function') { inst.onPaint(globalThis.__dummy_ctx); }
};
"#;

    /// Materialise the decoded FlatBuffer snapshot as the JS object the
    /// shim reads (`__rs2b0t_host.snapshot`), merging it onto the last
    /// posted object. A post is a delta: `tick` is always carried, other
    /// fields only when they changed — so an omitted vector must NOT clear
    /// the previous JS rows. Only the fields the buffer carries are
    /// overwritten; the first post (keyframe on Start / isolate spawn)
    /// builds the object and fail-closes the fields the keyframe also
    /// lacks (absent `here`, empty rows, false flags), exactly like the
    /// old JSON blob. The object is built on the isolate thread directly
    /// from the buffer (v8 object construction — not `JSON.parse`): a wall
    /// of 50+ isolates never parses a JSON document per tick.
    fn materialize_snapshot(
        runtime: &mut Runtime,
        snap: &crate::isolate_fb::SnapshotReader<'_>,
        host_hold: bool,
    ) -> Result<(), String> {
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let host_key = js_string(&mut scope, "__rs2b0t_host")?;
        let host = global
            .get(&mut scope, host_key)
            .ok_or_else(|| "no __rs2b0t_host global".to_string())?
            .to_object(&mut scope)
            .ok_or_else(|| "__rs2b0t_host is not an object".to_string())?;

        let snap_key = js_string(&mut scope, "snapshot")?;
        let existing = host.get(&mut scope, snap_key);
        let had = existing.is_some_and(|v| v.is_object());
        let obj = if had {
            existing
                .expect("checked above")
                .to_object(&mut scope)
                .ok_or_else(|| "snapshot is not an object".to_string())?
        } else {
            v8::Object::new(&mut scope)
        };
        // The fail-closed defaults a keyframe's absent fields materialise
        // to (the same values the shim's `snap()` reads with no snapshot).
        let empty_rows: v8::Local<v8::Value> = v8::Array::new(&mut scope, 0).into();
        let none: v8::Local<v8::Value> = v8::null(&mut scope).into();

        // `tick` is always carried. A field the buffer carries overwrites
        // the object; a field a delta omits keeps its last value. On the
        // keyframe (`had` is false) an absent field fail-closes to the
        // same value the shim's `snap()` reads without a snapshot.
        let tick = num(&mut scope, snap.tick() as f64);
        set(&mut scope, obj, "tick", tick)?;
        let falsy: v8::Local<v8::Value> = v8::Boolean::new(&mut scope, false).into();
        if snap.has_here() {
            let here = match snap.here() {
                Some(tile) => tile_object(&mut scope, &tile)?,
                None => v8::null(&mut scope).into(),
            };
            set(&mut scope, obj, "here", here)?;
            // The reader adapter's `worldTile` reads the host handle
            // directly (not the snapshot blob): mirror `here` there.
            set(&mut scope, host, "tile", here)?;
        } else if !had {
            set(&mut scope, obj, "here", none)?;
        }
        // The reader adapter's `inventorySize` reads the host handle too:
        // mirror the inv tab slot count (0 while the inv tab is
        // tutorial-locked — the gate an onStart waits on).
        if snap.has_inv_size() {
            let inv_size = num(&mut scope, snap.inv_size() as f64);
            set(&mut scope, host, "invSize", inv_size)?;
            set(&mut scope, obj, "inv_size", inv_size)?;
        }
        if snap.has_ingame() {
            let ingame = v8::Boolean::new(&mut scope, snap.ingame());
            set(&mut scope, obj, "ingame", ingame.into())?;
        } else if !had {
            set(&mut scope, obj, "ingame", falsy)?;
        }
        if snap.has_inv() {
            let inv = row_array(&mut scope, &snap.inv())?;
            set(&mut scope, obj, "inv", inv)?;
        } else if !had {
            set(&mut scope, obj, "inv", empty_rows)?;
        }
        if snap.has_stats() {
            let stats = stat_array(&mut scope, &snap.stats())?;
            set(&mut scope, obj, "stats", stats)?;
        } else if !had {
            set(&mut scope, obj, "stats", empty_rows)?;
        }
        if snap.has_booths() {
            let booths = tile_array(&mut scope, &snap.booths())?;
            set(&mut scope, obj, "booths", booths)?;
        } else if !had {
            set(&mut scope, obj, "booths", empty_rows)?;
        }
        if snap.has_nearest_booth() {
            let nb = nearest_booth_object(&mut scope, &snap.nearest_booth().expect("has flag"))?;
            set(&mut scope, obj, "nearest_booth", nb)?;
        } else if !had {
            set(&mut scope, obj, "nearest_booth", none)?;
        }
        if snap.has_banks() {
            let banks = bank_stand_array(&mut scope, &snap.banks())?;
            set(&mut scope, obj, "banks", banks)?;
        } else if !had {
            set(&mut scope, obj, "banks", empty_rows)?;
        }
        if snap.has_bank() {
            let bank = row_array(&mut scope, &snap.bank())?;
            set(&mut scope, obj, "bank", bank)?;
        } else if !had {
            set(&mut scope, obj, "bank", empty_rows)?;
        }
        if snap.has_bank_side() {
            let bank_side = row_array(&mut scope, &snap.bank_side())?;
            set(&mut scope, obj, "bank_side", bank_side)?;
        } else if !had {
            set(&mut scope, obj, "bank_side", empty_rows)?;
        }
        if snap.has_bank_open() {
            let bank_open = v8::Boolean::new(&mut scope, snap.bank_open());
            set(&mut scope, obj, "bank_open", bank_open.into())?;
        } else if !had {
            set(&mut scope, obj, "bank_open", falsy)?;
        }
        if snap.has_bank_loaded() {
            let bank_loaded = v8::Boolean::new(&mut scope, snap.bank_loaded());
            set(&mut scope, obj, "bank_loaded", bank_loaded.into())?;
        } else if !had {
            set(&mut scope, obj, "bank_loaded", falsy)?;
        }
        if snap.has_bank_note_on() {
            let bank_note_on = num(&mut scope, snap.bank_note_on() as f64);
            set(&mut scope, obj, "bank_note_on", bank_note_on)?;
        } else if !had {
            let bank_note_on = num(&mut scope, -1.0);
            set(&mut scope, obj, "bank_note_on", bank_note_on)?;
        }
        if snap.has_bank_note_off() {
            let bank_note_off = num(&mut scope, snap.bank_note_off() as f64);
            set(&mut scope, obj, "bank_note_off", bank_note_off)?;
        } else if !had {
            let bank_note_off = num(&mut scope, -1.0);
            set(&mut scope, obj, "bank_note_off", bank_note_off)?;
        }
        if snap.has_hold() {
            let hold = v8::Boolean::new(&mut scope, snap.hold());
            set(&mut scope, obj, "hold", hold.into())?;
        } else if !had {
            set(&mut scope, obj, "hold", falsy)?;
        }
        // Mirror the host-owned gate onto `__rs2b0t_host.hold` every post
        // (hold is re-posted every tick — SEC-004). JS writes cannot
        // unfreeze; tick_loop gates on `host_hold`, not this property.
        let hold_host = v8::Boolean::new(&mut scope, host_hold);
        set(&mut scope, host, "hold", hold_host.into())?;
        if snap.has_ours() {
            let ours = v8::Boolean::new(&mut scope, snap.ours());
            set(&mut scope, obj, "ours", ours.into())?;
        } else if !had {
            set(&mut scope, obj, "ours", falsy)?;
        }
        if snap.has_npcs() {
            let npcs = scene_entity_array(&mut scope, &snap.npcs())?;
            set(&mut scope, obj, "npcs", npcs)?;
        } else if !had {
            set(&mut scope, obj, "npcs", empty_rows)?;
        }
        if snap.has_locs() {
            let locs = scene_entity_array(&mut scope, &snap.locs())?;
            set(&mut scope, obj, "locs", locs)?;
        } else if !had {
            set(&mut scope, obj, "locs", empty_rows)?;
        }
        if snap.has_players() {
            let players = scene_entity_array(&mut scope, &snap.players())?;
            set(&mut scope, obj, "players", players)?;
        } else if !had {
            set(&mut scope, obj, "players", empty_rows)?;
        }
        if snap.has_ground() {
            let ground = scene_entity_array(&mut scope, &snap.ground())?;
            set(&mut scope, obj, "ground", ground)?;
        } else if !had {
            set(&mut scope, obj, "ground", empty_rows)?;
        }
        if snap.has_equipment() {
            let equipment = row_array(&mut scope, &snap.equipment())?;
            set(&mut scope, obj, "equipment", equipment)?;
        } else if !had {
            set(&mut scope, obj, "equipment", empty_rows)?;
        }
        if snap.has_chat_open() {
            let chat_open = v8::Boolean::new(&mut scope, snap.chat_open());
            set(&mut scope, obj, "chat_open", chat_open.into())?;
        } else if !had {
            set(&mut scope, obj, "chat_open", falsy)?;
        }
        if snap.has_chat_continue() {
            let chat_continue = v8::Boolean::new(&mut scope, snap.chat_continue());
            set(&mut scope, obj, "chat_continue", chat_continue.into())?;
        } else if !had {
            set(&mut scope, obj, "chat_continue", falsy)?;
        }
        if snap.has_chat_text() {
            let chat_text = match snap.chat_text() {
                Some("") | None => v8::null(&mut scope).into(),
                Some(s) => js_string(&mut scope, s)?,
            };
            set(&mut scope, obj, "chat_text", chat_text)?;
        } else if !had {
            set(&mut scope, obj, "chat_text", none)?;
        }
        if snap.has_chat_options() {
            let chat_options = chat_option_array(&mut scope, &snap.chat_options())?;
            set(&mut scope, obj, "chat_options", chat_options)?;
        } else if !had {
            set(&mut scope, obj, "chat_options", empty_rows)?;
        }
        if snap.has_side_tab() {
            let side_tab = num(&mut scope, snap.side_tab() as f64);
            set(&mut scope, obj, "side_tab", side_tab)?;
        } else if !had {
            let neg = num(&mut scope, -1.0);
            set(&mut scope, obj, "side_tab", neg)?;
        }
        if snap.has_varps() {
            let varps = varp_array(&mut scope, &snap.varps())?;
            set(&mut scope, obj, "varps", varps)?;
        } else if !had {
            set(&mut scope, obj, "varps", empty_rows)?;
        }
        if snap.has_combat_styles() {
            let combat_styles = combat_style_array(&mut scope, &snap.combat_styles())?;
            set(&mut scope, obj, "combat_styles", combat_styles)?;
        } else if !had {
            set(&mut scope, obj, "combat_styles", empty_rows)?;
        }
        if snap.has_run_energy() {
            let run_energy = num(&mut scope, snap.run_energy() as f64);
            set(&mut scope, obj, "run_energy", run_energy)?;
        } else if !had {
            let zero = num(&mut scope, 0.0);
            set(&mut scope, obj, "run_energy", zero)?;
        }
        if snap.has_run_enabled() {
            let run_enabled = v8::Boolean::new(&mut scope, snap.run_enabled());
            set(&mut scope, obj, "run_enabled", run_enabled.into())?;
        } else if !had {
            set(&mut scope, obj, "run_enabled", falsy)?;
        }
        if snap.has_retaliate_enabled() {
            let retaliate_enabled = v8::Boolean::new(&mut scope, snap.retaliate_enabled());
            set(
                &mut scope,
                obj,
                "retaliate_enabled",
                retaliate_enabled.into(),
            )?;
        } else if !had {
            set(&mut scope, obj, "retaliate_enabled", falsy)?;
        }
        if snap.has_my_name() {
            let my_name = match snap.my_name() {
                Some("") | None => v8::null(&mut scope).into(),
                Some(s) => js_string(&mut scope, s)?,
            };
            set(&mut scope, obj, "my_name", my_name)?;
        } else if !had {
            set(&mut scope, obj, "my_name", none)?;
        }
        if snap.has_in_combat() {
            let in_combat = v8::Boolean::new(&mut scope, snap.in_combat());
            set(&mut scope, obj, "in_combat", in_combat.into())?;
        } else if !had {
            set(&mut scope, obj, "in_combat", falsy)?;
        }
        if snap.has_animating() {
            let animating = v8::Boolean::new(&mut scope, snap.animating());
            set(&mut scope, obj, "animating", animating.into())?;
        } else if !had {
            set(&mut scope, obj, "animating", falsy)?;
        }
        if snap.has_main_modal_id() {
            let main_modal_id = num(&mut scope, snap.main_modal_id() as f64);
            set(&mut scope, obj, "main_modal_id", main_modal_id)?;
        } else if !had {
            let neg = num(&mut scope, -1.0);
            set(&mut scope, obj, "main_modal_id", neg)?;
        }
        if snap.has_chat_modal_id() {
            let chat_modal_id = num(&mut scope, snap.chat_modal_id() as f64);
            set(&mut scope, obj, "chat_modal_id", chat_modal_id)?;
        } else if !had {
            let neg = num(&mut scope, -1.0);
            set(&mut scope, obj, "chat_modal_id", neg)?;
        }
        if snap.has_make_products() {
            let make_products = make_product_array(&mut scope, &snap.make_products())?;
            set(&mut scope, obj, "make_products", make_products)?;
        } else if !had {
            set(&mut scope, obj, "make_products", empty_rows)?;
        }
        if snap.has_side_tab_ifaces() {
            let ifaces = side_tab_iface_array(&mut scope, &snap.side_tab_ifaces())?;
            set(&mut scope, obj, "side_tab_ifaces", ifaces)?;
        } else if !had {
            set(&mut scope, obj, "side_tab_ifaces", empty_rows)?;
        }
        if snap.has_spell_buttons() {
            let spell_buttons = combat_style_array(&mut scope, &snap.spell_buttons())?;
            set(&mut scope, obj, "spell_buttons", spell_buttons)?;
        } else if !had {
            set(&mut scope, obj, "spell_buttons", empty_rows)?;
        }
        if snap.has_chat_lines() {
            let chat_lines = chat_line_array(&mut scope, &snap.chat_lines())?;
            set(&mut scope, obj, "chat_lines", chat_lines)?;
        } else if !had {
            set(&mut scope, obj, "chat_lines", empty_rows)?;
        }
        let snapshot = obj.into();
        set(&mut scope, host, "snapshot", snapshot)
    }

    fn materialize_settings_bag(runtime: &mut Runtime, json: &str) -> Result<(), String> {
        runtime
            .eval::<()>(format!("globalThis.__rs2b0t_host.settingsBag = {json};"))
            .map_err(|e| format!("settings bag: {e}"))
    }

    fn js_string<'s>(
        scope: &mut v8::HandleScope<'s>,
        s: &str,
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        v8::String::new(scope, s)
            .map(|v| v.into())
            .ok_or_else(|| "v8 string alloc failed".to_string())
    }

    fn num<'s>(scope: &mut v8::HandleScope<'s>, n: f64) -> v8::Local<'s, v8::Value> {
        v8::Number::new(scope, n).into()
    }

    fn set<'s>(
        scope: &mut v8::HandleScope<'s>,
        obj: v8::Local<'s, v8::Object>,
        key: &str,
        value: v8::Local<'s, v8::Value>,
    ) -> Result<(), String> {
        let key = js_string(scope, key)?;
        obj.set(scope, key, value)
            .ok_or_else(|| format!("v8 object set failed for {key:?}"))?;
        Ok(())
    }

    /// One `{id, name, ops, count, noted, cert}` row from ItemView.
    fn row_object<'s>(
        scope: &mut v8::HandleScope<'s>,
        row: &crate::isolate_fb::RowReader<'_>,
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let o = v8::Object::new(scope);
        match row.name() {
            Some(name) => {
                let name = js_string(scope, name)?;
                set(scope, o, "name", name)?;
            }
            None => {
                let none = v8::null(scope);
                set(scope, o, "name", none.into())?;
            }
        }
        let count = num(scope, row.count() as f64);
        set(scope, o, "count", count)?;
        let id = num(scope, row.id() as f64);
        set(scope, o, "id", id)?;
        let ops = v8::Array::new(scope, row.ops().len() as i32);
        for (i, op) in row.ops().iter().enumerate() {
            let a = js_string(scope, op)?;
            ops.set_index(scope, i as u32, a)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        set(scope, o, "ops", ops.into())?;
        let noted = v8::Boolean::new(scope, row.noted());
        set(scope, o, "noted", noted.into())?;
        let cert = num(scope, row.cert() as f64);
        set(scope, o, "cert", cert)?;
        Ok(o.into())
    }

    fn row_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        rows: &[crate::isolate_fb::RowReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, rows.len() as i32);
        for (i, row) in rows.iter().enumerate() {
            let row = row_object(scope, row)?;
            arr.set_index(scope, i as u32, row)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn stat_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        stats: &[crate::isolate_fb::StatReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, stats.len() as i32);
        for (i, st) in stats.iter().enumerate() {
            let o = v8::Object::new(scope);
            let index = num(scope, st.index() as f64);
            set(scope, o, "index", index)?;
            let name = js_string(scope, st.name())?;
            set(scope, o, "name", name)?;
            let xp = num(scope, st.xp() as f64);
            set(scope, o, "xp", xp)?;
            let level = num(scope, st.level() as f64);
            set(scope, o, "level", level)?;
            let effective = num(scope, st.level() as f64);
            set(scope, o, "effective", effective)?;
            let obj = o.into();
            arr.set_index(scope, i as u32, obj)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn tile_object<'s>(
        scope: &mut v8::HandleScope<'s>,
        t: &crate::isolate_fb::TileReader<'_>,
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let o = v8::Object::new(scope);
        let x = num(scope, t.x() as f64);
        set(scope, o, "x", x)?;
        let z = num(scope, t.z() as f64);
        set(scope, o, "z", z)?;
        let level = num(scope, t.level() as f64);
        set(scope, o, "level", level)?;
        Ok(o.into())
    }

    fn nearest_booth_object<'s>(
        scope: &mut v8::HandleScope<'s>,
        nb: &crate::isolate_fb::NearestBoothReader<'_>,
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let o = v8::Object::new(scope);
        let x = num(scope, nb.x() as f64);
        set(scope, o, "x", x)?;
        let z = num(scope, nb.z() as f64);
        set(scope, o, "z", z)?;
        let level = num(scope, nb.level() as f64);
        set(scope, o, "level", level)?;
        let name = js_string(scope, nb.name())?;
        set(scope, o, "name", name)?;
        let op = js_string(scope, nb.op())?;
        set(scope, o, "op", op)?;
        Ok(o.into())
    }

    fn tile_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        tiles: &[crate::isolate_fb::TileReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, tiles.len() as i32);
        for (i, t) in tiles.iter().enumerate() {
            let t = tile_object(scope, t)?;
            arr.set_index(scope, i as u32, t)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn scene_entity_object<'s>(
        scope: &mut v8::HandleScope<'s>,
        ent: &crate::isolate_fb::SceneEntityReader<'_>,
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let o = v8::Object::new(scope);
        let index = num(scope, ent.index() as f64);
        set(scope, o, "index", index)?;
        let id = num(scope, ent.id() as f64);
        set(scope, o, "id", id)?;
        match ent.name() {
            Some(name) => {
                let name = js_string(scope, name)?;
                set(scope, o, "name", name)?;
            }
            None => {
                let none = v8::null(scope);
                set(scope, o, "name", none.into())?;
            }
        }
        let x = num(scope, ent.x() as f64);
        set(scope, o, "x", x)?;
        let z = num(scope, ent.z() as f64);
        set(scope, o, "z", z)?;
        let level = num(scope, ent.level() as f64);
        set(scope, o, "level", level)?;
        let distance = num(scope, ent.distance() as f64);
        set(scope, o, "distance", distance)?;
        let health = num(scope, ent.health() as f64);
        set(scope, o, "health", health)?;
        let max_health = num(scope, ent.max_health() as f64);
        set(scope, o, "max_health", max_health)?;
        let in_combat = v8::Boolean::new(scope, ent.in_combat());
        set(scope, o, "in_combat", in_combat.into())?;
        let animating = v8::Boolean::new(scope, ent.animating());
        set(scope, o, "animating", animating.into())?;
        let actions = v8::Array::new(scope, ent.actions().len() as i32);
        for (i, action) in ent.actions().iter().enumerate() {
            let a = js_string(scope, action)?;
            actions
                .set_index(scope, i as u32, a)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        set(scope, o, "actions", actions.into())?;
        let reachable = v8::Boolean::new(scope, ent.reachable());
        set(scope, o, "reachable", reachable.into())?;
        let reachable_adj = v8::Boolean::new(scope, ent.reachable_adj());
        set(scope, o, "reachable_adj", reachable_adj.into())?;
        Ok(o.into())
    }

    fn scene_entity_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        ents: &[crate::isolate_fb::SceneEntityReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, ents.len() as i32);
        for (i, ent) in ents.iter().enumerate() {
            let ent = scene_entity_object(scope, ent)?;
            arr.set_index(scope, i as u32, ent)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn chat_option_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        opts: &[crate::isolate_fb::ChatOptionReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, opts.len() as i32);
        for (i, opt) in opts.iter().enumerate() {
            let o = v8::Object::new(scope);
            let text = js_string(scope, opt.text())?;
            set(scope, o, "text", text)?;
            let obj = o.into();
            arr.set_index(scope, i as u32, obj)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn make_product_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        products: &[crate::isolate_fb::MakeProductReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, products.len() as i32);
        for (i, product) in products.iter().enumerate() {
            let o = v8::Object::new(scope);
            let name = js_string(scope, product.name())?;
            set(scope, o, "name", name)?;
            let oid = num(scope, product.object_id() as f64);
            set(scope, o, "object_id", oid)?;
            let buttons = v8::Array::new(scope, product.buttons().len() as i32);
            for (j, btn) in product.buttons().iter().enumerate() {
                let b = v8::Object::new(scope);
                let qty = num(scope, btn.qty() as f64);
                set(scope, b, "qty", qty)?;
                let com_id = num(scope, btn.com_id() as f64);
                set(scope, b, "comId", com_id)?;
                buttons
                    .set_index(scope, j as u32, b.into())
                    .ok_or_else(|| "v8 array set failed".to_string())?;
            }
            set(scope, o, "buttons", buttons.into())?;
            arr.set_index(scope, i as u32, o.into())
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn varp_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        varps: &[crate::isolate_fb::VarpReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, varps.len() as i32);
        for (i, v) in varps.iter().enumerate() {
            let o = v8::Object::new(scope);
            let index = num(scope, v.index() as f64);
            set(scope, o, "index", index)?;
            let value = num(scope, v.value() as f64);
            set(scope, o, "value", value)?;
            let obj = o.into();
            arr.set_index(scope, i as u32, obj)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn combat_style_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        styles: &[crate::isolate_fb::CombatStyleReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, styles.len() as i32);
        for (i, st) in styles.iter().enumerate() {
            let o = v8::Object::new(scope);
            let mode = num(scope, st.mode() as f64);
            set(scope, o, "mode", mode)?;
            let label = js_string(scope, st.label())?;
            set(scope, o, "label", label)?;
            let component_id = num(scope, st.component_id() as f64);
            set(scope, o, "component_id", component_id)?;
            let obj = o.into();
            arr.set_index(scope, i as u32, obj)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn side_tab_iface_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        tabs: &[crate::isolate_fb::SideTabIfaceReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, tabs.len() as i32);
        for (i, t) in tabs.iter().enumerate() {
            let o = v8::Object::new(scope);
            let index = num(scope, t.index() as f64);
            set(scope, o, "index", index)?;
            let id = num(scope, t.id() as f64);
            set(scope, o, "id", id)?;
            arr.set_index(scope, i as u32, o.into())
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn chat_line_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        lines: &[crate::isolate_fb::ChatLineReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, lines.len() as i32);
        for (i, line) in lines.iter().enumerate() {
            let o = v8::Object::new(scope);
            let seq = num(scope, line.seq() as f64);
            set(scope, o, "seq", seq)?;
            let text = js_string(scope, line.text())?;
            set(scope, o, "text", text)?;
            arr.set_index(scope, i as u32, o.into())
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    fn bank_stand_array<'s>(
        scope: &mut v8::HandleScope<'s>,
        stands: &[crate::isolate_fb::BankStandReader<'_>],
    ) -> Result<v8::Local<'s, v8::Value>, String> {
        let arr = v8::Array::new(scope, stands.len() as i32);
        for (i, s) in stands.iter().enumerate() {
            let o = v8::Object::new(scope);
            let name = js_string(scope, s.name())?;
            set(scope, o, "name", name)?;
            let x = num(scope, s.x() as f64);
            set(scope, o, "x", x)?;
            let z = num(scope, s.z() as f64);
            set(scope, o, "z", z)?;
            let level = num(scope, s.level() as f64);
            set(scope, o, "level", level)?;
            let kind = js_string(scope, s.kind())?;
            set(scope, o, "kind", kind)?;
            let op = num(scope, s.op() as f64);
            set(scope, o, "op", op)?;
            match s.choose() {
                Some(choose) => {
                    let choose = js_string(scope, choose)?;
                    set(scope, o, "choose", choose)?;
                }
                None => {
                    let none = v8::null(scope);
                    set(scope, o, "choose", none.into())?;
                }
            }
            let obj = o.into();
            arr.set_index(scope, i as u32, obj)
                .ok_or_else(|| "v8 array set failed".to_string())?;
        }
        Ok(arr.into())
    }

    /// Forward a paint frame to the host when it differs from the last one
    /// sent on this isolate thread (skips FlatBuffer encode on quiet ticks).
    fn forward_paint_if_changed(
        ipc: &mut crate::isolate_fb::IsolateBuf,
        out: &Sender<ThreadMsg>,
        last: &mut Option<crate::shim::ScriptPaint>,
        frame: crate::shim::ScriptPaint,
    ) {
        if last.as_ref() == Some(&frame) {
            return;
        }
        *last = Some(frame.clone());
        let _ = out.send(ThreadMsg::Paint(ipc.encode_paint(&frame)));
    }

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
        // Host-owned hold gate (SEC-004): set from the posted FlatBuffer
        // snapshot, never from a JS-writable `__rs2b0t_host.hold`.
        let mut host_hold = false;
        // One reusable encode buffer for this V8 isolate: interact batch
        // and paint frames share it (`reset` between messages).
        let mut ipc = crate::isolate_fb::IsolateBuf::new();
        let mut last_forwarded_paint: Option<crate::shim::ScriptPaint> = None;
        loop {
            let cmd = match pending.take() {
                Some(cmd) => cmd,
                None => match cmds.recv() {
                    Ok(cmd) => cmd,
                    Err(_) => break,
                },
            };
            match cmd {
                IsolateCmd::Snapshot(bytes) => {
                    // Decode the posted FlatBuffer and materialise the JS
                    // object the shim reads on the host handle. A
                    // malformed blob is logged, never fatal.
                    match crate::isolate_fb::SnapshotReader::from_bytes(&bytes) {
                        Ok(snap) => {
                            if snap.has_hold() {
                                host_hold = snap.hold();
                            }
                            if let Err(e) = materialize_snapshot(&mut runtime, &snap, host_hold) {
                                let _ = out.send(ThreadMsg::Log(format!("snapshot: {e}")));
                            }
                        }
                        Err(e) => {
                            let _ = out.send(ThreadMsg::Log(format!("snapshot: {e}")));
                        }
                    }
                }
                IsolateCmd::Settings(json) => {
                    if let Err(e) = materialize_settings_bag(&mut runtime, &json) {
                        let _ = out.send(ThreadMsg::Log(format!("settings: {e}")));
                    }
                }
                IsolateCmd::Tick(n) => {
                    if paused {
                        continue;
                    }
                    let start = Instant::now();
                    // Guardian hold: skip `loop()` AND skip resolving
                    // parked conds (time waits too) — the wait stays parked
                    // until the hold lifts. Still call `onPaint` so status
                    // rows keep updating. Pause already freezes above.
                    if host_hold {
                        // Paint-only tick: no loop, no pump. Use `__rs_bot`
                        // (global); module-local `inst` is not visible here.
                        let _ = runtime.eval::<()>(&format!("globalThis.__rs2b0t_host.tick = {n}"));
                        let _ = runtime.eval::<()>(
                            "(() => { const bot = globalThis.__rs_bot; if (bot && typeof bot.onPaint === 'function') { bot.onPaint(globalThis.__dummy_ctx); } })()",
                        );
                        let _ = runtime.block_on_event_loop(
                            rustyscript::deno_core::PollEventLoopOptions::default(),
                            Some(Duration::from_millis(10)),
                        );
                        // Forward paint the same as a normal tick.
                        let paint: Result<Option<crate::shim::ScriptPaint>, rustyscript::Error> =
                            runtime.eval("globalThis.__rs2b0t_host.paint || null");
                        if let Ok(Some(frame)) = paint {
                            forward_paint_if_changed(
                                &mut ipc,
                                &out,
                                &mut last_forwarded_paint,
                                frame,
                            );
                        }
                        let _ = out.send(ThreadMsg::IgnoredRandoms(eval_ignored_randoms(
                            &mut runtime,
                        )));
                        let _ = out.send(ThreadMsg::Completed(n));
                        continue;
                    }
                    // A parked Execution wait: settle it (cond / due tick /
                    // due time) so the loop's continuation runs — never call
                    // `loop()` again while parked. Otherwise start a fresh
                    // tick. `__rs2b0t_pump` is async and awaited through the
                    // event loop, so the resolved wait's continuation (which
                    // may re-park or complete the tick) lands here.
                    let parked = runtime
                        .eval::<bool>(
                            "!!(globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.parked)",
                        )
                        .unwrap_or(false);
                    let result: Result<(), rustyscript::Error> = if parked {
                        runtime.call_function(None, "__rs2b0t_pump", json_args!(n))
                    } else {
                        // `__rs_tick` is a synchronous entry that returns
                        // immediately (parked or not), so this cannot hang on
                        // a wait. Drain microtasks so the runner's await
                        // continuations and onPaint land inside this tick; a
                        // parked wait leaves no pending work, so the drain
                        // returns at once (the timeout is only a backstop).
                        let result =
                            runtime.call_function_immediate(None, "__rs_tick", json_args!(n));
                        let _ = runtime.block_on_event_loop(
                            rustyscript::deno_core::PollEventLoopOptions::default(),
                            Some(Duration::from_millis(10)),
                        );
                        result
                    };
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
                    // Async errors (a cond that throws, a rejected wait)
                    // surface on the runner's catch instead of throwing the
                    // tick; fold them into the log like sync tick errors.
                    let async_err: Option<String> = runtime
                        .eval("(() => { const e = globalThis.__rs2b0t_host.lastError; if (e) { globalThis.__rs2b0t_host.lastError = null; return e; } return null; })()")
                        .unwrap_or(None);
                    if let Some(e) = async_err {
                        let _ = out.send(ThreadMsg::Log(format!("tick {n}: {e}")));
                    }
                    // `LoopingBot.log` / `this.log` push onto the host
                    // handle; fold them into the isolate log so BOT_DEBUG
                    // and the panel can see script-side lines.
                    let bot_log: Result<Vec<String>, rustyscript::Error> =
                        runtime.eval("(() => { const h = globalThis.__rs2b0t_host; const rows = h && h.log; if (!Array.isArray(rows) || rows.length === 0) return []; h.log = []; return rows.map(String); })()");
                    if let Ok(rows) = bot_log {
                        for line in rows {
                            let _ = out.send(ThreadMsg::Log(line));
                        }
                    }
                    // Forward the tick's shim interact queue (Bank/Banking
                    // requests written to `__rs2b0t_host.interact`) to the
                    // host, then clear it for the next tick. The queue is
                    // evaluated only now, after the tick's JS (and any
                    // parked continuation) has fully run, so a request
                    // reaches the host exactly once. The queue is read
                    // through the runtime's value bridge (v8 object walk,
                    // not `JSON.parse`) and forwarded as a FlatBuffer
                    // batch, not a stringified JSON document.
                    let interact: Result<Vec<crate::shim::InteractReq>, rustyscript::Error> =
                        runtime.eval("globalThis.__rs2b0t_host.interact || []");
                    if let Ok(reqs) = interact {
                        if !reqs.is_empty() {
                            let _ = out.send(ThreadMsg::Interact(ipc.encode_interact_batch(&reqs)));
                        }
                    }
                    let _ = runtime.eval::<()>("globalThis.__rs2b0t_host.interact = []");
                    // Forward the tick's recorded paint frame
                    // (`Paint.begin` … `end()` on the host handle) to the
                    // host, so the script paint views read it without a
                    // probe round-trip. Only non-empty frames are sent —
                    // a tick that painted nothing leaves the last frame
                    // in place (Stop drops the whole isolate). serde_v8
                    // walks the v8 object into `ScriptPaint`; the channel
                    // carries a FlatBuffer, never a `serde_json::Value`.
                    let paint: Result<Option<crate::shim::ScriptPaint>, rustyscript::Error> =
                        runtime.eval("globalThis.__rs2b0t_host.paint || null");
                    if let Ok(Some(frame)) = paint {
                        forward_paint_if_changed(&mut ipc, &out, &mut last_forwarded_paint, frame);
                    }
                    let _ = out.send(ThreadMsg::IgnoredRandoms(eval_ignored_randoms(
                        &mut runtime,
                    )));
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
