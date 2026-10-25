//! JS card library: picker cards, catalog apply, and prepare/commit.

use std::collections::{HashMap, HashSet};
#[path = "library/catalog.rs"]
mod catalog;

pub use catalog::{
    CatalogApplyReport, CatalogDiff, CatalogPreparationResult, CatalogPreparationSnapshot,
};

pub use catalog::{CatalogSnapshotError, CatalogWorkerError};

use std::path::{Path, PathBuf};

use crate::js_cache::{default_js_cache_root, CacheMeta, CachedJs, JsCache};

use crate::rs2b0t_registry::{
    absolute_clean, parse_registry_with_sources, persist_rs2b0t_root_at, script_file_path,
};
use crate::rs2b0t_registry::{ScriptKind, ScriptSource, SettingDef};

use super::shape::{
    catalog_unloadable, first_unloadable_for_card, first_unloadable_for_card_bounded, is_reserved,
    raw_content_fingerprint, raw_content_fingerprint_bounded, resolve_api_family,
    resolve_sibling_modules_bounded, ApiFamily, FirstUnloadableWork, LoadShape,
    PreparationFingerprintWork, SiblingCompilationWork, MAX_PREPARATION_RESULT_BYTES,
    MAX_PREPARATION_SIBLINGS, MAX_PREPARATION_SOURCE_BYTES, MAX_SOURCE_BYTES,
};

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
    /// First import specifier that does not remap to a registered shim.
    /// `None` means Start may spawn; `not impl` members do not set this.
    pub unloadable: Option<String>,
    /// Provenance only — not part of identity or the cache key.
    pub api_family: ApiFamily,
}

/// Stage of a retained per-source load failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStage {
    Read,
    ParseTranspile,
    ImportResolution,
    RuntimeLoad,
}

impl LoadStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ParseTranspile => "parse/transpile",
            Self::ImportResolution => "import",
            Self::RuntimeLoad => "runtime-load",
        }
    }
}

/// One inspectable load/transpile/import/runtime-load failure.
/// Keyed by stable identity (path for files, register name for catalog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadFailure {
    pub identity_key: String,
    pub name: String,
    pub path: PathBuf,
    pub stage: LoadStage,
    pub diagnostic: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub sibling: Option<String>,
    pub fingerprint: String,
    pub api_family: Option<ApiFamily>,
}

impl LoadFailure {
    pub fn capture(
        source: ScriptSource,
        path: &Path,
        name: &str,
        diagnostic: &str,
        origin: Option<&str>,
        api_family: Option<ApiFamily>,
    ) -> Self {
        let fingerprint = origin
            .map(|text| raw_content_fingerprint(path, text))
            .unwrap_or_default();
        Self::capture_with_fingerprint(source, path, name, diagnostic, fingerprint, api_family)
    }

    fn capture_bounded(
        source: ScriptSource,
        path: &Path,
        name: &str,
        diagnostic: &str,
        origin: &str,
        api_family: Option<ApiFamily>,
        source_limit: usize,
    ) -> Self {
        let fingerprint =
            raw_content_fingerprint_bounded(path, origin, source_limit).unwrap_or_default();
        Self::capture_with_fingerprint(source, path, name, diagnostic, fingerprint, api_family)
    }

    fn capture_with_fingerprint(
        source: ScriptSource,
        path: &Path,
        name: &str,
        diagnostic: &str,
        fingerprint: String,
        api_family: Option<ApiFamily>,
    ) -> Self {
        let (line, column) = extract_line_column(diagnostic);
        let sibling = extract_sibling(diagnostic);
        let stage = classify_stage(diagnostic, sibling.as_deref());
        Self {
            identity_key: script::identity::card_identity_key(source, path, name),
            name: name.to_string(),
            path: path.to_path_buf(),
            stage,
            diagnostic: diagnostic.to_string(),
            line,
            column,
            sibling,
            fingerprint,
            api_family,
        }
    }

    pub fn named_line(&self) -> String {
        let loc = match (self.line, self.column) {
            (Some(line), Some(column)) => format!(":{line}:{column}"),
            (Some(line), None) => format!(":{line}"),
            _ => String::new(),
        };
        let sibling = self
            .sibling
            .as_deref()
            .map(|s| format!(" sibling {s}"))
            .unwrap_or_default();
        format!(
            "{} ({}) {}{}{}: {}",
            self.name,
            self.path.display(),
            self.stage.as_str(),
            loc,
            sibling,
            self.diagnostic
        )
    }

    pub fn copyable_text(&self) -> String {
        let mut out = self.named_line();
        if let Some(family) = self.api_family {
            out.push_str(&format!("\napi_family: {}", family.as_str()));
        }
        out.push_str(&format!("\nidentity: {}", self.identity_key));
        if !self.fingerprint.is_empty() {
            out.push_str(&format!("\nfingerprint: {}", self.fingerprint));
        }
        out
    }
}

/// Readable/copyable named list for panel and TUI.
pub fn format_load_failures<'a, I>(failures: I) -> String
where
    I: IntoIterator<Item = &'a LoadFailure>,
{
    failures
        .into_iter()
        .map(LoadFailure::named_line)
        .collect::<Vec<_>>()
        .join("\n")
}

impl JsCard {
    pub fn identity_id(&self) -> String {
        script::identity::card_identity_id(self.source, &self.path, &self.name)
    }

    pub fn identity_key(&self) -> String {
        script::identity::card_identity_key(self.source, &self.path, &self.name)
    }

    pub fn assignment(&self) -> vault::ScriptAssignment {
        script::identity::card_assignment(self.source, &self.path, &self.name)
    }
}

/// Default persisted library path (`~/.274bot/js-scripts.json`).
pub fn default_js_store() -> PathBuf {
    script::bot_file("js-scripts.json")
}

/// One persisted library record: only the name and the source path (the
/// source itself is re-read from disk on restore).

#[derive(serde::Serialize, serde::Deserialize)]
struct StoreEntry {
    name: String,
    path: String,
}

/// Bound on the short persisted card list.
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;

/// A stored entry path is restorable only when absolute (a relative one would
/// resolve against wherever the host happens to be started) and free of `..`.
fn restorable_path(stored: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(stored);
    if !path.is_absolute() {
        return Err("not an absolute path".into());
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("the path contains `..`".into());
    }
    Ok(path)
}

enum StoredSource {
    Text(String),
    /// Missing or unreadable: dropped quietly, as before.
    Gone,
    Refused(String),
}

/// Reads a script source the store names. Symlinks are followed and the checks
/// apply to what they resolve to, on one open file ([`vault::read_regular_file`]):
/// a regular file within [`MAX_SOURCE_BYTES`] (a FIFO or device would hang the
/// read or never end). There is no permission check on purpose: sources live
/// wherever the operator keeps them, including filesystems that report every
/// file world-writable.
fn read_stored_source(path: &Path) -> StoredSource {
    match vault::read_regular_file(path, MAX_SOURCE_BYTES) {
        Ok(text) => StoredSource::Text(text),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            StoredSource::Refused(e.to_string())
        }
        Err(_) => StoredSource::Gone,
    }
}

