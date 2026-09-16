//! JS card library: picker cards, catalog apply, and prepare/commit.

#[cfg(feature = "load")]
use std::collections::{HashMap, HashSet};
#[cfg(feature = "load")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "load")]
use crate::js_cache::{default_js_cache_root, CacheMeta, JsCache};
#[cfg(feature = "load")]
use crate::rs2b0t_registry::{
    parse_registry_with_sources, persist_rs2b0t_root_at, script_file_path,
};
use crate::rs2b0t_registry::{ScriptKind, ScriptSource, SettingDef};

use super::shape::LoadShape;
#[cfg(feature = "load")]
use super::shape::{
    catalog_unloadable, detect_shape, first_unloadable_for_card, is_reserved,
    raw_content_fingerprint, resolve_sibling_modules,
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
        let sibling_rels: Vec<String> = sources
            .iter()
            .flat_map(|(rel, text)| crate::rs2b0t_registry::same_dir_import_rels(rel, text))
            .collect();
        for sib in sibling_rels {
            if sources.contains_key(&sib) {
                continue;
            }
            let Some(path) = script_file_path(root, &sib) else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                sources.insert(sib, text);
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
            let unloadable = catalog_unloadable(
                &card.name,
                ScriptSource::Catalog,
                &sha256,
                &path,
                first_unloadable_for_card(&origin, &path),
            );
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
                unloadable,
            });
            n += 1;
            if let Some(last) = self.cards.last().cloned() {
                self.remember_fingerprint(&last);
            }
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

    /// Transpile/validate a candidate without replacing the live card.
    /// Old isolates keep the previous registration until [`JsLibrary::commit_prepared`].
    pub fn prepare_card(&self, source: ScriptSource, name: &str) -> Result<PreparedCard, String> {
        let card = self
            .get(source, name)
            .ok_or_else(|| format!("no card ({source:?}, {name})"))?
            .clone();
        let origin = std::fs::read_to_string(&card.path)
            .map_err(|e| format!("prepare {}: {e}", card.path.display()))?;
        let shape = detect_shape(&origin);
        if shape == LoadShape::Reject {
            return Err(format!("not a bot shape: {name}"));
        }
        let cached = self.cache.get_or_transpile(
            &card.path,
            origin.as_bytes(),
            CacheMeta {
                kind: shape_to_kind(shape),
                source,
                shape: Some(shape_label(shape).into()),
            },
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
            CacheMeta {
                kind: shape_to_kind(shape),
                source,
                shape: Some(shape_label(shape).into()),
            },
        )?;
        let fingerprint = raw_content_fingerprint(&card.path, &origin);
        #[cfg(feature = "load")]
        crate::LoadIsolate::validate_source(&cached.js, shape, &siblings)
            .map_err(|e| format!("prepare {name}: {e}"))?;
        let mut prepared = card;
        prepared.origin = origin;
        prepared.shape = shape;
        prepared.js = cached.js;
        prepared.kind = shape_to_kind(shape);
        prepared.sha256 = cached.sha256;
        prepared.unloadable = unloadable;
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
        self.fingerprints.insert(key, prepared.fingerprint.clone());
        if prepared.card.source == ScriptSource::File {
            self.persist_file_cards(&self.cards)?;
        }
        Ok(prepared.card)
    }

    /// Diff `$RS2B0T` against registered catalog cards without clearing them.
    /// Does not rewrite the persisted catalog path.
    pub fn diff_catalog(&self, root: &Path) -> Result<CatalogDiff, String> {
        let index = crate::rs2b0t_registry::registry_index_path(root);
        let index_ts = std::fs::read_to_string(&index)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let registry_cards = parse_registry_with_sources(&index_ts, &HashMap::new())
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut sources = HashMap::new();
        let mut failed = Vec::new();
        for reg in &registry_cards {
            let Some(path) = script_file_path(root, &reg.rel_path) else {
                failed.push((reg.name.clone(), format!("missing {}", reg.rel_path)));
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    sources.insert(reg.rel_path.clone(), text);
                }
                Err(e) => failed.push((reg.name.clone(), e.to_string())),
            }
        }
        let sibling_rels: Vec<String> = sources
            .iter()
            .flat_map(|(rel, text)| crate::rs2b0t_registry::same_dir_import_rels(rel, text))
            .collect();
        for sib in sibling_rels {
            if sources.contains_key(&sib) {
                continue;
            }
            let Some(path) = script_file_path(root, &sib) else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                sources.insert(sib, text);
            }
        }
        let cards = parse_registry_with_sources(&index_ts, &sources)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut incoming: HashMap<String, (crate::rs2b0t_registry::RegistryCard, PathBuf, String)> =
            HashMap::new();
        for card in cards {
            if is_reserved(&card.name) {
                continue;
            }
            let Some(path) = script_file_path(root, &card.rel_path) else {
                failed.push((card.name, format!("missing {}", card.rel_path)));
                continue;
            };
            let Ok(origin) = std::fs::read_to_string(&path) else {
                failed.push((card.name, format!("unreadable {}", path.display())));
                continue;
            };
            if detect_shape(&origin) == LoadShape::Reject {
                continue;
            }
            incoming.insert(card.name.clone(), (card, path, origin));
        }
        let existing: HashSet<String> = self
            .cards
            .iter()
            .filter(|c| c.source == ScriptSource::Catalog)
            .map(|c| c.name.clone())
            .collect();
        let mut added = Vec::new();
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        for name in incoming.keys() {
            if existing.contains(name) {
                let Some((_, path, origin)) = incoming.get(name) else {
                    continue;
                };
                let now = raw_content_fingerprint(path, origin);
                let key = crate::identity::card_identity_key(ScriptSource::Catalog, path, name);
                if self.fingerprints.get(&key).map(String::as_str) != Some(now.as_str()) {
                    changed.push(name.clone());
                }
            } else {
                added.push(name.clone());
            }
        }
        for name in &existing {
            if !incoming.contains_key(name) {
                removed.push(name.clone());
            }
        }
        added.sort();
        changed.sort();
        removed.sort();
        Ok(CatalogDiff {
            added,
            changed,
            removed,
            failed,
            incoming,
        })
    }

    /// Apply an already-computed catalog diff. Added cards register as
    /// unstarted; removed catalog cards are dropped from the library (running
    /// isolates keep their frozen source). Changed cards are **not** mutated
    /// here — the caller must [`JsLibrary::commit_prepared`] only after a
    /// successful prepare so a failed card keeps its old registration.
    /// One failed card does not block others.
    pub fn apply_catalog_diff(&mut self, diff: CatalogDiff) -> CatalogApplyReport {
        let mut added = 0usize;
        let mut removed = 0usize;
        let failed = diff.failed.clone();
        for name in &diff.added {
            let Some((card, path, origin)) = diff.incoming.get(name) else {
                continue;
            };
            let shape = detect_shape(origin);
            if shape == LoadShape::Reject {
                continue;
            }
            let sha256 = JsCache::origin_sha(origin.as_bytes());
            let unloadable = catalog_unloadable(
                &card.name,
                ScriptSource::Catalog,
                &sha256,
                path,
                first_unloadable_for_card(origin, path),
            );
            self.cards
                .retain(|c| !(c.source == ScriptSource::Catalog && c.name == card.name));
            let js_card = JsCard {
                name: card.name.clone(),
                path: path.clone(),
                shape,
                origin: origin.clone(),
                js: String::new(),
                kind: card.kind,
                source: ScriptSource::Catalog,
                sha256,
                description: card.description.clone(),
                category: card.category.clone(),
                tags: card.tags.clone(),
                settings_schema: card.settings_schema.clone(),
                unloadable,
            };
            self.remember_fingerprint(&js_card);
            self.cards.push(js_card);
            added += 1;
        }
        for name in &diff.removed {
            let key =
                crate::identity::card_identity_key(ScriptSource::Catalog, Path::new(""), name);
            self.fingerprints.remove(&key);
            self.cards
                .retain(|c| !(c.source == ScriptSource::Catalog && c.name == *name));
            removed += 1;
        }
        CatalogApplyReport {
            added,
            changed: 0,
            removed,
            failed,
        }
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

/// Catalog refresh diff. `incoming` is the parsed replacement set, read
/// only by the cache-backed apply step.
pub struct CatalogDiff {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<(String, String)>,
    #[cfg(feature = "load")]
    incoming: HashMap<String, (crate::rs2b0t_registry::RegistryCard, PathBuf, String)>,
}

impl CatalogDiff {
    pub fn is_noop(&self) -> bool {
        self.added.is_empty()
            && self.changed.is_empty()
            && self.removed.is_empty()
            && self.failed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogApplyReport {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    pub failed: Vec<(String, String)>,
}

impl CatalogApplyReport {
    pub fn summary(&self) -> String {
        if self.added == 0 && self.changed == 0 && self.removed == 0 && self.failed.is_empty() {
            return crate::identity::NOTHING_CHANGED_CATALOG.to_string();
        }
        let mut parts = Vec::new();
        if self.added > 0 {
            parts.push(format!("added {}", self.added));
        }
        if self.changed > 0 {
            parts.push(format!("changed {}", self.changed));
        }
        if self.removed > 0 {
            parts.push(format!("removed {}", self.removed));
        }
        if !self.failed.is_empty() {
            parts.push(format!("failed {}", self.failed.len()));
        }
        parts.join(", ")
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
