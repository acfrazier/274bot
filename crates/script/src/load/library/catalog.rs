use super::*;

#[cfg(feature = "load")]
impl JsLibrary {
    /// Same-dir siblings plus the registered root's `SHOP_DB` file.
    fn complete_catalog_sources(sources: &mut HashMap<String, String>, root: &Path) {
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
        crate::rs2b0t_registry::insert_shop_db_from_root(sources, root);
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
        Self::complete_catalog_sources(&mut sources, root);
        let cards = parse_registry_with_sources(&index_ts, &sources)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut n = 0;
        for card in &cards {
            if is_reserved(&card.name) {
                continue;
            }
            let Some(path) = script_file_path(root, &card.rel_path) else {
                self.note_err(
                    ScriptSource::Catalog,
                    Path::new(&card.rel_path),
                    &card.name,
                    &format!("missing {}", card.rel_path),
                    None,
                    None,
                );
                continue;
            };
            let origin = match std::fs::read_to_string(&path) {
                Ok(origin) => origin,
                Err(e) => {
                    self.note_err(
                        ScriptSource::Catalog,
                        &path,
                        &card.name,
                        &format!("unreadable {}: {e}", path.display()),
                        None,
                        None,
                    );
                    continue;
                }
            };
            let (shape, api_family) = match resolve_api_family(&origin) {
                Ok(pair) => pair,
                Err(e) => {
                    self.note_err(
                        ScriptSource::Catalog,
                        &path,
                        &card.name,
                        &e,
                        Some(&origin),
                        None,
                    );
                    continue;
                }
            };
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
                api_family,
            });
            n += 1;
            if let Some(last) = self.cards.last().cloned() {
                self.remember_fingerprint(&last);
                self.note_card_outcome(&last);
            }
        }
        let _ = persist_rs2b0t_root_at(root, path_file);
        Ok(n)
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
        let mut failed_names = HashSet::new();
        for reg in &registry_cards {
            let Some(path) = script_file_path(root, &reg.rel_path) else {
                failed.push(LoadFailure::capture(
                    ScriptSource::Catalog,
                    Path::new(&reg.rel_path),
                    &reg.name,
                    &format!("missing {}", reg.rel_path),
                    None,
                    None,
                ));
                failed_names.insert(reg.name.clone());
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    sources.insert(reg.rel_path.clone(), text);
                }
                Err(e) => {
                    failed.push(LoadFailure::capture(
                        ScriptSource::Catalog,
                        &path,
                        &reg.name,
                        &e.to_string(),
                        None,
                        None,
                    ));
                    failed_names.insert(reg.name.clone());
                }
            }
        }
        Self::complete_catalog_sources(&mut sources, root);
        let cards = parse_registry_with_sources(&index_ts, &sources)
            .map_err(|e| format!("$RS2B0T registry {}: {e}", index.display()))?;
        let mut incoming: HashMap<String, (crate::rs2b0t_registry::RegistryCard, PathBuf, String)> =
            HashMap::new();
        for card in cards {
            if is_reserved(&card.name) {
                continue;
            }
            let Some(path) = script_file_path(root, &card.rel_path) else {
                failed.push(LoadFailure::capture(
                    ScriptSource::Catalog,
                    Path::new(&card.rel_path),
                    &card.name,
                    &format!("missing {}", card.rel_path),
                    None,
                    None,
                ));
                failed_names.insert(card.name.clone());
                continue;
            };
            let Ok(origin) = std::fs::read_to_string(&path) else {
                failed.push(LoadFailure::capture(
                    ScriptSource::Catalog,
                    &path,
                    &card.name,
                    &format!("unreadable {}", path.display()),
                    None,
                    None,
                ));
                failed_names.insert(card.name.clone());
                continue;
            };
            match resolve_api_family(&origin) {
                Err(e) => {
                    failed.push(LoadFailure::capture(
                        ScriptSource::Catalog,
                        &path,
                        &card.name,
                        &e,
                        Some(&origin),
                        None,
                    ));
                    failed_names.insert(card.name.clone());
                    continue;
                }
                Ok((LoadShape::Reject, _)) => continue,
                Ok(_) => {}
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
            if !incoming.contains_key(name) && !failed_names.contains(name) {
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
        for failure in &failed {
            self.record_failure(failure.clone());
        }
        for name in &diff.added {
            let Some((card, path, origin)) = diff.incoming.get(name) else {
                continue;
            };
            let Ok((shape, api_family)) = resolve_api_family(origin) else {
                continue;
            };
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
                api_family,
            };
            self.remember_fingerprint(&js_card);
            self.note_card_outcome(&js_card);
            self.cards.push(js_card);
            added += 1;
        }
        for name in &diff.removed {
            let key =
                crate::identity::card_identity_key(ScriptSource::Catalog, Path::new(""), name);
            self.fingerprints.remove(&key);
            self.clear_failure(&key);
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
}

/// Catalog refresh diff. `incoming` is the parsed replacement set, read
/// only by the cache-backed apply step.
pub struct CatalogDiff {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<LoadFailure>,
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
    pub failed: Vec<LoadFailure>,
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

    pub fn named_failure_output(&self) -> String {
        format_load_failures(&self.failed)
    }
}
