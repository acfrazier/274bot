//! JS card library: picker cards, catalog apply, and prepare/commit.

#[cfg(feature = "load")]
use std::collections::{HashMap, HashSet};
mod catalog;

pub use catalog::{CatalogApplyReport, CatalogDiff};

use std::path::{Path, PathBuf};

#[cfg(feature = "load")]
use crate::js_cache::{default_js_cache_root, CacheMeta, JsCache};
#[cfg(feature = "load")]
use crate::rs2b0t_registry::{
    parse_registry_with_sources, persist_rs2b0t_root_at, script_file_path,
};
use crate::rs2b0t_registry::{ScriptKind, ScriptSource, SettingDef};

#[cfg(feature = "load")]
use super::shape::{
    catalog_unloadable, first_unloadable_for_card, is_reserved, resolve_api_family,
    resolve_sibling_modules,
};
use super::shape::{raw_content_fingerprint, ApiFamily, LoadShape};

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
        let fingerprint = match origin {
            Some(text) => raw_content_fingerprint(path, text),
            None => String::new(),
        };
        let (line, column) = extract_line_column(diagnostic);
        let sibling = extract_sibling(diagnostic);
        let stage = classify_stage(diagnostic, sibling.as_deref());
        Self {
            identity_key: crate::identity::card_identity_key(source, path, name),
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
        crate::identity::card_identity_id(self.source, &self.path, &self.name)
    }

    pub fn identity_key(&self) -> String {
        crate::identity::card_identity_key(self.source, &self.path, &self.name)
    }

    pub fn assignment(&self) -> vault::ScriptAssignment {
        crate::identity::card_assignment(self.source, &self.path, &self.name)
    }
}

/// Default persisted library path (`~/.274bot/js-scripts.json`).
pub fn default_js_store() -> PathBuf {
    crate::bot_file("js-scripts.json")
}

/// One persisted library record: only the name and the source path (the
/// source itself is re-read from disk on restore).
#[cfg(feature = "load")]
#[derive(serde::Serialize, serde::Deserialize)]
struct StoreEntry {
    name: String,
    path: String,
}

/// The out-of-tree JS library: picker cards for loaded files and the
/// `$RS2B0T` catalog, persisted to `store`, with origin bytes cached
/// under `cache`. Same `(source, name)` overwrites; only WalkTo is
/// reserved; non-bot shapes are rejected at Load.
#[cfg(feature = "load")]
pub struct JsLibrary {
    store: PathBuf,
    cache: JsCache,
    cards: Vec<JsCard>,
    /// Combined raw entry+sibling hashes keyed by identity, used to skip
    /// no-op reloads without transpiling.
    fingerprints: HashMap<String, String>,
    /// Retained per-identity load failures. One card's success does not
    /// clear another identity's record.
    failures: HashMap<String, LoadFailure>,
}