/// The out-of-tree JS library: picker cards for loaded files and the
/// `$RS2B0T` catalog, persisted to `store`, with origin bytes cached
/// under `cache`. Same `(source, name)` overwrites; only WalkTo is
/// reserved; non-bot shapes are rejected at Load.
pub struct JsLibrary {
    store: PathBuf,
    cache: JsCache,
    cards: Vec<JsCard>,
    /// Registry root used to re-resolve catalog-selected schema exports.
    catalog_root: Option<PathBuf>,
    /// Combined raw entry+sibling hashes keyed by identity, used to skip
    /// no-op reloads without transpiling.
    fingerprints: HashMap<String, String>,
    /// Retained per-identity load failures. One card's success does not
    /// clear another identity's record.
    failures: HashMap<String, LoadFailure>,
    /// Set when [`JsLibrary::restore`] refused the store file. While set,
    /// nothing is saved, so the refused file is never replaced by a library
    /// that could not read it.
    store_refusal: Option<String>,
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
            catalog_root: None,
            fingerprints: HashMap::new(),
            failures: HashMap::new(),
            store_refusal: None,
        }
    }

    /// Read the persisted `{name, path}` list back into cards. Sources are
    /// re-read from disk and re-classified; entries whose file is gone or
    /// that no longer look like a bot are dropped. A missing store is not
    /// an error (first run).
    ///
    /// The store decides which files the host loads, so it is read with
    /// [`vault::read_private_file`]: a store another user could have written
    /// is refused (`Err`, and nothing is saved over it afterwards), and an
    /// entry is restored only when its path is absolute, has no `..` and names
    /// a regular file of sane size. A refused entry is recorded as a load
    /// failure so the operator can see why it did not come back.
    pub fn restore(&mut self) -> Result<(), String> {
        self.store_refusal = None;
        let raw = match vault::read_private_file(&self.store, MAX_STORE_BYTES) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                let error = format!("js-scripts.json: {e}");
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    self.store_refusal = Some(error.clone());
                }
                return Err(error);
            }
        };
        let entries: Vec<StoreEntry> =
            serde_json::from_str(&raw).map_err(|e| format!("js-scripts.json: {e}"))?;
        self.cards.clear();
        for entry in entries {
            let path = match restorable_path(&entry.path) {
                Ok(path) => path,
                Err(reason) => {
                    let diagnostic = format!("not restored: {reason}");
                    let stored = Path::new(&entry.path);
                    self.note_err(
                        ScriptSource::File,
                        stored,
                        &entry.name,
                        &diagnostic,
                        None,
                        None,
                    );
                    continue;
                }
            };
            let origin = match read_stored_source(&path) {
                StoredSource::Text(origin) => origin,
                StoredSource::Gone => continue,
                StoredSource::Refused(reason) => {
                    let diagnostic = format!("not restored: {reason}");
                    self.note_err(
                        ScriptSource::File,
                        &path,
                        &entry.name,
                        &diagnostic,
                        None,
                        None,
                    );
                    continue;
                }
            };
            let Ok((shape, api_family)) = resolve_api_family(&origin) else {
                continue;
            };
            if shape == LoadShape::Reject {
                continue;
            }
            if is_reserved(&entry.name) {
                continue;
            }
            let cached = match self.cache.get_or_transpile(
                &path,
                origin.as_bytes(),
                cache_meta(shape, ScriptSource::File, api_family),
            ) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let settings_schema =
                match crate::rs2b0t_registry::try_settings_schema_from_source(&origin) {
                    Ok(schema) => schema,
                    Err(reason) => {
                        self.note_err(
                            ScriptSource::File,
                            &path,
                            &entry.name,
                            &reason,
                            Some(&origin),
                            Some(api_family),
                        );
                        continue;
                    }
                };
            let unloadable = first_unloadable_for_card(&origin, &path);
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
                unloadable,
                api_family,
            });
            if let Some(last) = self.cards.last().cloned() {
                self.remember_fingerprint(&last);
            }
        }
        Ok(())
    }

    /// Register a JS bot from a filesystem path. Reads the origin, caches
    /// transpiled JS under `~/.274bot/js-cache`, and statically parses
    /// `export const SETTINGS` for the picker schema. V8 only runs on
    /// Start. Loading the same file path replaces its previous card;
    /// different paths remain distinct even when their file stems match.
    pub fn load(&mut self, path: &Path) -> Result<JsCard, String> {
        // Stored absolute and `..`-free, so a later restore means the same file
        // wherever the host is started from.
        let path = absolute_clean(path).map_err(|e| format!("load {}: {e}", path.display()))?;
        let path = path.as_path();
        match self.load_unrecorded(path) {
            Ok(card) => {
                self.note_card_outcome(&card);
                Ok(card)
            }
            Err(e) => {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let origin = std::fs::read_to_string(path).ok();
                let family = origin
                    .as_deref()
                    .and_then(|text| resolve_api_family(text).ok().map(|(_, family)| family));
                self.note_err(
                    ScriptSource::File,
                    path,
                    &name,
                    &e,
                    origin.as_deref(),
                    family,
                );
                Err(e)
            }
        }
    }

    fn load_unrecorded(&mut self, path: &Path) -> Result<JsCard, String> {
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
        let (shape, api_family) =
            resolve_api_family(&origin).map_err(|e| format!("{name}: {e}"))?;
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        let cached = self
            .cache
            .get_or_transpile(
                path,
                origin.as_bytes(),
                cache_meta(shape, ScriptSource::File, api_family),
            )
            .map_err(|e| format!("{name}: {e}"))?;
        let settings_schema = crate::rs2b0t_registry::try_settings_schema_from_source(&origin)
            .map_err(|e| format!("{name}: {e}"))?;
        let unloadable = first_unloadable_for_card(&origin, path);
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
            unloadable,
            api_family,
        };
        let new_cards: Vec<JsCard> = self
            .cards
            .iter()
            .filter(|c| {
                !(c.source == ScriptSource::File
                    && script::identity::paths_match(&card.path.to_string_lossy(), &c.path))
            })
            .cloned()
            .chain(std::iter::once(card.clone()))
            .collect();
        self.persist_file_cards(&new_cards)?;
        self.cards = new_cards;
        self.remember_fingerprint(&card);
        Ok(card)
    }

    /// All registered cards, in load order (a re-load moves the card to
    /// the back; the picker shows them after the compiled ids).
    pub fn cards(&self) -> &[JsCard] {
        &self.cards
    }

    pub fn load_failures(&self) -> Vec<&LoadFailure> {
        let mut rows: Vec<&LoadFailure> = self.failures.values().collect();
        rows.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.identity_key.cmp(&b.identity_key))
        });
        rows
    }

    pub fn load_failure(&self, identity_key: &str) -> Option<&LoadFailure> {
        self.failures.get(identity_key)
    }

    pub fn named_failure_output(&self) -> String {
        format_load_failures(self.load_failures())
    }

    pub fn record_failure(&mut self, failure: LoadFailure) {
        self.failures.insert(failure.identity_key.clone(), failure);
    }

    /// Retain an initial runtime failure against the exact attempted card bytes.
    /// Refusals leave prior diagnostics untouched; success clears this identity.
    pub fn record_start_result(
        &mut self,
        card: &JsCard,
        result: Result<(), script::StartLoadError>,
    ) -> Result<(), String> {
        match result {
            Ok(()) => {
                self.clear_failure(&card.identity_key());
                Ok(())
            }
            Err(
                script::StartLoadError::RuntimeLoad(diagnostic)
                | script::StartLoadError::Surface(script::surface::SurfaceError::Spawn(diagnostic)),
            ) => {
                let user_diagnostic = missing_compat_api_export(card.source, &diagnostic)
                    .map(|export| {
                        format!(
                            "This script imports compatibility API member '{export}', which this build does not provide."
                        )
                    })
                    .unwrap_or_else(|| diagnostic.clone());
                let mut failure = LoadFailure::capture(
                    card.source,
                    &card.path,
                    &card.name,
                    &diagnostic,
                    Some(&card.origin),
                    Some(card.api_family),
                );
                // The typed spawn boundary, not diagnostic text, owns the stage.
                failure.stage = LoadStage::RuntimeLoad;
                self.record_failure(failure);
                Err(user_diagnostic)
            }
            Err(
                script::StartLoadError::Waiting(diagnostic)
                | script::StartLoadError::Refused(diagnostic)
                | script::StartLoadError::NoLoadouts(diagnostic),
            ) => Err(diagnostic),
            Err(script::StartLoadError::Surface(error)) => Err(error.to_string()),
        }
    }

    pub fn clear_failure(&mut self, identity_key: &str) {
        self.failures.remove(identity_key);
    }

    fn note_err(
        &mut self,
        source: ScriptSource,
        path: &Path,
        name: &str,
        diagnostic: &str,
        origin: Option<&str>,
        api_family: Option<ApiFamily>,
    ) {
        self.record_failure(LoadFailure::capture(
            source, path, name, diagnostic, origin, api_family,
        ));
    }

    fn note_card_outcome(&mut self, card: &JsCard) {
        if let Some(spec) = &card.unloadable {
            self.record_failure(LoadFailure::capture(
                card.source,
                &card.path,
                &card.name,
                &format!("unloadable import: {spec}"),
                Some(card.origin.as_str()),
                Some(card.api_family),
            ));
        } else {
            self.clear_failure(&card.identity_key());
        }
    }

    /// Re-read `card`'s origin from disk; when the SHA differs, fetch a new
    /// cached object and update that `(source, name)` card in place. Isolate
    /// respawn is the caller's job — the updated `js`/`sha256` are on the
    /// card when they do.
    pub fn refresh(&mut self, source: ScriptSource, name: &str) -> Result<(), String> {
        let prior = self
            .get(source, name)
            .map(|c| (c.path.clone(), c.api_family));
        match self.refresh_unrecorded(source, name) {
            Ok(()) => {
                if let Some(card) = self.get(source, name).cloned() {
                    self.note_card_outcome(&card);
                }
                Ok(())
            }
            Err(e) => {
                if let Some((path, family)) = prior {
                    let origin = std::fs::read_to_string(&path).ok();
                    self.note_err(source, &path, name, &e, origin.as_deref(), Some(family));
                }
                Err(e)
            }
        }
    }

    fn candidate_settings_schema(
        &self,
        card: &JsCard,
        origin: &str,
    ) -> Result<Vec<SettingDef>, String> {
        match card.source {
            ScriptSource::Catalog => {
                self.resolve_catalog_settings_schema(&card.name, &card.path, origin)
            }
            ScriptSource::File | ScriptSource::Builtin => {
                crate::rs2b0t_registry::try_settings_schema_from_source(origin)
                    .map_err(|error| format!("{}: {error}", card.name))
            }
        }
    }

    fn refresh_unrecorded(&mut self, source: ScriptSource, name: &str) -> Result<(), String> {
        let idx = self
            .cards
            .iter()
            .position(|c| c.source == source && c.name == name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?;
        let original = self.cards[idx].clone();
        let path = original.path.clone();
        let origin = std::fs::read_to_string(&path)
            .map_err(|e| format!("refresh {}: {e}", path.display()))?;
        let settings_schema = self.candidate_settings_schema(&original, &origin)?;
        let (shape, api_family) =
            resolve_api_family(&origin).map_err(|e| format!("{name}: {e}"))?;
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        let cached = self
            .cache
            .get_or_transpile(
                &path,
                origin.as_bytes(),
                cache_meta(shape, source, api_family),
            )
            .map_err(|e| format!("{name}: {e}"))?;
        let unloadable = catalog_unloadable(
            name,
            source,
            &cached.sha256,
            &path,
            first_unloadable_for_card(&origin, &path),
        );
        let card = &mut self.cards[idx];
        card.path = path;
        card.shape = shape;
        card.origin = origin;
        card.js = cached.js;
        card.kind = shape_to_kind(shape);
        card.sha256 = cached.sha256;
        card.unloadable = unloadable;
        card.api_family = api_family;
        card.settings_schema = settings_schema;
        let snap = card.clone();
        self.remember_fingerprint(&snap);
        Ok(())
    }

    /// The card registered under `(source, name)`, if any.
    /// File cards match stored path first, then display stem (legacy).
    pub fn get(&self, source: ScriptSource, name: &str) -> Option<&JsCard> {
        if source == ScriptSource::File {
            self.cards
                .iter()
                .find(|c| c.source == source && script::identity::paths_match(name, &c.path))
                .or_else(|| {
                    self.cards
                        .iter()
                        .find(|c| c.source == source && c.name == name)
                })
        } else {
            self.cards
                .iter()
                .find(|c| c.source == source && c.name == name)
        }
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
        if let Some(refusal) = &self.store_refusal {
            return Err(format!(
                "{refusal}; it is left untouched, so nothing is saved until it is fixed or removed"
            ));
        }
        let json =
            serde_json::to_string_pretty(entries).map_err(|e| format!("js-scripts.json: {e}"))?;
        vault::write_private_file(&self.store, json.as_bytes())
            .map_err(|e| format!("js-scripts.json: {e}"))
    }

    /// The SHA cache backing this library (Start sibling resolve).
    pub fn cache(&self) -> &JsCache {
        &self.cache
    }

    fn remember_fingerprint(&mut self, card: &JsCard) {
        let fp = raw_content_fingerprint(&card.path, &card.origin);
        self.fingerprints.insert(card.identity_key(), fp);
    }

    pub fn stored_fingerprint(&self, key: &str) -> Option<&str> {
        self.fingerprints.get(key).map(String::as_str)
    }

    /// Hash raw entry + supported sibling bytes. No transpile.
    pub fn disk_fingerprint(&self, source: ScriptSource, name: &str) -> Result<String, String> {
        let card = self
            .get(source, name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?;
        let origin = match std::fs::read_to_string(&card.path) {
            Ok(text) => text,
            Err(e) => {
                return Err(format!("missing {}: {e}", card.path.display()));
            }
        };
        Ok(raw_content_fingerprint(&card.path, &origin))
    }

    /// Compare stored fingerprint to current disk bytes. Unchanged means
    /// skip transpile/reload.
    pub fn raw_source_changed(&self, source: ScriptSource, name: &str) -> Result<bool, String> {
        let key = self
            .get(source, name)
            .map(|c| c.identity_key())
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?;
        let now = self.disk_fingerprint(source, name)?;
        Ok(self.fingerprints.get(&key).map(String::as_str) != Some(now.as_str()))
    }

    /// Capture only the immutable metadata needed to prepare one candidate.
    /// The source, siblings, cache output and the rest of the library are not
    /// copied; disk work begins only when the snapshot runs on WorkAdmission.
    pub fn card_preparation_snapshot(
        &self,
        source: ScriptSource,
        name: &str,
        confirmation_fingerprint: Option<String>,
    ) -> Result<CardPreparationSnapshot, String> {
        let card = self
            .get(source, name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?;
        let identity_key = card.identity_key();
        let snapshot = CardPreparationSnapshot {
            template: CardPreparationTemplate::from_card(card),
            cache_root: self.cache.root().to_path_buf(),
            catalog_root: self.catalog_root.clone(),
            stored_fingerprint: self.fingerprints.get(&identity_key).cloned(),
            confirmation_fingerprint,
            catalog_settings_schema: None,
            allow_unloadable: false,
            source_limit_override: None,
            force_prepare: false,
        };
        snapshot.source_limit()?;
        Ok(snapshot)
    }

    /// Retain a worker-side validation failure against the exact bytes that
    /// were validated.
    pub fn record_prepared_failure(&mut self, prepared: &PreparedCard, diagnostic: &str) {
        let mut failure = LoadFailure::capture_with_fingerprint(
            prepared.card.source,
            &prepared.card.path,
            &prepared.card.name,
            diagnostic,
            prepared.fingerprint.clone(),
            Some(prepared.card.api_family),
        );
        // Validation batches are the typed runtime boundary; diagnostic
        // wording does not decide whether this was a transpile failure.
        failure.stage = LoadStage::RuntimeLoad;
        self.record_failure(failure);
    }

    pub fn commit_prepared(&mut self, prepared: PreparedCard) -> Result<JsCard, String> {
        self.commit_prepared_with_failure_policy(prepared, true)
    }

    /// Keep the prior runtime diagnostic visible until the newly prepared
    /// Start actually reaches Ready; preparation alone does not prove it runs.
    pub fn commit_prepared_for_start(&mut self, prepared: PreparedCard) -> Result<JsCard, String> {
        self.commit_prepared_with_failure_policy(prepared, false)
    }

    fn commit_prepared_with_failure_policy(
        &mut self,
        prepared: PreparedCard,
        clear_failure: bool,
    ) -> Result<JsCard, String> {
        let key = prepared.card.identity_key();
        if let Some(idx) = self.cards.iter().position(|c| c.identity_key() == key) {
            self.cards[idx] = prepared.card.clone();
        } else {
            self.cards.push(prepared.card.clone());
        }
        self.fingerprints
            .insert(key.clone(), prepared.fingerprint.clone());
        if prepared.card.source == ScriptSource::File {
            self.persist_file_cards(&self.cards)?;
        }
        if clear_failure {
            self.clear_failure(&key);
        }
        Ok(prepared.card)
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

fn add_capacity(total: &mut usize, bytes: usize) -> Option<()> {
    *total = total.checked_add(bytes)?;
    Some(())
}

fn add_optional_capacity(total: &mut usize, value: Option<&String>) -> Option<()> {
    if let Some(value) = value {
        add_capacity(total, value.capacity())?;
    }
    Some(())
}

fn add_setting_bytes(total: &mut usize, setting: &SettingDef) -> Option<()> {
    add_capacity(total, setting.id.capacity())?;
    add_capacity(total, setting.ty.capacity())?;
    for value in [
        &setting.default,
        &setting.label,
        &setting.min,
        &setting.max,
        &setting.step,
        &setting.group,
        &setting.show_if,
        &setting.options_from,
        &setting.csv_toggle,
        &setting.help,
    ] {
        add_optional_capacity(total, value.as_ref())?;
    }
    for values in [&setting.options, &setting.option_labels] {
        add_capacity(
            total,
            values
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        for value in values {
            add_capacity(total, value.capacity())?;
        }
    }
    if let Some(spec) = &setting.item_option_spec {
        add_capacity(
            total,
            spec.prefix
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        for prefix in &spec.prefix {
            add_capacity(total, prefix.capacity())?;
        }
        add_capacity(
            total,
            spec.candidates.capacity().checked_mul(std::mem::size_of::<
                crate::rs2b0t_registry::ItemOptionCandidate,
            >())?,
        )?;
        for candidate in &spec.candidates {
            add_capacity(total, candidate.key.capacity())?;
            add_optional_capacity(total, candidate.label.as_ref())?;
        }
    }
    Some(())
}

fn add_schema_bytes(total: &mut usize, schema: &[SettingDef], capacity: usize) -> Option<()> {
    add_capacity(
        total,
        capacity.checked_mul(std::mem::size_of::<SettingDef>())?,
    )?;
    for setting in schema {
        add_setting_bytes(total, setting)?;
    }
    Some(())
}
/// A reload candidate computed from one immutable card snapshot. `Unchanged`
/// skips V8; `Confirmed` means the warned fingerprint is still exact.

#[derive(Debug)]
pub enum CardPreparationOutcome {
    Unchanged,
    Confirmed,
    Prepared(PreparedCard),
    Failed { error: String, failure: LoadFailure },
}

// `Clone` is needed: `catalog.rs` clones the retained template out of the
// snapshot map once per card it prepares (`existing.get(&name).cloned()`).

#[derive(Debug, Clone)]
struct CardPreparationTemplate {
    name: String,
    path: PathBuf,
    source: ScriptSource,
    description: String,
    category: String,
    tags: Vec<String>,
    settings_schema: Vec<SettingDef>,
    unloadable: Option<String>,
    api_family: Option<ApiFamily>,
}

impl CardPreparationTemplate {
    fn from_card(card: &JsCard) -> Self {
        Self {
            name: card.name.clone(),
            path: card.path.clone(),
            source: card.source,
            description: card.description.clone(),
            category: card.category.clone(),
            tags: card.tags.clone(),
            settings_schema: card.settings_schema.clone(),
            unloadable: card.unloadable.clone(),
            api_family: Some(card.api_family),
        }
    }

    fn from_registry_card(card: &crate::rs2b0t_registry::RegistryCard, path: PathBuf) -> Self {
        Self {
            name: card.name.clone(),
            path,
            source: ScriptSource::Catalog,
            description: card.description.clone(),
            category: card.category.clone(),
            tags: card.tags.clone(),
            settings_schema: card.settings_schema.clone(),
            unloadable: None,
            api_family: None,
        }
    }

    fn retained_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>();
        add_capacity(&mut bytes, self.name.capacity())?;
        add_capacity(&mut bytes, self.path.as_os_str().len())?;
        add_capacity(&mut bytes, self.description.capacity())?;
        add_capacity(&mut bytes, self.category.capacity())?;
        add_capacity(
            &mut bytes,
            self.tags
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        for tag in &self.tags {
            add_capacity(&mut bytes, tag.capacity())?;
        }
        if let Some(unloadable) = &self.unloadable {
            add_capacity(&mut bytes, unloadable.capacity())?;
        }
        add_schema_bytes(
            &mut bytes,
            &self.settings_schema,
            self.settings_schema.capacity(),
        )?;
        Some(bytes)
    }

    fn build(
        &self,
        origin: String,
        cached: CachedJs,
        shape: LoadShape,
        api_family: ApiFamily,
        settings_schema: Vec<SettingDef>,
        unloadable: Option<String>,
    ) -> JsCard {
        JsCard {
            name: self.name.clone(),
            path: self.path.clone(),
            shape,
            origin,
            js: cached.js,
            kind: shape_to_kind(shape),
            source: self.source,
            sha256: cached.sha256,
            description: self.description.clone(),
            category: self.category.clone(),
            tags: self.tags.clone(),
            settings_schema,
            unloadable,
            api_family,
        }
    }
}

/// The only retained library state in a card preparation job: one card's
/// metadata, its cache root, registry root and generation-baseline hashes.
/// Source text and compiled JS are deliberately not cloned from the library.

#[derive(Debug)]
pub struct CardPreparationSnapshot {
    template: CardPreparationTemplate,
    cache_root: PathBuf,
    catalog_root: Option<PathBuf>,
    stored_fingerprint: Option<String>,
    confirmation_fingerprint: Option<String>,
    catalog_settings_schema: Option<Vec<SettingDef>>,
    source_limit_override: Option<usize>,
    force_prepare: bool,
    allow_unloadable: bool,
}

impl CardPreparationSnapshot {
    /// Maximum charged bytes for this bounded card job.
    pub const fn input_bytes(&self) -> usize {
        MAX_PREPARATION_SOURCE_BYTES
    }

    pub const fn scratch_bytes(&self) -> usize {
        16 * 1024 * 1024
    }

    pub const fn result_bytes(&self) -> usize {
        MAX_PREPARATION_RESULT_BYTES
    }

    /// Prepare even when the stored source fingerprint matches.
    ///
    /// This is used to warm a card that was registered without transpiling;
    /// the bounded input, scratch, and result charges do not change.
    pub fn force_prepare(mut self) -> Self {
        self.force_prepare = true;
        self
    }

    /// Fingerprint captured with the library card before the worker read.
    /// A consumer checks this against the current library before publication.
    pub fn stored_fingerprint(&self) -> Option<&str> {
        self.stored_fingerprint.as_deref()
    }

    /// Check the card identity captured before preparation without copying
    /// its (potentially 8 MiB) source text back onto the UI thread.
    pub fn matches_card(&self, card: &JsCard) -> bool {
        self.template.source == card.source
            && self.template.name == card.name
            && self.template.path == card.path
    }

    /// Warm display/cache output even for an unavailable catalogue card.
    /// The prepared card retains its unloadable reason; Start still refuses it.
    pub fn cache_even_if_unloadable(mut self) -> Self {
        self.allow_unloadable = true;
        self
    }

    /// Source + siblings share the worker's 8 MiB input charge with the
    /// immutable metadata snapshot.
    pub fn source_limit(&self) -> Result<usize, String> {
        let mut retained = self
            .template
            .retained_bytes()
            .ok_or_else(|| "card preparation snapshot is too large".to_string())?;
        add_capacity(&mut retained, self.cache_root.as_os_str().len())
            .ok_or_else(|| "card preparation snapshot is too large".to_string())?;
        if let Some(root) = &self.catalog_root {
            add_capacity(&mut retained, root.as_os_str().len())
                .ok_or_else(|| "card preparation snapshot is too large".to_string())?;
        }
        if let Some(fingerprint) = &self.stored_fingerprint {
            add_capacity(&mut retained, fingerprint.capacity())
                .ok_or_else(|| "card preparation snapshot is too large".to_string())?;
        }
        if let Some(fingerprint) = &self.confirmation_fingerprint {
            add_capacity(&mut retained, fingerprint.capacity())
                .ok_or_else(|| "card preparation snapshot is too large".to_string())?;
        }
        if let Some(schema) = &self.catalog_settings_schema {
            add_schema_bytes(&mut retained, schema, schema.capacity())
                .ok_or_else(|| "catalog settings snapshot is too large".to_string())?;
        }
        let remaining = MAX_PREPARATION_SOURCE_BYTES
            .checked_sub(retained)
            .filter(|remaining| *remaining > 0)
            .ok_or_else(|| "card metadata exceeds the 8 MiB preparation bound".to_string())?;
        Ok(self
            .source_limit_override
            .map_or(remaining, |limit| remaining.min(limit)))
    }

    /// Read and fingerprint on the worker; only changed candidates transpile.
    pub fn prepare(&self) -> Result<CardPreparationOutcome, String> {
        let source_limit = match self.source_limit() {
            Ok(limit) => limit,
            Err(error) => {
                return Ok(self.failed_outcome(error, None, None, MAX_PREPARATION_SOURCE_BYTES));
            }
        };
        let origin = match self.read_origin(source_limit) {
            Ok(origin) => origin,
            Err(error) => {
                return Ok(self.failed_outcome(
                    error,
                    None,
                    self.template.api_family,
                    source_limit,
                ));
            }
        };
        let fingerprint =
            match raw_content_fingerprint_bounded(&self.template.path, &origin, source_limit) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    let family = resolve_api_family(&origin)
                        .ok()
                        .map(|(_, family)| family)
                        .or(self.template.api_family);
                    return Ok(self.failed_outcome(error, Some(&origin), family, source_limit));
                }
            };
        if self.confirmation_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return Ok(CardPreparationOutcome::Confirmed);
        }
        if !self.force_prepare && self.stored_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return Ok(CardPreparationOutcome::Unchanged);
        }
        match self.prepare_candidate(origin, source_limit) {
            Ok(prepared) => Ok(CardPreparationOutcome::Prepared(prepared)),
            Err((error, origin, family)) => {
                Ok(self.failed_outcome(error, Some(&origin), family, source_limit))
            }
        }
    }

    fn failed_outcome(
        &self,
        error: String,
        origin: Option<&str>,
        family: Option<ApiFamily>,
        source_limit: usize,
    ) -> CardPreparationOutcome {
        let failure = if let Some(origin) = origin {
            LoadFailure::capture_bounded(
                self.template.source,
                &self.template.path,
                &self.template.name,
                &error,
                origin,
                family,
                source_limit,
            )
        } else {
            LoadFailure::capture(
                self.template.source,
                &self.template.path,
                &self.template.name,
                &error,
                None,
                family,
            )
        };
        CardPreparationOutcome::Failed { error, failure }
    }

    fn read_origin(&self, source_limit: usize) -> Result<String, String> {
        let limit = source_limit.min(MAX_SOURCE_BYTES as usize) as u64;
        vault::read_regular_file(&self.template.path, limit)
            .map_err(|error| format!("missing {}: {error}", self.template.path.display()))
    }

    fn prepare_candidate(
        &self,
        origin: String,
        source_limit: usize,
    ) -> Result<PreparedCard, (String, String, Option<ApiFamily>)> {
        let fallback_family = self.template.api_family;
        let prepared = self.prepare_candidate_inner(&origin, source_limit);
        match prepared {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                let family = resolve_api_family(&origin)
                    .ok()
                    .map(|(_, family)| family)
                    .or(fallback_family);
                Err((error, origin, family))
            }
        }
    }

    fn prepare_candidate_inner(
        &self,
        origin: &str,
        source_limit: usize,
    ) -> Result<PreparedCard, String> {
        let settings_schema = if let Some(schema) = &self.catalog_settings_schema {
            schema.clone()
        } else {
            match self.template.source {
                ScriptSource::Catalog => catalog::resolve_catalog_settings_schema_bounded(
                    self.catalog_root.as_deref(),
                    &self.template.name,
                    &self.template.path,
                    origin,
                    source_limit,
                )?,
                ScriptSource::File | ScriptSource::Builtin => {
                    crate::rs2b0t_registry::try_settings_schema_from_source(origin)
                        .map_err(|error| format!("{}: {error}", self.template.name))?
                }
            }
        };
        let (shape, api_family) = resolve_api_family(origin)
            .map_err(|error| format!("{}: {error}", self.template.name))?;
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {}", self.template.name));
        }
        let cache = JsCache::new(self.cache_root.clone());
        let meta = cache_meta(shape, self.template.source, api_family);
        let object = cache.object_path(&JsCache::origin_sha(origin.as_bytes()));
        if std::fs::metadata(&object)
            .is_ok_and(|metadata| metadata.len() > MAX_PREPARATION_RESULT_BYTES as u64)
        {
            return Err("cached JavaScript exceeds the 16 MiB result limit".into());
        }
        let cached = cache
            .get_or_transpile(&self.template.path, origin.as_bytes(), meta.clone())
            .map_err(|error| format!("{}: {error}", self.template.name))?;
        let mut result_bytes = self
            .template
            .retained_bytes()
            .and_then(|bytes| bytes.checked_add(origin.len()))
            .and_then(|bytes| bytes.checked_add(cached.js.capacity()))
            .ok_or_else(|| "prepared card exceeds the 16 MiB result limit".to_string())?;
        if let Some(schema) = &self.catalog_settings_schema {
            add_schema_bytes(&mut result_bytes, schema, schema.capacity())
                .ok_or_else(|| "prepared card exceeds the 16 MiB result limit".to_string())?;
        }
        if result_bytes > MAX_PREPARATION_RESULT_BYTES {
            return Err("prepared card exceeds the 16 MiB result limit".into());
        }
        let first_unloadable =
            first_unloadable_for_card_bounded(origin, &self.template.path, source_limit)?;
        let unloadable = catalog_unloadable(
            &self.template.name,
            self.template.source,
            &cached.sha256,
            &self.template.path,
            first_unloadable,
        );
        if !self.allow_unloadable {
            if let Some(reason) = &unloadable {
                return Err(format!("unloadable import: {reason}"));
            }
        }
        let siblings = resolve_sibling_modules_bounded(
            &self.template.path,
            origin,
            &cache,
            meta,
            result_bytes,
            source_limit,
        )?;
        let fingerprint =
            raw_content_fingerprint_bounded(&self.template.path, origin, source_limit)?;
        result_bytes = result_bytes
            .checked_add(fingerprint.capacity())
            .ok_or_else(|| "prepared card exceeds the 16 MiB result limit".to_string())?;
        if result_bytes > MAX_PREPARATION_RESULT_BYTES {
            return Err("prepared card exceeds the 16 MiB result limit".into());
        }
        let card = self.template.build(
            origin.to_string(),
            cached,
            shape,
            api_family,
            settings_schema,
            unloadable,
        );
        Ok(PreparedCard {
            card,
            siblings,
            fingerprint,
        })
    }

    /// Submit this captured card through one bounded preparation stage per
    /// worker dispatch, without cloning its retained source or card metadata.
    pub fn cooperative_prepare(
        self: std::sync::Arc<Self>,
    ) -> impl FnMut(
        &mut api::selected::FamilyPreparation,
    ) -> script::preparation::CooperativePreparationStep<
        Result<CardPreparationOutcome, String>,
    > + Send
           + 'static {
        let mut work = CardPreparationTask::new_shared(self);
        let mut completed = 0_u32;
        move |_| {
            completed = completed.saturating_add(1);
            match work.step() {
                Ok(None) => script::preparation::CooperativePreparationStep::Continue {
                    completed,
                    total: None,
                },
                Ok(Some(outcome)) => script::preparation::CooperativePreparationStep::Complete {
                    output: Ok(outcome),
                    completed,
                    total: Some(completed),
                },
                Err(error) => script::preparation::CooperativePreparationStep::Complete {
                    output: Err(error),
                    completed,
                    total: Some(completed),
                },
            }
        }
    }

    /// Begin incremental preparation; each call advances one bounded stage.
    pub(crate) fn start_cooperative(self) -> CardPreparationTask {
        CardPreparationTask::new(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardPreparationPhase {
    SourceLimit,
    ReadOrigin,
    FingerprintStart,
    Fingerprint,
    Settings,
    Shape,
    Cache,
    UnloadableStart,
    Unloadable,
    SiblingsStart,
    Siblings,
    FinalFingerprintStart,
    FinalFingerprint,
    FailureFingerprint,
    Finish,
    Complete,
}

// Keep the owned snapshot inline: boxing each card would add an allocation
// solely to shrink the already boxed preparation job's enum footprint.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum CardTaskSnapshot {
    Owned(CardPreparationSnapshot),
    Shared(std::sync::Arc<CardPreparationSnapshot>),
}

impl std::ops::Deref for CardTaskSnapshot {
    type Target = CardPreparationSnapshot;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(snapshot) => snapshot,
            Self::Shared(snapshot) => snapshot,
        }
    }
}