#[cfg(feature = "load")]
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
            fingerprints: HashMap::new(),
            failures: HashMap::new(),
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
            let settings_schema = crate::rs2b0t_registry::settings_schema_from_source(&origin);
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
        let settings_schema = crate::rs2b0t_registry::settings_schema_from_source(&origin);
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
                    && crate::identity::paths_match(&card.path.to_string_lossy(), &c.path))
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
        result: Result<(), crate::StartLoadError>,
    ) -> Result<(), String> {
        match result {
            Ok(()) => {
                self.clear_failure(&card.identity_key());
                Ok(())
            }
            Err(crate::StartLoadError::RuntimeLoad(diagnostic)) => {
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
                Err(diagnostic)
            }
            Err(crate::StartLoadError::Refused(diagnostic)) => Err(diagnostic),
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

    fn refresh_unrecorded(&mut self, source: ScriptSource, name: &str) -> Result<(), String> {
        let idx = self
            .cards
            .iter()
            .position(|c| c.source == source && c.name == name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?;
        let path = self.cards[idx].path.clone();
        let origin = std::fs::read_to_string(&path)
            .map_err(|e| format!("refresh {}: {e}", path.display()))?;
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
                .find(|c| c.source == source && crate::identity::paths_match(name, &c.path))
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

    /// Build a candidate without creating a V8 runtime. UI callers use this
    /// finite disk/transpile phase, then move [`PreparedCard::validate`] to a
    /// worker so hostile module evaluation never owns the UI thread.
    pub fn prepare_card_unvalidated(
        &mut self,
        source: ScriptSource,
        name: &str,
    ) -> Result<PreparedCard, String> {
        let prior = self.get(source, name).cloned();
        match self.prepare_card_unvalidated_unrecorded(source, name) {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                if let Some(card) = prior {
                    let origin = std::fs::read_to_string(&card.path).ok();
                    let family = origin
                        .as_deref()
                        .and_then(|text| resolve_api_family(text).ok().map(|(_, family)| family))
                        .or(Some(card.api_family));
                    self.note_err(source, &card.path, name, &error, origin.as_deref(), family);
                }
                Err(error)
            }
        }
    }

    /// Retain a worker-side validation failure against the exact bytes that
    /// were validated.
    pub fn record_prepared_failure(&mut self, prepared: &PreparedCard, diagnostic: &str) {
        let mut failure = LoadFailure::capture(
            prepared.card.source,
            &prepared.card.path,
            &prepared.card.name,
            diagnostic,
            Some(&prepared.card.origin),
            Some(prepared.card.api_family),
        );
        // PreparedCard::validate is the typed runtime boundary; diagnostic
        // wording does not decide whether this was a transpile failure.
        failure.stage = LoadStage::RuntimeLoad;
        self.record_failure(failure);
    }

    fn prepare_card_unvalidated_unrecorded(
        &self,
        source: ScriptSource,
        name: &str,
    ) -> Result<PreparedCard, String> {
        let card = self
            .get(source, name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?
            .clone();
        let origin = std::fs::read_to_string(&card.path)
            .map_err(|e| format!("prepare {}: {e}", card.path.display()))?;
        let (shape, api_family) =
            resolve_api_family(&origin).map_err(|e| format!("{name}: {e}"))?;
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        let cached = self.cache.get_or_transpile(
            &card.path,
            origin.as_bytes(),
            cache_meta(shape, source, api_family),
        )?;
        let unloadable = catalog_unloadable(
            &card.name,
            source,
            &cached.sha256,
            &card.path,
            first_unloadable_for_card(&origin, &card.path),
        );
        if let Some(reason) = &unloadable {
            return Err(format!("unloadable import: {reason}"));
        }
        let siblings = resolve_sibling_modules(
            &card.path,
            &origin,
            &self.cache,
            cache_meta(shape, source, api_family),
        )?;
        let fingerprint = raw_content_fingerprint(&card.path, &origin);
        let mut prepared = card;
        prepared.origin = origin;
        prepared.shape = shape;
        prepared.js = cached.js;
        prepared.kind = shape_to_kind(shape);
        prepared.sha256 = cached.sha256;
        prepared.unloadable = unloadable;
        prepared.api_family = api_family;
        prepared.settings_schema =
            crate::rs2b0t_registry::settings_schema_from_source(&prepared.origin);
        Ok(PreparedCard {
            card: prepared,
            siblings,
            fingerprint,
        })
    }

    pub fn commit_prepared(&mut self, prepared: PreparedCard) -> Result<JsCard, String> {
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
        self.clear_failure(&key);
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

/// Candidate prepared while old library cards and isolates stay intact.
#[derive(Debug, Clone)]
pub struct PreparedCard {
    pub card: JsCard,
    pub siblings: Vec<(String, String)>,
    pub fingerprint: String,
}

#[cfg(feature = "load")]
impl PreparedCard {
    /// Evaluate this candidate in a bounded throwaway isolate. Callers that
    /// serve UI actions must run this method on a worker thread.
    pub fn validate(&self) -> Result<(), String> {
        crate::LoadIsolate::validate_source(&self.card.js, self.card.shape, &self.siblings)
            .map_err(|error| format!("prepare {}: {error}", self.card.name))
    }
}

#[cfg(feature = "load")]
fn shape_to_kind(shape: LoadShape) -> ScriptKind {
    match shape {
        LoadShape::NativeTick => ScriptKind::NativeTick,
        LoadShape::CompatDefineBot | LoadShape::CompatClass => ScriptKind::Compat,
        LoadShape::Reject => ScriptKind::Compat,
    }
}

#[cfg(feature = "load")]
fn shape_label(shape: LoadShape) -> &'static str {
    match shape {
        LoadShape::CompatDefineBot => "CompatDefineBot",
        LoadShape::CompatClass => "CompatClass",
        LoadShape::NativeTick => "NativeTick",
        LoadShape::Reject => "Reject",
    }
}

#[cfg(feature = "load")]
fn cache_meta(shape: LoadShape, source: ScriptSource, family: ApiFamily) -> CacheMeta {
    CacheMeta {
        kind: shape_to_kind(shape),
        source,
        shape: Some(shape_label(shape).into()),
        api_family: Some(family.as_str().into()),
    }
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