impl CardTaskSnapshot {
    fn take_catalog_settings_schema(&mut self) -> Option<Vec<SettingDef>> {
        match self {
            Self::Owned(snapshot) => snapshot.catalog_settings_schema.take(),
            Self::Shared(snapshot) => {
                if let Some(unique) = std::sync::Arc::get_mut(snapshot) {
                    unique.catalog_settings_schema.take()
                } else {
                    snapshot.catalog_settings_schema.clone()
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct CardPreparationTask {
    snapshot: CardTaskSnapshot,
    phase: CardPreparationPhase,
    source_limit: usize,
    origin: Option<String>,
    fingerprint_work: Option<PreparationFingerprintWork>,
    failure_work: Option<PreparationFingerprintWork>,
    pending_failure: Option<(String, Option<ApiFamily>)>,
    settings_schema: Option<Vec<SettingDef>>,
    has_catalog_settings_schema: bool,
    shape: Option<LoadShape>,
    api_family: Option<ApiFamily>,
    meta: Option<CacheMeta>,
    cached: Option<CachedJs>,
    result_bytes: usize,
    unloadable_work: Option<FirstUnloadableWork>,
    unloadable: Option<String>,
    siblings_work: Option<SiblingCompilationWork>,
    siblings: Option<Vec<(String, String)>>,
    fingerprint: Option<String>,
}

impl CardPreparationTask {
    fn new(snapshot: CardPreparationSnapshot) -> Self {
        Self::with_snapshot(CardTaskSnapshot::Owned(snapshot))
    }

    fn new_shared(snapshot: std::sync::Arc<CardPreparationSnapshot>) -> Self {
        Self::with_snapshot(CardTaskSnapshot::Shared(snapshot))
    }

    fn with_snapshot(snapshot: CardTaskSnapshot) -> Self {
        let has_catalog_settings_schema = snapshot.catalog_settings_schema.is_some();
        Self {
            snapshot,
            phase: CardPreparationPhase::SourceLimit,
            source_limit: MAX_PREPARATION_SOURCE_BYTES,
            origin: None,
            fingerprint_work: None,
            failure_work: None,
            pending_failure: None,
            settings_schema: None,
            has_catalog_settings_schema,
            shape: None,
            api_family: None,
            meta: None,
            cached: None,
            result_bytes: 0,
            unloadable_work: None,
            unloadable: None,
            siblings_work: None,
            siblings: None,
            fingerprint: None,
        }
    }

    pub(crate) fn step(&mut self) -> Result<Option<CardPreparationOutcome>, String> {
        match self.phase {
            CardPreparationPhase::SourceLimit => {
                self.source_limit = match self.snapshot.source_limit() {
                    Ok(source_limit) => source_limit,
                    Err(error) => return self.begin_failure(error, None),
                };
                self.phase = CardPreparationPhase::ReadOrigin;
            }
            CardPreparationPhase::ReadOrigin => {
                self.origin = match self.snapshot.read_origin(self.source_limit) {
                    Ok(origin) => Some(origin),
                    Err(error) => {
                        return self.begin_failure(error, self.snapshot.template.api_family);
                    }
                };
                self.phase = CardPreparationPhase::FingerprintStart;
            }
            CardPreparationPhase::FingerprintStart => {
                let origin = self.origin.as_deref().expect("origin was read");
                self.fingerprint_work = match PreparationFingerprintWork::new(
                    &self.snapshot.template.path,
                    origin,
                    self.source_limit,
                ) {
                    Ok(work) => Some(work),
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                };
                self.phase = CardPreparationPhase::Fingerprint;
            }
            CardPreparationPhase::Fingerprint => {
                let origin = self.origin.as_deref().expect("origin was read");
                match self
                    .fingerprint_work
                    .as_mut()
                    .expect("fingerprint work exists")
                    .step(origin)
                {
                    Ok(None) => return Ok(None),
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                    Ok(Some(fingerprint)) => {
                        self.fingerprint_work = None;
                        if self.snapshot.confirmation_fingerprint.as_deref()
                            == Some(fingerprint.as_str())
                        {
                            self.phase = CardPreparationPhase::Complete;
                            return Ok(Some(CardPreparationOutcome::Confirmed));
                        }
                        if !self.snapshot.force_prepare
                            && self.snapshot.stored_fingerprint.as_deref()
                                == Some(fingerprint.as_str())
                        {
                            self.phase = CardPreparationPhase::Complete;
                            return Ok(Some(CardPreparationOutcome::Unchanged));
                        }
                        self.phase = CardPreparationPhase::Settings;
                    }
                }
            }
            CardPreparationPhase::Settings => {
                self.settings_schema = if let Some(schema) =
                    self.snapshot.take_catalog_settings_schema()
                {
                    Some(schema)
                } else {
                    let origin = self.origin.as_deref().expect("origin was read");
                    let result = match self.snapshot.template.source {
                        ScriptSource::Catalog => catalog::resolve_catalog_settings_schema_bounded(
                            self.snapshot.catalog_root.as_deref(),
                            &self.snapshot.template.name,
                            &self.snapshot.template.path,
                            origin,
                            self.source_limit,
                        ),
                        ScriptSource::File | ScriptSource::Builtin => {
                            crate::rs2b0t_registry::try_settings_schema_from_source(origin).map_err(
                                |error| format!("{}: {error}", self.snapshot.template.name),
                            )
                        }
                    };
                    match result {
                        Ok(schema) => Some(schema),
                        Err(error) => {
                            let family = self.failure_family();
                            return self.begin_failure(error, family);
                        }
                    }
                };
                self.phase = CardPreparationPhase::Shape;
            }
            CardPreparationPhase::Shape => {
                let origin = self.origin.as_deref().expect("origin was read");
                let (shape, api_family) = match resolve_api_family(origin) {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        let error = format!("{}: {error}", self.snapshot.template.name);
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                };
                if shape == LoadShape::Reject {
                    return self.begin_failure(
                        format!("not a bot shape: {}", self.snapshot.template.name),
                        Some(api_family),
                    );
                }
                self.shape = Some(shape);
                self.api_family = Some(api_family);
                self.phase = CardPreparationPhase::Cache;
            }
            CardPreparationPhase::Cache => {
                let origin = self.origin.as_deref().expect("origin was read");
                let shape = self.shape.expect("shape was resolved");
                let api_family = self.api_family.expect("API family was resolved");
                let cache = JsCache::new(self.snapshot.cache_root.clone());
                let meta = cache_meta(shape, self.snapshot.template.source, api_family);
                let object = cache.object_path(&JsCache::origin_sha(origin.as_bytes()));
                if std::fs::metadata(&object)
                    .is_ok_and(|metadata| metadata.len() > MAX_PREPARATION_RESULT_BYTES as u64)
                {
                    return self.begin_failure(
                        "cached JavaScript exceeds the 16 MiB result limit".into(),
                        Some(api_family),
                    );
                }
                let cached = match cache.get_or_transpile(
                    &self.snapshot.template.path,
                    origin.as_bytes(),
                    meta.clone(),
                ) {
                    Ok(cached) => cached,
                    Err(error) => {
                        let error = format!("{}: {error}", self.snapshot.template.name);
                        return self.begin_failure(error, Some(api_family));
                    }
                };
                let result_bytes = self
                    .snapshot
                    .template
                    .retained_bytes()
                    .and_then(|bytes| bytes.checked_add(origin.len()))
                    .and_then(|bytes| bytes.checked_add(cached.js.capacity()))
                    .ok_or_else(|| "prepared card exceeds the 16 MiB result limit".to_string());
                let mut result_bytes = match result_bytes {
                    Ok(bytes) => bytes,
                    Err(error) => return self.begin_failure(error, Some(api_family)),
                };
                if self.has_catalog_settings_schema {
                    if let Some(schema) = &self.settings_schema {
                        if add_schema_bytes(&mut result_bytes, schema, schema.capacity()).is_none()
                        {
                            return self.begin_failure(
                                "prepared card exceeds the 16 MiB result limit".into(),
                                Some(api_family),
                            );
                        }
                    }
                }
                if result_bytes > MAX_PREPARATION_RESULT_BYTES {
                    return self.begin_failure(
                        "prepared card exceeds the 16 MiB result limit".into(),
                        Some(api_family),
                    );
                }
                self.result_bytes = result_bytes;
                self.meta = Some(meta);
                self.cached = Some(cached);
                self.phase = CardPreparationPhase::UnloadableStart;
            }
            CardPreparationPhase::UnloadableStart => {
                let origin = self.origin.as_deref().expect("origin was read");
                self.unloadable_work = match FirstUnloadableWork::new(
                    origin,
                    &self.snapshot.template.path,
                    self.source_limit,
                ) {
                    Ok(work) => Some(work),
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                };
                self.phase = CardPreparationPhase::Unloadable;
            }
            CardPreparationPhase::Unloadable => {
                let first_unloadable = match self
                    .unloadable_work
                    .as_mut()
                    .expect("unloadable work exists")
                    .step()
                {
                    Ok(None) => return Ok(None),
                    Ok(Some(first_unloadable)) => first_unloadable,
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                };
                let cached = self.cached.as_ref().expect("cached card exists");
                let unloadable = catalog_unloadable(
                    &self.snapshot.template.name,
                    self.snapshot.template.source,
                    &cached.sha256,
                    &self.snapshot.template.path,
                    first_unloadable,
                );
                if !self.snapshot.allow_unloadable {
                    if let Some(reason) = &unloadable {
                        return self.begin_failure(
                            format!("unloadable import: {reason}"),
                            self.api_family,
                        );
                    }
                }
                self.unloadable = unloadable;
                self.unloadable_work = None;
                self.phase = CardPreparationPhase::SiblingsStart;
            }
            CardPreparationPhase::SiblingsStart => {
                let origin = self.origin.as_deref().expect("origin was read");
                let work = match SiblingCompilationWork::new(
                    &self.snapshot.template.path,
                    origin,
                    JsCache::new(self.snapshot.cache_root.clone()),
                    self.meta.clone().expect("cache metadata exists"),
                    self.result_bytes,
                    self.source_limit,
                ) {
                    Ok(work) => work,
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                };
                self.siblings_work = Some(work);
                self.phase = CardPreparationPhase::Siblings;
            }
            CardPreparationPhase::Siblings => {
                match self
                    .siblings_work
                    .as_mut()
                    .expect("sibling work exists")
                    .step()
                {
                    Ok(None) => return Ok(None),
                    Ok(Some(siblings)) => {
                        self.siblings = Some(siblings);
                        self.siblings_work = None;
                        self.phase = CardPreparationPhase::FinalFingerprintStart;
                    }
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                }
            }
            CardPreparationPhase::FinalFingerprintStart => {
                let origin = self.origin.as_deref().expect("origin was read");
                self.fingerprint_work = match PreparationFingerprintWork::new(
                    &self.snapshot.template.path,
                    origin,
                    self.source_limit,
                ) {
                    Ok(work) => Some(work),
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                };
                self.phase = CardPreparationPhase::FinalFingerprint;
            }
            CardPreparationPhase::FinalFingerprint => {
                let origin = self.origin.as_deref().expect("origin was read");
                match self
                    .fingerprint_work
                    .as_mut()
                    .expect("final fingerprint work exists")
                    .step(origin)
                {
                    Ok(None) => return Ok(None),
                    Err(error) => {
                        let family = self.failure_family();
                        return self.begin_failure(error, family);
                    }
                    Ok(Some(fingerprint)) => {
                        let Some(result_bytes) =
                            self.result_bytes.checked_add(fingerprint.capacity())
                        else {
                            return self.begin_failure(
                                "prepared card exceeds the 16 MiB result limit".into(),
                                self.api_family,
                            );
                        };
                        if result_bytes > MAX_PREPARATION_RESULT_BYTES {
                            return self.begin_failure(
                                "prepared card exceeds the 16 MiB result limit".into(),
                                self.api_family,
                            );
                        }
                        self.result_bytes = result_bytes;
                        self.fingerprint = Some(fingerprint);
                        self.fingerprint_work = None;
                        self.phase = CardPreparationPhase::Finish;
                    }
                }
            }
            CardPreparationPhase::FailureFingerprint => {
                let origin = self.origin.as_deref().expect("failed source exists");
                match self
                    .failure_work
                    .as_mut()
                    .expect("failure fingerprint work exists")
                    .step(origin)
                {
                    Ok(None) => return Ok(None),
                    Ok(Some(fingerprint)) => {
                        let (error, family) =
                            self.pending_failure.take().expect("failure is pending");
                        return Ok(Some(self.finish_failure(error, fingerprint, family)));
                    }
                    Err(_) => {
                        let (error, family) =
                            self.pending_failure.take().expect("failure is pending");
                        return Ok(Some(self.finish_failure(error, String::new(), family)));
                    }
                }
            }
            CardPreparationPhase::Finish => {
                let cached = self.cached.take().expect("cached card exists");
                let card = self.snapshot.template.build(
                    self.origin.take().expect("origin was read"),
                    cached,
                    self.shape.expect("shape was resolved"),
                    self.api_family.expect("API family was resolved"),
                    self.settings_schema
                        .take()
                        .expect("settings schema was resolved"),
                    self.unloadable.take(),
                );
                let prepared = PreparedCard {
                    card,
                    siblings: self.siblings.take().expect("siblings were compiled"),
                    fingerprint: self
                        .fingerprint
                        .take()
                        .expect("final fingerprint was computed"),
                };
                self.phase = CardPreparationPhase::Complete;
                return Ok(Some(CardPreparationOutcome::Prepared(prepared)));
            }
            CardPreparationPhase::Complete => {
                return Err("card preparation task is already complete".into());
            }
        }
        Ok(None)
    }

    fn failure_family(&self) -> Option<ApiFamily> {
        self.api_family
            .or_else(|| {
                self.origin
                    .as_deref()
                    .and_then(|origin| resolve_api_family(origin).ok().map(|(_, family)| family))
            })
            .or(self.snapshot.template.api_family)
    }

    fn begin_failure(
        &mut self,
        error: String,
        family: Option<ApiFamily>,
    ) -> Result<Option<CardPreparationOutcome>, String> {
        let Some(origin) = self.origin.as_deref() else {
            self.phase = CardPreparationPhase::Complete;
            return Ok(Some(self.snapshot.failed_outcome(
                error,
                None,
                family,
                self.source_limit,
            )));
        };
        match PreparationFingerprintWork::new(
            &self.snapshot.template.path,
            origin,
            self.source_limit,
        ) {
            Ok(work) => {
                self.pending_failure = Some((error, family));
                self.failure_work = Some(work);
                self.phase = CardPreparationPhase::FailureFingerprint;
                Ok(None)
            }
            Err(_) => Ok(Some(self.finish_failure(error, String::new(), family))),
        }
    }

    fn finish_failure(
        &mut self,
        error: String,
        fingerprint: String,
        family: Option<ApiFamily>,
    ) -> CardPreparationOutcome {
        let failure = LoadFailure::capture_with_fingerprint(
            self.snapshot.template.source,
            &self.snapshot.template.path,
            &self.snapshot.template.name,
            &error,
            fingerprint,
            family,
        );
        self.phase = CardPreparationPhase::Complete;
        CardPreparationOutcome::Failed { error, failure }
    }
}

/// Candidate prepared while old library cards and isolates stay intact.
#[derive(Debug, Clone)]
pub struct PreparedCard {
    pub card: JsCard,
    pub siblings: Vec<(String, String)>,
    pub fingerprint: String,
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

fn cache_meta(shape: LoadShape, source: ScriptSource, family: ApiFamily) -> CacheMeta {
    CacheMeta {
        kind: shape_to_kind(shape),
        source,
        shape: Some(shape_label(shape).into()),
        api_family: Some(family.as_str().into()),
    }
}

fn missing_compat_api_export(source: ScriptSource, diagnostic: &str) -> Option<&str> {
    if source != ScriptSource::Catalog {
        return None;
    }
    let (_, diagnostic) = diagnostic.split_once("The requested module '")?;
    let (module, diagnostic) = diagnostic.split_once("' does not provide an export named '")?;
    let export = diagnostic.split_once('\'')?.0;
    if export.is_empty() || export == "default" || !is_compat_api_module(module) {
        return None;
    }
    Some(export)
}

fn is_compat_api_module(module: &str) -> bool {
    if module == "@rs2b0t/api" {
        return true;
    }
    let Some(path) = module
        .strip_prefix("./../../")
        .or_else(|| module.strip_prefix("../../"))
        .or_else(|| module.strip_prefix("#/bot/"))
        .or_else(|| module.strip_prefix("/rs2b0t/bot/"))
    else {
        return false;
    };
    let Some(path) = path.strip_prefix("api/") else {
        return false;
    };
    !path.is_empty()
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn classify_stage(diagnostic: &str, sibling: Option<&str>) -> LoadStage {
    let d = diagnostic.to_ascii_lowercase();
    if sibling.is_some()
        || d.contains("unloadable")
        || d.contains("sibling ")
        || d.contains("unresolvable")
    {
        return LoadStage::ImportResolution;
    }
    if d.contains("missing export")
        || d.contains("js engine")
        || d.contains("top-level")
        || d.contains("validate")
    {
        return LoadStage::RuntimeLoad;
    }
    if d.contains("no such file")
        || d.contains("unreadable")
        || d.contains("not utf-8")
        || d.contains("permission denied")
        || d.contains("os error")
        || d.contains("missing ")
    {
        return LoadStage::Read;
    }
    LoadStage::ParseTranspile
}

fn extract_line_column(diagnostic: &str) -> (Option<u32>, Option<u32>) {
    for token in
        diagnostic.split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '[' | ']'))
    {
        let mut parts = token.rsplitn(3, ':');
        let col = parts.next();
        let line = parts.next();
        let rest = parts.next();
        if rest.is_none() {
            continue;
        }
        if let (Some(line), Some(col)) = (line, col) {
            if let (Ok(line), Ok(col)) = (line.parse::<u32>(), col.parse::<u32>()) {
                if line > 0 {
                    return (Some(line), Some(col));
                }
            }
        }
    }
    (None, None)
}

fn extract_sibling(diagnostic: &str) -> Option<String> {
    const MARK: &str = "unloadable import: ";
    let idx = diagnostic.find(MARK)?;
    let rest = diagnostic[idx + MARK.len()..].trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
