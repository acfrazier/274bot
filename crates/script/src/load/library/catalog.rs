use super::*;

#[cfg(feature = "load")]
const MAX_CATALOG_INPUT_BYTES: usize = 64 * 1024 * 1024;
#[cfg(feature = "load")]
const MAX_CATALOG_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
#[cfg(feature = "load")]
const MAX_CATALOG_SOURCE_BYTES: usize = MAX_CATALOG_INPUT_BYTES - MAX_CATALOG_SNAPSHOT_BYTES;
#[cfg(feature = "load")]
const MAX_CATALOG_SCRATCH_BYTES: usize = 32 * 1024 * 1024;
#[cfg(feature = "load")]
const MAX_CATALOG_RESULT_BYTES: usize = 32 * 1024 * 1024;
#[cfg(feature = "load")]
const MAX_CATALOG_INDEX_BYTES: usize = 8 * 1024 * 1024;
#[cfg(feature = "load")]
const MAX_CATALOG_CARDS: usize = 4096;
#[cfg(feature = "load")]
const MAX_CATALOG_FILES: usize = 8192;

/// Immutable catalog owner snapshot for the one admitted catalogue demand.
/// It contains card metadata and stored hashes only, never the live library,
/// source text, or compiled JavaScript.
#[cfg(feature = "load")]
#[derive(Debug)]
pub struct CatalogPreparationSnapshot {
    root: PathBuf,
    cache_root: PathBuf,
    existing: HashMap<String, CardPreparationTemplate>,
    fingerprints: HashMap<String, String>,
    snapshot_bytes: usize,
}

#[cfg(feature = "load")]
#[derive(Debug)]
pub struct CatalogPreparationResult {
    pub diff: CatalogDiff,
    pub prepared: Vec<PreparedCard>,
    pub warmed: Vec<PreparedCard>,
    pub prepare_failed: Vec<LoadFailure>,
}

#[cfg(feature = "load")]
#[derive(Debug)]
struct CatalogIncoming {
    card: crate::rs2b0t_registry::RegistryCard,
    path: PathBuf,
    origin: Option<String>,
    addition: Option<CatalogAddedCard>,
}

#[cfg(feature = "load")]
#[derive(Debug)]
struct CatalogAddedCard {
    shape: LoadShape,
    api_family: ApiFamily,
    sha256: String,
    unloadable: Option<String>,
}
#[cfg(feature = "load")]
impl JsLibrary {
    /// Capture bounded metadata needed to diff and prepare the current
    /// catalogue without carrying the live library into WorkAdmission.
    pub fn catalog_preparation_snapshot(
        &self,
        root: &Path,
    ) -> Result<CatalogPreparationSnapshot, String> {
        let mut existing = HashMap::new();
        let mut fingerprints = HashMap::new();
        let mut snapshot_bytes = std::mem::size_of::<CatalogPreparationSnapshot>();
        add_capacity(&mut snapshot_bytes, root.as_os_str().len())
            .ok_or_else(|| "catalog snapshot is too large".to_string())?;
        add_capacity(&mut snapshot_bytes, self.cache.root().as_os_str().len())
            .ok_or_else(|| "catalog snapshot is too large".to_string())?;
        for card in self
            .cards
            .iter()
            .filter(|card| card.source == ScriptSource::Catalog)
        {
            let template = CardPreparationTemplate::from_card(card);
            add_capacity(
                &mut snapshot_bytes,
                template
                    .retained_bytes()
                    .ok_or_else(|| "catalog card metadata is too large".to_string())?,
            )
            .ok_or_else(|| "catalog snapshot is too large".to_string())?;
            add_capacity(&mut snapshot_bytes, card.name.capacity())
                .ok_or_else(|| "catalog snapshot is too large".to_string())?;
            let identity_key = card.identity_key();
            if let Some(fingerprint) = self.fingerprints.get(&identity_key) {
                add_capacity(
                    &mut snapshot_bytes,
                    identity_key
                        .capacity()
                        .checked_add(fingerprint.capacity())
                        .ok_or_else(|| "catalog snapshot is too large".to_string())?,
                )
                .ok_or_else(|| "catalog snapshot is too large".to_string())?;
                fingerprints.insert(identity_key, fingerprint.clone());
            }
            existing.entry(card.name.clone()).or_insert(template);
        }
        add_capacity(
            &mut snapshot_bytes,
            catalog_map_storage_bytes(
                existing.capacity(),
                std::mem::size_of::<(String, CardPreparationTemplate)>(),
            )
            .ok_or_else(|| "catalog snapshot is too large".to_string())?,
        )
        .ok_or_else(|| "catalog snapshot is too large".to_string())?;
        add_capacity(
            &mut snapshot_bytes,
            catalog_map_storage_bytes(
                fingerprints.capacity(),
                std::mem::size_of::<(String, String)>(),
            )
            .ok_or_else(|| "catalog snapshot is too large".to_string())?,
        )
        .ok_or_else(|| "catalog snapshot is too large".to_string())?;
        if snapshot_bytes > MAX_CATALOG_SNAPSHOT_BYTES {
            return Err("catalog card metadata exceeds the 8 MiB snapshot bound".into());
        }
        Ok(CatalogPreparationSnapshot {
            root: root.to_path_buf(),
            cache_root: self.cache.root().to_path_buf(),
            existing,
            fingerprints,
            snapshot_bytes,
        })
    }
}

#[cfg(feature = "load")]
impl CatalogPreparationSnapshot {
    pub const fn input_bytes(&self) -> usize {
        MAX_CATALOG_INPUT_BYTES
    }

    pub const fn scratch_bytes(&self) -> usize {
        MAX_CATALOG_SCRATCH_BYTES
    }

    pub const fn result_bytes(&self) -> usize {
        MAX_CATALOG_RESULT_BYTES
    }

    fn source_limit(&self) -> usize {
        MAX_CATALOG_SOURCE_BYTES.min(
            MAX_CATALOG_INPUT_BYTES
                .checked_sub(self.snapshot_bytes)
                .unwrap_or(0),
        )
    }

    pub fn diff(&self) -> Result<CatalogDiff, String> {
        let source_limit = self.source_limit();
        let index = crate::rs2b0t_registry::registry_index_path(&self.root);
        let index_limit = source_limit.min(MAX_CATALOG_INDEX_BYTES);
        let index_ts = vault::read_regular_file(&index, index_limit as u64)
            .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
        let mut used = index_ts.capacity();
        let registry_index = crate::rs2b0t_registry::RegistryIndex::parse(&index_ts)
            .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
        let registrations = registry_index
            .registrations()
            .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
        if registrations.len() > MAX_CATALOG_CARDS {
            return Err("catalog has more than 4096 registrations".into());
        }
        for card in &registrations {
            add_catalog_bytes(
                &mut used,
                registry_card_bytes(card).ok_or("catalog registry size overflow")?,
            )
            .map_err(|_| "catalog registry metadata exceeds its 56 MiB bound")?;
        }
        if used > source_limit {
            return Err("catalog inputs exceed the WorkAdmission byte bound".into());
        }

        let mut sources = HashMap::new();
        let mut failed = Vec::new();
        let mut failed_names = HashSet::new();
        let mut source_files = 0usize;
        for card in &registrations {
            source_files += 1;
            if source_files > MAX_CATALOG_FILES {
                return Err("catalog source graph exceeds 8192 files".into());
            }
            let Some(path) = script_file_path(&self.root, &card.rel_path) else {
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
            match read_catalog_source(&path, &mut used, source_limit) {
                Ok(text) => {
                    add_catalog_source(&mut sources, &card.rel_path, text, &mut used, source_limit)
                        .map_err(|error| error.message())?
                }
                Err(CatalogSourceError::Aggregate) => {
                    return Err("catalog sources exceed the 56 MiB WorkAdmission bound".into());
                }
                Err(CatalogSourceError::File(error)) => {
                    failed.push(LoadFailure::capture(
                        ScriptSource::Catalog,
                        &path,
                        &card.name,
                        &error,
                        None,
                        None,
                    ));
                    failed_names.insert(card.name.clone());
                }
            }
        }
        self.complete_catalog_sources(&mut sources, &mut used, source_limit, &mut source_files)?;
        let mut diff = CatalogDiff {
            added: Vec::new(),
            changed: Vec::new(),
            removed: Vec::new(),
            failed,
            current_fingerprints: HashMap::new(),
            incoming: HashMap::new(),
        };
        ensure_catalog_diff_bound(&diff)?;
        for registration in registrations {
            if is_reserved(&registration.name) {
                continue;
            }
            let Some(path) = script_file_path(&self.root, &registration.rel_path) else {
                diff.failed.push(LoadFailure::capture(
                    ScriptSource::Catalog,
                    Path::new(&registration.rel_path),
                    &registration.name,
                    &format!("missing {}", registration.rel_path),
                    None,
                    None,
                ));
                failed_names.insert(registration.name.clone());
                ensure_catalog_diff_bound(&diff)?;
                continue;
            };
            let Some(origin) = sources.get(&registration.rel_path) else {
                diff.failed.push(LoadFailure::capture(
                    ScriptSource::Catalog,
                    &path,
                    &registration.name,
                    &format!("unreadable {}", path.display()),
                    None,
                    None,
                ));
                failed_names.insert(registration.name.clone());
                ensure_catalog_diff_bound(&diff)?;
                continue;
            };
            let (shape, api_family) = match resolve_api_family(origin) {
                Err(error) => {
                    diff.failed.push(LoadFailure::capture_bounded(
                        ScriptSource::Catalog,
                        &path,
                        &registration.name,
                        &error,
                        origin,
                        None,
                        MAX_PREPARATION_SOURCE_BYTES,
                    ));
                    failed_names.insert(registration.name.clone());
                    ensure_catalog_diff_bound(&diff)?;
                    continue;
                }
                Ok((LoadShape::Reject, _)) => continue,
                Ok(resolved) => resolved,
            };
            let fingerprint = match raw_content_fingerprint_bounded(
                &path,
                origin,
                MAX_PREPARATION_SOURCE_BYTES,
            ) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    diff.failed.push(LoadFailure::capture_bounded(
                        ScriptSource::Catalog,
                        &path,
                        &registration.name,
                        &error,
                        origin,
                        Some(api_family),
                        MAX_PREPARATION_SOURCE_BYTES,
                    ));
                    failed_names.insert(registration.name.clone());
                    ensure_catalog_diff_bound(&diff)?;
                    continue;
                }
            };
            let identity_key = crate::identity::card_identity_key(
                ScriptSource::Catalog,
                &path,
                &registration.name,
            );
            let is_added = !self.existing.contains_key(&registration.name);
            let is_changed = !is_added
                && self.fingerprints.get(&identity_key).map(String::as_str)
                    != Some(fingerprint.as_str());
            diff.current_fingerprints
                .insert(registration.name.clone(), fingerprint);
            ensure_catalog_diff_bound(&diff)?;
            if !is_added && !is_changed {
                continue;
            }
            let addition = if is_added {
                let sha256 = JsCache::origin_sha(origin.as_bytes());
                let first_unloadable = match first_unloadable_for_card_bounded(
                    origin,
                    &path,
                    MAX_PREPARATION_SOURCE_BYTES,
                ) {
                    Ok(first) => first,
                    Err(error) => {
                        diff.failed.push(LoadFailure::capture_bounded(
                            ScriptSource::Catalog,
                            &path,
                            &registration.name,
                            &error,
                            origin,
                            Some(api_family),
                            MAX_PREPARATION_SOURCE_BYTES,
                        ));
                        failed_names.insert(registration.name.clone());
                        ensure_catalog_diff_bound(&diff)?;
                        continue;
                    }
                };
                let unloadable = catalog_unloadable(
                    &registration.name,
                    ScriptSource::Catalog,
                    &sha256,
                    &path,
                    first_unloadable,
                );
                Some(CatalogAddedCard {
                    shape,
                    api_family,
                    sha256,
                    unloadable,
                })
            } else {
                None
            };
            let card = registry_index
                .card_with_sources_bounded(
                    &sources,
                    &registration.name,
                    MAX_PREPARATION_SOURCE_BYTES,
                )
                .map_err(|error| format!("catalog {}: {error}", registration.name))?;
            if is_added {
                diff.added.push(registration.name.clone());
            } else {
                diff.changed.push(registration.name.clone());
            }
            diff.incoming.insert(
                registration.name.clone(),
                CatalogIncoming {
                    card,
                    path,
                    origin: is_added.then(|| origin.clone()),
                    addition,
                },
            );
            ensure_catalog_diff_bound(&diff)?;
        }
        for name in self.existing.keys() {
            if !diff.current_fingerprints.contains_key(name) && !failed_names.contains(name) {
                diff.removed.push(name.clone());
                ensure_catalog_diff_bound(&diff)?;
            }
        }
        diff.added.sort();
        diff.changed.sort();
        diff.removed.sort();
        ensure_catalog_diff_bound(&diff)?;
        Ok(diff)
    }

    pub fn prepare(&self) -> Result<CatalogPreparationResult, String> {
        let mut diff = self.diff()?;
        let changed_len = diff.changed.len();
        let added_len = diff.added.len();
        let candidate_count = changed_len + added_len;
        let mut prepared = Vec::with_capacity(diff.changed.len());
        let mut warmed = Vec::with_capacity(diff.added.len());
        let mut prepare_failed = Vec::with_capacity(candidate_count);
        let mut unchanged_changed = Vec::new();
        let mut result_bytes = catalog_preparation_result_bytes(
            &diff,
            &prepared,
            prepared.capacity(),
            &warmed,
            warmed.capacity(),
            &prepare_failed,
            prepare_failed.capacity(),
        )
        .ok_or_else(|| "catalog result size overflow".to_string())?;
        if result_bytes > MAX_CATALOG_RESULT_BYTES {
            return Err("prepared catalog result exceeds the 32 MiB result bound".into());
        }
        for index in 0..candidate_count {
            let is_added = index >= changed_len;
            let name = if is_added {
                &diff.added[index - changed_len]
            } else {
                &diff.changed[index]
            };
            let incoming = diff
                .incoming
                .get(name)
                .ok_or_else(|| format!("catalog {name}: parsed registration is missing"))?;
            let template = if is_added {
                CardPreparationTemplate::from_registry_card(&incoming.card, incoming.path.clone())
            } else {
                self.existing
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("catalog {name}: existing card snapshot is missing"))?
            };
            let identity_key =
                crate::identity::card_identity_key(ScriptSource::Catalog, &incoming.path, name);
            let snapshot = CardPreparationSnapshot {
                template,
                cache_root: self.cache_root.clone(),
                catalog_root: Some(self.root.clone()),
                stored_fingerprint: self.fingerprints.get(&identity_key).cloned(),
                confirmation_fingerprint: None,
                catalog_settings_schema: Some(incoming.card.settings_schema.clone()),
                allow_unloadable: false,
            };
            match snapshot.prepare() {
                Ok(CardPreparationOutcome::Prepared(card)) => {
                    let bytes = prepared_card_retained_bytes(&card)
                        .ok_or_else(|| "prepared catalog card size overflow".to_string())?;
                    add_catalog_bytes(&mut result_bytes, bytes)
                        .map_err(|_| "prepared catalog result size overflow")?;
                    if result_bytes > MAX_CATALOG_RESULT_BYTES {
                        return Err(
                            "prepared catalog result exceeds the 32 MiB result bound".into()
                        );
                    }
                    if is_added {
                        warmed.push(card);
                    } else {
                        prepared.push(card);
                    }
                }
                Ok(CardPreparationOutcome::Failed { failure, .. }) => {
                    let bytes = load_failure_retained_bytes(&failure)
                        .ok_or_else(|| "catalog failure size overflow".to_string())?;
                    add_catalog_bytes(&mut result_bytes, bytes)
                        .map_err(|_| "catalog failure size overflow")?;
                    if result_bytes > MAX_CATALOG_RESULT_BYTES {
                        return Err(
                            "prepared catalog result exceeds the 32 MiB result bound".into()
                        );
                    }
                    prepare_failed.push(failure);
                }
                Ok(CardPreparationOutcome::Unchanged) if !is_added => {
                    unchanged_changed.push(index);
                }
                Ok(CardPreparationOutcome::Unchanged | CardPreparationOutcome::Confirmed) => {}
                Err(error) => {
                    let failure = LoadFailure::capture(
                        ScriptSource::Catalog,
                        &incoming.path,
                        name,
                        &error,
                        None,
                        None,
                    );
                    let bytes = load_failure_retained_bytes(&failure)
                        .ok_or_else(|| "catalog failure size overflow".to_string())?;
                    add_catalog_bytes(&mut result_bytes, bytes)
                        .map_err(|_| "catalog failure size overflow")?;
                    if result_bytes > MAX_CATALOG_RESULT_BYTES {
                        return Err(
                            "prepared catalog result exceeds the 32 MiB result bound".into()
                        );
                    }
                    prepare_failed.push(failure);
                }
            }
        }
        for index in unchanged_changed.iter().copied() {
            let name = &diff.changed[index];
            if let Some(incoming) = diff.incoming.get(name) {
                let identity_key =
                    crate::identity::card_identity_key(ScriptSource::Catalog, &incoming.path, name);
                if let Some(fingerprint) = self.fingerprints.get(&identity_key) {
                    if let Some(current) = diff.current_fingerprints.get_mut(name) {
                        *current = fingerprint.clone();
                    }
                } else {
                    diff.current_fingerprints.remove(name);
                }
            }
            diff.incoming.remove(name);
        }
        let mut unchanged_pos = 0;
        let mut changed_pos = 0;
        diff.changed.retain(|_| {
            let unchanged = unchanged_changed.get(unchanged_pos).copied() == Some(changed_pos);
            if unchanged {
                unchanged_pos += 1;
            }
            changed_pos += 1;
            !unchanged
        });
        let result_bytes = catalog_preparation_result_bytes(
            &diff,
            &prepared,
            prepared.capacity(),
            &warmed,
            warmed.capacity(),
            &prepare_failed,
            prepare_failed.capacity(),
        )
        .ok_or_else(|| "catalog result size overflow".to_string())?;
        if result_bytes > MAX_CATALOG_RESULT_BYTES {
            return Err("prepared catalog result exceeds the 32 MiB result bound".into());
        }
        Ok(CatalogPreparationResult {
            diff,
            prepared,
            warmed,
            prepare_failed,
        })
    }

    /// Build a catalogue diff incrementally on the shared WorkAdmission
    /// worker. The returned closure owns only bounded preparation state.
    pub fn cooperative_diff(
        self: std::sync::Arc<Self>,
    ) -> impl FnMut(
        &mut api::selected::FamilyPreparation,
    ) -> crate::preparation::CooperativePreparationStep<Result<CatalogDiff, String>>
           + Send
           + 'static {
        let mut work = CatalogDiffWork::default();
        move |_| match work.step(&self) {
            Ok(Some((diff, _usage))) => crate::preparation::CooperativePreparationStep::Complete {
                output: Ok(diff),
                completed: work.completed,
                total: None,
            },
            Ok(None) => crate::preparation::CooperativePreparationStep::Continue {
                completed: work.completed,
                total: None,
            },
            Err(error) => crate::preparation::CooperativePreparationStep::Complete {
                output: Err(error),
                completed: work.completed,
                total: None,
            },
        }
    }

    /// Build a full catalogue preparation incrementally on WorkAdmission.
    pub fn cooperative_prepare(
        self: std::sync::Arc<Self>,
    ) -> impl FnMut(
        &mut api::selected::FamilyPreparation,
    ) -> crate::preparation::CooperativePreparationStep<
        Result<CatalogPreparationResult, String>,
    > + Send
           + 'static {
        let mut work = CatalogPreparationWork::default();
        move |_| match work.step(&self) {
            Ok(Some(result)) => crate::preparation::CooperativePreparationStep::Complete {
                output: Ok(result),
                completed: work.completed,
                total: None,
            },
            Ok(None) => crate::preparation::CooperativePreparationStep::Continue {
                completed: work.completed,
                total: None,
            },
            Err(error) => crate::preparation::CooperativePreparationStep::Complete {
                output: Err(error),
                completed: work.completed,
                total: None,
            },
        }
    }
    fn complete_catalog_sources(
        &self,
        sources: &mut HashMap<String, String>,
        used: &mut usize,
        source_limit: usize,
        source_files: &mut usize,
    ) -> Result<(), String> {
        complete_catalog_sources_bounded(&self.root, sources, used, source_limit, source_files)
    }
}

#[cfg(feature = "load")]
enum CatalogSourceError {
    File(String),
    Aggregate,
}

#[cfg(feature = "load")]
impl CatalogSourceError {
    fn message(self) -> String {
        match self {
            Self::File(error) => error,
            Self::Aggregate => "catalog sources exceed the WorkAdmission byte bound".into(),
        }
    }
}

#[cfg(feature = "load")]
#[derive(Debug, Default)]
struct CatalogDiffUsage {
    bytes: usize,
    added_capacity: usize,
    changed_capacity: usize,
    removed_capacity: usize,
    failed_capacity: usize,
    fingerprints_capacity: usize,
    incoming_capacity: usize,
}

#[cfg(feature = "load")]
impl CatalogDiffUsage {
    fn new(diff: &CatalogDiff) -> Result<Self, String> {
        let mut usage = Self {
            bytes: std::mem::size_of::<CatalogDiff>(),
            ..Self::default()
        };
        usage.sync_capacities(diff)?;
        for failure in &diff.failed {
            usage.add(
                load_failure_retained_bytes(failure)
                    .ok_or_else(|| "catalog failure size overflow".to_string())?,
            )?;
        }
        usage.ensure_bound()?;
        Ok(usage)
    }

    fn add(&mut self, bytes: usize) -> Result<(), String> {
        add_catalog_bytes(&mut self.bytes, bytes)
            .map_err(|_| "catalog diff size overflow".to_string())
    }

    fn replace(&mut self, old: usize, new: usize) -> Result<(), String> {
        if new >= old {
            self.add(new - old)
        } else {
            self.bytes = self
                .bytes
                .checked_sub(old - new)
                .ok_or_else(|| "catalog diff size accounting underflow".to_string())?;
            Ok(())
        }
    }

    fn sync_vec_capacity(
        &mut self,
        previous: usize,
        current: usize,
        item_bytes: usize,
    ) -> Result<usize, String> {
        let old_bytes = previous
            .checked_mul(item_bytes)
            .ok_or_else(|| "catalog diff size overflow".to_string())?;
        let new_bytes = current
            .checked_mul(item_bytes)
            .ok_or_else(|| "catalog diff size overflow".to_string())?;
        self.replace(old_bytes, new_bytes)?;
        Ok(current)
    }

    fn sync_map_capacity(
        &mut self,
        previous: usize,
        current: usize,
        item_bytes: usize,
    ) -> Result<usize, String> {
        let old_bytes = catalog_map_storage_bytes(previous, item_bytes)
            .ok_or_else(|| "catalog diff size overflow".to_string())?;
        let new_bytes = catalog_map_storage_bytes(current, item_bytes)
            .ok_or_else(|| "catalog diff size overflow".to_string())?;
        self.replace(old_bytes, new_bytes)?;
        Ok(current)
    }

    fn sync_capacities(&mut self, diff: &CatalogDiff) -> Result<(), String> {
        self.added_capacity = self.sync_vec_capacity(
            self.added_capacity,
            diff.added.capacity(),
            std::mem::size_of::<String>(),
        )?;
        self.changed_capacity = self.sync_vec_capacity(
            self.changed_capacity,
            diff.changed.capacity(),
            std::mem::size_of::<String>(),
        )?;
        self.removed_capacity = self.sync_vec_capacity(
            self.removed_capacity,
            diff.removed.capacity(),
            std::mem::size_of::<String>(),
        )?;
        self.failed_capacity = self.sync_vec_capacity(
            self.failed_capacity,
            diff.failed.capacity(),
            std::mem::size_of::<LoadFailure>(),
        )?;
        self.fingerprints_capacity = self.sync_map_capacity(
            self.fingerprints_capacity,
            diff.current_fingerprints.capacity(),
            std::mem::size_of::<(String, String)>(),
        )?;
        self.incoming_capacity = self.sync_map_capacity(
            self.incoming_capacity,
            diff.incoming.capacity(),
            std::mem::size_of::<(String, CatalogIncoming)>(),
        )?;
        Ok(())
    }

    fn ensure_bound(&self) -> Result<(), String> {
        if self.bytes > MAX_CATALOG_RESULT_BYTES {
            Err("catalog diff exceeds the 32 MiB result bound".into())
        } else {
            Ok(())
        }
    }

    fn push_name(
        &mut self,
        diff: &mut CatalogDiff,
        list: CatalogNameList,
        name: String,
    ) -> Result<(), String> {
        let item_bytes = name.capacity();
        match list {
            CatalogNameList::Added => diff.added.push(name),
            CatalogNameList::Changed => diff.changed.push(name),
            CatalogNameList::Removed => diff.removed.push(name),
        }
        self.sync_capacities(diff)?;
        self.add(item_bytes)?;
        self.ensure_bound()
    }

    fn push_failure(&mut self, diff: &mut CatalogDiff, failure: LoadFailure) -> Result<(), String> {
        let bytes = load_failure_retained_bytes(&failure)
            .ok_or_else(|| "catalog failure size overflow".to_string())?;
        diff.failed.push(failure);
        self.sync_capacities(diff)?;
        self.add(bytes)?;
        self.ensure_bound()
    }

    fn set_fingerprint(
        &mut self,
        diff: &mut CatalogDiff,
        name: &str,
        fingerprint: String,
    ) -> Result<(), String> {
        let old_bytes = match diff.current_fingerprints.get_key_value(name) {
            Some((key, value)) => key
                .capacity()
                .checked_add(value.capacity())
                .ok_or_else(|| "catalog diff size overflow".to_string())?,
            None => 0,
        };
        diff.current_fingerprints
            .insert(name.to_owned(), fingerprint);
        let new_bytes = diff
            .current_fingerprints
            .get_key_value(name)
            .and_then(|(key, value)| key.capacity().checked_add(value.capacity()))
            .ok_or_else(|| "catalog fingerprint insertion disappeared".to_string())?;
        self.replace(old_bytes, new_bytes)?;
        self.sync_capacities(diff)?;
        self.ensure_bound()
    }

    fn set_incoming(
        &mut self,
        diff: &mut CatalogDiff,
        name: &str,
        incoming: CatalogIncoming,
    ) -> Result<(), String> {
        let old_bytes = match diff.incoming.get_key_value(name) {
            Some((key, value)) => key
                .capacity()
                .checked_add(
                    catalog_incoming_bytes(value)
                        .ok_or_else(|| "catalog diff size overflow".to_string())?,
                )
                .ok_or_else(|| "catalog diff size overflow".to_string())?,
            None => 0,
        };
        diff.incoming.insert(name.to_owned(), incoming);
        let new_bytes = match diff.incoming.get_key_value(name) {
            Some((key, value)) => key
                .capacity()
                .checked_add(
                    catalog_incoming_bytes(value)
                        .ok_or_else(|| "catalog diff size overflow".to_string())?,
                )
                .ok_or_else(|| "catalog diff size overflow".to_string())?,
            None => return Err("catalog incoming insertion disappeared".into()),
        };
        self.replace(old_bytes, new_bytes)?;
        self.sync_capacities(diff)?;
        self.ensure_bound()
    }

    fn remove_fingerprint(&mut self, diff: &mut CatalogDiff, name: &str) -> Result<(), String> {
        let bytes = match diff.current_fingerprints.get_key_value(name) {
            Some((key, value)) => key
                .capacity()
                .checked_add(value.capacity())
                .ok_or_else(|| "catalog diff size overflow".to_string())?,
            None => 0,
        };
        diff.current_fingerprints.remove(name);
        self.replace(bytes, 0)?;
        self.ensure_bound()
    }

    fn remove_incoming(&mut self, diff: &mut CatalogDiff, name: &str) -> Result<(), String> {
        let bytes = match diff.incoming.get_key_value(name) {
            Some((key, value)) => key
                .capacity()
                .checked_add(
                    catalog_incoming_bytes(value)
                        .ok_or_else(|| "catalog diff size overflow".to_string())?,
                )
                .ok_or_else(|| "catalog diff size overflow".to_string())?,
            None => 0,
        };
        diff.incoming.remove(name);
        self.replace(bytes, 0)?;
        self.ensure_bound()
    }

    fn remove_changed_names(
        &mut self,
        diff: &mut CatalogDiff,
        unchanged: &[usize],
    ) -> Result<(), String> {
        let mut removed = 0usize;
        for index in unchanged {
            if let Some(name) = diff.changed.get(*index) {
                add_catalog_bytes(&mut removed, name.capacity())
                    .map_err(|_| "catalog diff size overflow".to_string())?;
            }
        }
        self.replace(removed, 0)?;
        let mut unchanged_pos = 0;
        let mut changed_pos = 0;
        diff.changed.retain(|_| {
            let is_unchanged = unchanged.get(unchanged_pos).copied() == Some(changed_pos);
            if is_unchanged {
                unchanged_pos += 1;
            }
            changed_pos += 1;
            !is_unchanged
        });
        self.ensure_bound()
    }
}

#[cfg(feature = "load")]
#[derive(Debug, Clone, Copy)]
enum CatalogNameList {
    Added,
    Changed,
    Removed,
}

#[cfg(feature = "load")]
fn catalog_incoming_bytes(incoming: &CatalogIncoming) -> Option<usize> {
    let card_bytes = registry_card_bytes(&incoming.card)?
        .checked_sub(std::mem::size_of::<crate::rs2b0t_registry::RegistryCard>())?;
    let mut bytes = card_bytes.checked_add(incoming.path.as_os_str().len())?;
    if let Some(origin) = &incoming.origin {
        bytes = bytes.checked_add(origin.capacity())?;
    }
    if let Some(addition) = &incoming.addition {
        bytes = bytes.checked_add(addition.sha256.capacity())?;
        if let Some(unloadable) = &addition.unloadable {
            bytes = bytes.checked_add(unloadable.capacity())?;
        }
    }
    Some(bytes)
}

#[derive(Debug, Clone, Copy, Default)]
enum CatalogDiffPhase {
    #[default]
    Registry,
    RootSources,
    SiblingScan,
    SiblingSources,
    ShopDb,
    Analyze,
    FailedIndex,
    Removed,
    SortAdded,
    SortChanged,
    SortRemoved,
    Finish,
    Complete,
}

#[cfg(feature = "load")]
#[derive(Debug, Clone, Copy, Default)]
enum CatalogAnalyzePhase {
    #[default]
    Start,
    Fingerprint,
    AddedHash,
    Unloadable,
    ParseCard,
}

#[cfg(feature = "load")]
#[derive(Debug)]
struct CatalogAnalyzeState {
    registration_index: usize,
    path: PathBuf,
    shape: LoadShape,
    api_family: ApiFamily,
    is_added: bool,
    sha256: Option<String>,
    addition: Option<CatalogAddedCard>,
}

#[cfg(feature = "load")]
#[derive(Debug, Default)]
struct CatalogDiffWork {
    phase: CatalogDiffPhase,
    completed: u32,
    used: usize,
    source_limit: usize,
    registry_index: Option<crate::rs2b0t_registry::RegistryIndex>,
    registrations: Vec<crate::rs2b0t_registry::RegistryCard>,
    sources: HashMap<String, String>,
    source_files: usize,
    registration_index: usize,
    source_iter: Option<std::collections::hash_map::IntoIter<String, String>>,
    sibling_rels: HashSet<String>,
    sibling_values_bytes: usize,
    sibling_set_capacity: usize,
    sibling_iter: Option<std::collections::hash_set::IntoIter<String>>,
    shop_index: usize,
    analyze_index: usize,
    analyze_phase: CatalogAnalyzePhase,
    analysis: Option<CatalogAnalyzeState>,
    fingerprint_work: Option<PreparationFingerprintWork>,
    unloadable_work: Option<FirstUnloadableWork>,
    failed_indexes: Vec<usize>,
    failed_index_fallback: bool,
    removed_index: usize,
    diff: Option<CatalogDiff>,
    usage: Option<CatalogDiffUsage>,
}

#[cfg(feature = "load")]
impl CatalogDiffWork {
    fn step(
        &mut self,
        snapshot: &CatalogPreparationSnapshot,
    ) -> Result<Option<(CatalogDiff, CatalogDiffUsage)>, String> {
        self.completed = self.completed.saturating_add(1);
        match self.phase {
            CatalogDiffPhase::Registry => self.read_registry(snapshot)?,
            CatalogDiffPhase::RootSources => self.read_root_source(snapshot)?,
            CatalogDiffPhase::SiblingScan => self.scan_siblings()?,
            CatalogDiffPhase::SiblingSources => self.read_sibling_source(snapshot)?,
            CatalogDiffPhase::ShopDb => self.read_shop_db(snapshot)?,
            CatalogDiffPhase::Analyze => self.analyze_registration(snapshot)?,
            CatalogDiffPhase::FailedIndex => self.index_failures()?,
            CatalogDiffPhase::Removed => self.add_removed(snapshot)?,
            CatalogDiffPhase::SortAdded => {
                self.diff
                    .as_mut()
                    .expect("diff exists before sorting")
                    .added
                    .sort();
                self.phase = CatalogDiffPhase::SortChanged;
            }
            CatalogDiffPhase::SortChanged => {
                self.diff
                    .as_mut()
                    .expect("diff exists before sorting")
                    .changed
                    .sort();
                self.phase = CatalogDiffPhase::SortRemoved;
            }
            CatalogDiffPhase::SortRemoved => {
                self.diff
                    .as_mut()
                    .expect("diff exists before sorting")
                    .removed
                    .sort();
                self.phase = CatalogDiffPhase::Finish;
            }
            CatalogDiffPhase::Finish => {
                let diff = self.diff.take().expect("diff exists at finish");
                let usage = self.usage.take().expect("diff usage exists at finish");
                ensure_catalog_diff_bound(&diff)?;
                self.phase = CatalogDiffPhase::Complete;
                return Ok(Some((diff, usage)));
            }
            CatalogDiffPhase::Complete => {
                return Err("catalog diff work completed more than once".into());
            }
        }
        Ok(None)
    }

    fn read_registry(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<(), String> {
        self.source_limit = snapshot.source_limit();
        let index = crate::rs2b0t_registry::registry_index_path(&snapshot.root);
        let index_limit = self.source_limit.min(MAX_CATALOG_INDEX_BYTES);
        let index_ts = vault::read_regular_file(&index, index_limit as u64)
            .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
        self.used = index_ts.capacity();
        let registry_index = crate::rs2b0t_registry::RegistryIndex::parse(&index_ts)
            .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
        let registrations = registry_index
            .registrations()
            .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
        if registrations.len() > MAX_CATALOG_CARDS {
            return Err("catalog has more than 4096 registrations".into());
        }
        for card in &registrations {
            add_catalog_bytes(
                &mut self.used,
                registry_card_bytes(card).ok_or("catalog registry size overflow")?,
            )
            .map_err(|_| "catalog registry metadata exceeds its 56 MiB bound")?;
        }
        if self.used > self.source_limit {
            return Err("catalog inputs exceed the WorkAdmission byte bound".into());
        }
        self.registry_index = Some(registry_index);
        self.registrations = registrations;
        let diff = CatalogDiff {
            added: Vec::new(),
            changed: Vec::new(),
            removed: Vec::new(),
            failed: Vec::new(),
            current_fingerprints: HashMap::new(),
            incoming: HashMap::new(),
        };
        self.usage = Some(CatalogDiffUsage::new(&diff)?);
        self.diff = Some(diff);
        self.phase = CatalogDiffPhase::RootSources;
        Ok(())
    }

    fn read_root_source(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<(), String> {
        if self.registration_index >= self.registrations.len() {
            self.phase = CatalogDiffPhase::SiblingScan;
            self.source_iter = Some(std::mem::take(&mut self.sources).into_iter());
            return Ok(());
        }
        let registration = &self.registrations[self.registration_index];
        self.registration_index += 1;
        self.source_files = self.source_files.saturating_add(1);
        if self.source_files > MAX_CATALOG_FILES {
            return Err("catalog source graph exceeds 8192 files".into());
        }
        let Some(path) = script_file_path(&snapshot.root, &registration.rel_path) else {
            let failure = LoadFailure::capture(
                ScriptSource::Catalog,
                Path::new(&registration.rel_path),
                &registration.name,
                &format!("missing {}", registration.rel_path),
                None,
                None,
            );
            record_catalog_failure(&mut self.diff, &mut self.usage, failure)?;
            return Ok(());
        };
        match read_catalog_source(&path, &mut self.used, self.source_limit) {
            Ok(text) => add_catalog_source(
                &mut self.sources,
                &registration.rel_path,
                text,
                &mut self.used,
                self.source_limit,
            )
            .map_err(|error| error.message()),
            Err(CatalogSourceError::Aggregate) => {
                Err("catalog sources exceed the 56 MiB WorkAdmission bound".into())
            }
            Err(CatalogSourceError::File(error)) => {
                let failure = LoadFailure::capture(
                    ScriptSource::Catalog,
                    &path,
                    &registration.name,
                    &error,
                    None,
                    None,
                );
                record_catalog_failure(&mut self.diff, &mut self.usage, failure)
            }
        }
    }

    fn scan_siblings(&mut self) -> Result<(), String> {
        let next_source = self.source_iter.as_mut().and_then(|iter| iter.next());
        let Some((relative, text)) = next_source else {
            self.source_iter = None;
            self.sibling_iter = Some(std::mem::take(&mut self.sibling_rels).into_iter());
            self.phase = CatalogDiffPhase::SiblingSources;
            return Ok(());
        };
        let siblings = crate::rs2b0t_registry::same_dir_import_rels(&relative, &text);
        if siblings.len() > MAX_PREPARATION_SIBLINGS {
            return Err(format!(
                "catalog source {relative} has more than 128 sibling modules"
            ));
        }
        for sibling in siblings {
            let capacity = sibling.capacity();
            if !self.sibling_rels.insert(sibling) {
                continue;
            }
            add_catalog_bytes(&mut self.sibling_values_bytes, capacity)
                .map_err(|_| "catalog sibling index size overflow")?;
            self.sibling_set_capacity = self.sibling_rels.capacity();
            let set_bytes =
                catalog_map_storage_bytes(self.sibling_set_capacity, std::mem::size_of::<String>())
                    .ok_or("catalog sibling index size overflow")?;
            let used_scratch = self
                .usage
                .as_ref()
                .expect("diff usage exists while scanning siblings")
                .bytes
                .checked_add(self.sibling_values_bytes)
                .and_then(|bytes| bytes.checked_add(set_bytes))
                .ok_or("catalog sibling index size overflow")?;
            if used_scratch > MAX_CATALOG_SCRATCH_BYTES {
                return Err("catalog preparation scratch exceeds the 32 MiB bound".into());
            }
            if self.sibling_rels.len() > MAX_CATALOG_FILES {
                return Err("catalog source graph exceeds 8192 sibling modules".into());
            }
        }
        self.sources.insert(relative, text);
        Ok(())
    }

    fn read_sibling_source(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<(), String> {
        let Some(sibling) = self.sibling_iter.as_mut().and_then(|iter| iter.next()) else {
            self.sibling_iter = None;
            self.sibling_values_bytes = 0;
            self.sibling_set_capacity = 0;
            self.phase = CatalogDiffPhase::ShopDb;
            return Ok(());
        };
        if self.sources.contains_key(&sibling) {
            return Ok(());
        }
        self.source_files = self.source_files.saturating_add(1);
        if self.source_files > MAX_CATALOG_FILES {
            return Err("catalog source graph exceeds 8192 files".into());
        }
        let Some(path) = script_file_path(&snapshot.root, &sibling) else {
            return Ok(());
        };
        match read_catalog_source(&path, &mut self.used, self.source_limit) {
            Ok(text) => add_catalog_source(
                &mut self.sources,
                &sibling,
                text,
                &mut self.used,
                self.source_limit,
            )
            .map_err(|error| error.message()),
            Err(CatalogSourceError::Aggregate) => {
                Err("catalog sources exceed the WorkAdmission byte bound".into())
            }
            Err(CatalogSourceError::File(_)) => Ok(()),
        }
    }

    fn read_shop_db(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<(), String> {
        if self.sources.contains_key("./src/bot/data/shopdb.ts")
            || self.sources.contains_key("./src/bot/data/shopdb.js")
        {
            self.phase = CatalogDiffPhase::Analyze;
            self.analyze_index = 0;
            return Ok(());
        }
        let (relative, path) = match self.shop_index {
            0 => (
                "./src/bot/data/shopdb.ts",
                snapshot.root.join("src/bot/data/shopdb.ts"),
            ),
            1 => (
                "./src/bot/data/shopdb.js",
                snapshot.root.join("src/bot/data/shopdb.js"),
            ),
            _ => {
                self.phase = CatalogDiffPhase::Analyze;
                self.analyze_index = 0;
                return Ok(());
            }
        };
        self.shop_index += 1;
        if !path.exists() {
            return Ok(());
        }
        self.source_files = self.source_files.saturating_add(1);
        if self.source_files > MAX_CATALOG_FILES {
            return Err("catalog source graph exceeds 8192 files".into());
        }
        match read_catalog_source(&path, &mut self.used, self.source_limit) {
            Ok(text) => {
                add_catalog_source(
                    &mut self.sources,
                    relative,
                    text,
                    &mut self.used,
                    self.source_limit,
                )
                .map_err(|error| error.message())?;
                self.phase = CatalogDiffPhase::Analyze;
                self.analyze_index = 0;
                Ok(())
            }
            Err(CatalogSourceError::Aggregate) => {
                Err("catalog sources exceed the WorkAdmission byte bound".into())
            }
            Err(CatalogSourceError::File(_)) => Ok(()),
        }
    }

    fn analyze_registration(
        &mut self,
        snapshot: &CatalogPreparationSnapshot,
    ) -> Result<(), String> {
        match self.analyze_phase {
            CatalogAnalyzePhase::Start => {
                if self.analyze_index >= self.registrations.len() {
                    self.release_sources();
                    self.phase = CatalogDiffPhase::FailedIndex;
                    return Ok(());
                }
                let registration_index = self.analyze_index;
                self.analyze_index += 1;
                let registration = &self.registrations[registration_index];
                if is_reserved(&registration.name) {
                    return Ok(());
                }
                let Some(path) = script_file_path(&snapshot.root, &registration.rel_path) else {
                    let failure = LoadFailure::capture(
                        ScriptSource::Catalog,
                        Path::new(&registration.rel_path),
                        &registration.name,
                        &format!("missing {}", registration.rel_path),
                        None,
                        None,
                    );
                    return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                };
                let Some(origin) = self.sources.get(&registration.rel_path) else {
                    let failure = LoadFailure::capture(
                        ScriptSource::Catalog,
                        &path,
                        &registration.name,
                        &format!("unreadable {}", path.display()),
                        None,
                        None,
                    );
                    return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                };
                let (shape, api_family) = match resolve_api_family(origin) {
                    Err(error) => {
                        let failure = LoadFailure::capture_bounded(
                            ScriptSource::Catalog,
                            &path,
                            &registration.name,
                            &error,
                            origin,
                            None,
                            MAX_PREPARATION_SOURCE_BYTES,
                        );
                        return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                    }
                    Ok((LoadShape::Reject, _)) => return Ok(()),
                    Ok(resolved) => resolved,
                };
                let fingerprint_work = match PreparationFingerprintWork::new(
                    &path,
                    origin,
                    MAX_PREPARATION_SOURCE_BYTES,
                ) {
                    Ok(work) => work,
                    Err(error) => {
                        let failure = LoadFailure::capture_bounded(
                            ScriptSource::Catalog,
                            &path,
                            &registration.name,
                            &error,
                            origin,
                            Some(api_family),
                            MAX_PREPARATION_SOURCE_BYTES,
                        );
                        return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                    }
                };
                self.analysis = Some(CatalogAnalyzeState {
                    registration_index,
                    path,
                    shape,
                    api_family,
                    is_added: !snapshot.existing.contains_key(&registration.name),
                    sha256: None,
                    addition: None,
                });
                self.fingerprint_work = Some(fingerprint_work);
                self.analyze_phase = CatalogAnalyzePhase::Fingerprint;
            }
            CatalogAnalyzePhase::Fingerprint => {
                let analysis = self
                    .analysis
                    .as_ref()
                    .expect("registration analysis exists");
                let registration = &self.registrations[analysis.registration_index];
                let origin = self
                    .sources
                    .get(&registration.rel_path)
                    .expect("root source exists during analysis");
                let fingerprint = match self
                    .fingerprint_work
                    .as_mut()
                    .expect("fingerprint work exists")
                    .step(origin)
                {
                    Ok(Some(fingerprint)) => fingerprint,
                    Ok(None) => return Ok(()),
                    Err(error) => {
                        let failure = LoadFailure::capture_bounded(
                            ScriptSource::Catalog,
                            &analysis.path,
                            &registration.name,
                            &error,
                            origin,
                            Some(analysis.api_family),
                            MAX_PREPARATION_SOURCE_BYTES,
                        );
                        self.analysis = None;
                        self.fingerprint_work = None;
                        self.analyze_phase = CatalogAnalyzePhase::Start;
                        return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                    }
                };
                let identity_key = crate::identity::card_identity_key(
                    ScriptSource::Catalog,
                    &analysis.path,
                    &registration.name,
                );
                let is_changed = !analysis.is_added
                    && snapshot.fingerprints.get(&identity_key).map(String::as_str)
                        != Some(fingerprint.as_str());
                {
                    let (diff, usage) = catalog_diff_and_usage(&mut self.diff, &mut self.usage)?;
                    usage.set_fingerprint(diff, &registration.name, fingerprint)?;
                }
                self.fingerprint_work = None;
                if !analysis.is_added && !is_changed {
                    self.analysis = None;
                    self.analyze_phase = CatalogAnalyzePhase::Start;
                } else if analysis.is_added {
                    self.analyze_phase = CatalogAnalyzePhase::AddedHash;
                } else {
                    self.analyze_phase = CatalogAnalyzePhase::ParseCard;
                }
            }
            CatalogAnalyzePhase::AddedHash => {
                let analysis = self
                    .analysis
                    .as_mut()
                    .expect("registration analysis exists");
                let registration = &self.registrations[analysis.registration_index];
                let origin = self
                    .sources
                    .get(&registration.rel_path)
                    .expect("root source exists during analysis");
                analysis.sha256 = Some(JsCache::origin_sha(origin.as_bytes()));
                let unloadable_work = match FirstUnloadableWork::new(
                    origin,
                    &analysis.path,
                    MAX_PREPARATION_SOURCE_BYTES,
                ) {
                    Ok(work) => work,
                    Err(error) => {
                        let failure = LoadFailure::capture_bounded(
                            ScriptSource::Catalog,
                            &analysis.path,
                            &registration.name,
                            &error,
                            origin,
                            Some(analysis.api_family),
                            MAX_PREPARATION_SOURCE_BYTES,
                        );
                        self.analysis = None;
                        self.analyze_phase = CatalogAnalyzePhase::Start;
                        return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                    }
                };
                self.unloadable_work = Some(unloadable_work);
                self.analyze_phase = CatalogAnalyzePhase::Unloadable;
            }
            CatalogAnalyzePhase::Unloadable => {
                let analysis = self
                    .analysis
                    .as_ref()
                    .expect("registration analysis exists");
                let registration = &self.registrations[analysis.registration_index];
                let origin = self
                    .sources
                    .get(&registration.rel_path)
                    .expect("root source exists during analysis");
                let first_unloadable = match self
                    .unloadable_work
                    .as_mut()
                    .expect("unloadable work exists")
                    .step()
                {
                    Ok(Some(first_unloadable)) => first_unloadable,
                    Ok(None) => return Ok(()),
                    Err(error) => {
                        let failure = LoadFailure::capture_bounded(
                            ScriptSource::Catalog,
                            &analysis.path,
                            &registration.name,
                            &error,
                            origin,
                            Some(analysis.api_family),
                            MAX_PREPARATION_SOURCE_BYTES,
                        );
                        self.analysis = None;
                        self.unloadable_work = None;
                        self.analyze_phase = CatalogAnalyzePhase::Start;
                        return record_catalog_failure(&mut self.diff, &mut self.usage, failure);
                    }
                };
                let analysis = self
                    .analysis
                    .as_mut()
                    .expect("registration analysis exists");
                let registration = &self.registrations[analysis.registration_index];
                let sha256 = analysis.sha256.take().expect("added-card hash exists");
                let unloadable = catalog_unloadable(
                    &registration.name,
                    ScriptSource::Catalog,
                    &sha256,
                    &analysis.path,
                    first_unloadable,
                );
                analysis.addition = Some(CatalogAddedCard {
                    shape: analysis.shape,
                    api_family: analysis.api_family,
                    sha256,
                    unloadable,
                });
                self.unloadable_work = None;
                self.analyze_phase = CatalogAnalyzePhase::ParseCard;
            }
            CatalogAnalyzePhase::ParseCard => {
                let analysis = self.analysis.take().expect("registration analysis exists");
                let registration = &self.registrations[analysis.registration_index];
                let origin = self
                    .sources
                    .get(&registration.rel_path)
                    .expect("root source exists during analysis");
                let card = self
                    .registry_index
                    .as_ref()
                    .expect("registry index exists during analysis")
                    .card_with_sources_bounded(
                        &self.sources,
                        &registration.name,
                        MAX_PREPARATION_SOURCE_BYTES,
                    )
                    .map_err(|error| format!("catalog {}: {error}", registration.name))?;
                let incoming = CatalogIncoming {
                    card,
                    path: analysis.path,
                    origin: analysis.is_added.then(|| origin.clone()),
                    addition: analysis.addition,
                };
                let list = if analysis.is_added {
                    CatalogNameList::Added
                } else {
                    CatalogNameList::Changed
                };
                let (diff, usage) = catalog_diff_and_usage(&mut self.diff, &mut self.usage)?;
                usage.push_name(diff, list, registration.name.clone())?;
                usage.set_incoming(diff, &registration.name, incoming)?;
                self.analyze_phase = CatalogAnalyzePhase::Start;
            }
        }
        Ok(())
    }

    fn index_failures(&mut self) -> Result<(), String> {
        let diff = self.diff.as_ref().expect("diff exists before removals");
        let required = diff
            .failed
            .len()
            .checked_mul(std::mem::size_of::<usize>())
            .ok_or("catalog failure index size overflow")?;
        if self
            .usage
            .as_ref()
            .expect("diff usage exists before removals")
            .bytes
            .checked_add(required)
            .is_some_and(|bytes| bytes <= MAX_CATALOG_SCRATCH_BYTES)
        {
            self.failed_indexes = (0..diff.failed.len()).collect();
            self.failed_indexes.sort_unstable_by(|left, right| {
                diff.failed[*left].name.cmp(&diff.failed[*right].name)
            });
        } else {
            self.failed_index_fallback = true;
        }
        self.phase = CatalogDiffPhase::Removed;
        Ok(())
    }

    fn add_removed(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<(), String> {
        if self.removed_index >= snapshot.existing.len() {
            self.failed_indexes = Vec::new();
            self.failed_index_fallback = false;
            self.phase = CatalogDiffPhase::SortAdded;
            return Ok(());
        }
        let name = snapshot
            .existing
            .keys()
            .nth(self.removed_index)
            .expect("existing card index is within map length");
        self.removed_index += 1;
        let should_remove = {
            let diff = self.diff.as_ref().expect("diff exists during removals");
            let not_failed = if self.failed_index_fallback {
                !diff
                    .failed
                    .iter()
                    .any(|failure| failure.name.as_str() == name.as_str())
            } else {
                self.failed_indexes
                    .binary_search_by(|index| diff.failed[*index].name.as_str().cmp(name.as_str()))
                    .is_err()
            };
            !diff.current_fingerprints.contains_key(name) && not_failed
        };
        if should_remove {
            let name = name.clone();
            let (diff, usage) = catalog_diff_and_usage(&mut self.diff, &mut self.usage)?;
            usage.push_name(diff, CatalogNameList::Removed, name)?;
        }
        Ok(())
    }

    fn release_sources(&mut self) {
        self.registry_index = None;
        self.registrations = Vec::new();
        self.sources = HashMap::new();
        self.sibling_rels = HashSet::new();
        self.sibling_iter = None;
        self.sibling_values_bytes = 0;
        self.sibling_set_capacity = 0;
        self.used = 0;
        self.source_limit = 0;
    }
}

#[derive(Debug, Clone, Copy, Default)]
enum CatalogPreparationPhase {
    #[default]
    Diff,
    Candidates,
    Cleanup,
    Compact,
    Finish,
    Complete,
}

#[cfg(feature = "load")]
#[derive(Debug, Default)]
struct CatalogPreparationWork {
    phase: CatalogPreparationPhase,
    completed: u32,
    diff_work: CatalogDiffWork,
    candidates: Option<CatalogCandidateWork>,
}

#[cfg(feature = "load")]
impl CatalogPreparationWork {
    fn step(
        &mut self,
        snapshot: &CatalogPreparationSnapshot,
    ) -> Result<Option<CatalogPreparationResult>, String> {
        self.completed = self.completed.saturating_add(1);
        match self.phase {
            CatalogPreparationPhase::Diff => {
                if let Some((diff, usage)) = self.diff_work.step(snapshot)? {
                    self.candidates = Some(CatalogCandidateWork::new(diff, usage)?);
                    self.phase = CatalogPreparationPhase::Candidates;
                }
            }
            CatalogPreparationPhase::Candidates => {
                let candidates = self
                    .candidates
                    .as_mut()
                    .expect("candidate state exists after diff");
                if !candidates.prepare_next(snapshot)? {
                    self.phase = CatalogPreparationPhase::Cleanup;
                }
            }
            CatalogPreparationPhase::Cleanup => {
                let candidates = self
                    .candidates
                    .as_mut()
                    .expect("candidate state exists during cleanup");
                if !candidates.cleanup_next(snapshot)? {
                    self.phase = CatalogPreparationPhase::Compact;
                }
            }
            CatalogPreparationPhase::Compact => {
                let candidates = self
                    .candidates
                    .as_mut()
                    .expect("candidate state exists during compaction");
                candidates.compact_changed()?;
                self.phase = CatalogPreparationPhase::Finish;
            }
            CatalogPreparationPhase::Finish => {
                let candidates = self
                    .candidates
                    .take()
                    .expect("candidate state exists at completion");
                candidates.ensure_result_bound()?;
                self.phase = CatalogPreparationPhase::Complete;
                return Ok(Some(CatalogPreparationResult {
                    diff: candidates.diff,
                    prepared: candidates.prepared,
                    warmed: candidates.warmed,
                    prepare_failed: candidates.prepare_failed,
                }));
            }
            CatalogPreparationPhase::Complete => {
                return Err("catalog preparation work completed more than once".into());
            }
        }
        Ok(None)
    }
}

#[cfg(feature = "load")]
#[derive(Debug)]
struct CatalogCandidateWork {
    diff: CatalogDiff,
    usage: CatalogDiffUsage,
    changed_len: usize,
    candidate_count: usize,
    candidate_index: usize,
    card_task: Option<CardPreparationTask>,
    card_task_index: usize,
    card_task_is_added: bool,
    prepared: Vec<PreparedCard>,
    warmed: Vec<PreparedCard>,
    prepare_failed: Vec<LoadFailure>,
    unchanged_changed: Vec<usize>,
    cleanup_index: usize,
    result_bytes: usize,
}

#[cfg(feature = "load")]
impl CatalogCandidateWork {
    fn new(diff: CatalogDiff, usage: CatalogDiffUsage) -> Result<Self, String> {
        let changed_len = diff.changed.len();
        let added_len = diff.added.len();
        let candidate_count = changed_len
            .checked_add(added_len)
            .ok_or_else(|| "catalog candidate count overflow".to_string())?;
        let prepared = Vec::with_capacity(changed_len);
        let warmed = Vec::with_capacity(added_len);
        let prepare_failed = Vec::with_capacity(candidate_count);
        let unchanged_changed = Vec::new();
        let mut result_bytes = usage.bytes;
        add_catalog_bytes(
            &mut result_bytes,
            std::mem::size_of::<CatalogPreparationResult>()
                .checked_sub(std::mem::size_of::<CatalogDiff>())
                .ok_or_else(|| "catalog result size overflow".to_string())?,
        )
        .map_err(|_| "catalog result size overflow")?;
        for capacity in [prepared.capacity(), warmed.capacity()] {
            add_catalog_bytes(
                &mut result_bytes,
                capacity
                    .checked_mul(std::mem::size_of::<PreparedCard>())
                    .ok_or_else(|| "catalog result size overflow".to_string())?,
            )
            .map_err(|_| "catalog result size overflow")?;
        }
        add_catalog_bytes(
            &mut result_bytes,
            prepare_failed
                .capacity()
                .checked_mul(std::mem::size_of::<LoadFailure>())
                .ok_or_else(|| "catalog result size overflow".to_string())?,
        )
        .map_err(|_| "catalog result size overflow")?;
        let work = Self {
            diff,
            usage,
            changed_len,
            candidate_count,
            candidate_index: 0,
            card_task: None,
            card_task_index: 0,
            card_task_is_added: false,
            prepared,
            warmed,
            prepare_failed,
            unchanged_changed,
            cleanup_index: 0,
            result_bytes,
        };
        work.ensure_result_bound()?;
        work.ensure_scratch_bound()?;
        Ok(work)
    }

    fn prepare_next(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<bool, String> {
        if let Some(card_task) = self.card_task.as_mut() {
            let outcome = card_task.step()?;
            let Some(outcome) = outcome else {
                return Ok(true);
            };
            let index = self.card_task_index;
            let is_added = self.card_task_is_added;
            self.card_task = None;
            match outcome {
                CardPreparationOutcome::Prepared(card) => {
                    self.push_prepared(card, is_added)?;
                }
                CardPreparationOutcome::Failed { failure, .. } => {
                    self.push_failure(failure)?;
                }
                CardPreparationOutcome::Unchanged if !is_added => {
                    self.unchanged_changed.push(index);
                    self.ensure_scratch_bound()?;
                }
                CardPreparationOutcome::Unchanged | CardPreparationOutcome::Confirmed => {}
            }
            return Ok(true);
        }
        if self.candidate_index >= self.candidate_count {
            return Ok(false);
        }
        let index = self.candidate_index;
        self.candidate_index += 1;
        let is_added = index >= self.changed_len;
        let name = if is_added {
            self.diff.added[index - self.changed_len].clone()
        } else {
            self.diff.changed[index].clone()
        };
        let incoming = self
            .diff
            .incoming
            .get(&name)
            .ok_or_else(|| format!("catalog {name}: parsed registration is missing"))?;
        let path = incoming.path.clone();
        let template = if is_added {
            CardPreparationTemplate::from_registry_card(&incoming.card, path.clone())
        } else {
            snapshot
                .existing
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("catalog {name}: existing card snapshot is missing"))?
        };
        let identity_key = crate::identity::card_identity_key(ScriptSource::Catalog, &path, &name);
        let card_snapshot = CardPreparationSnapshot {
            template,
            cache_root: snapshot.cache_root.clone(),
            catalog_root: Some(snapshot.root.clone()),
            stored_fingerprint: snapshot.fingerprints.get(&identity_key).cloned(),
            confirmation_fingerprint: None,
            catalog_settings_schema: Some(incoming.card.settings_schema.clone()),
            allow_unloadable: false,
        };
        self.card_task_index = index;
        self.card_task_is_added = is_added;
        self.card_task = Some(card_snapshot.start_cooperative());
        Ok(true)
    }

    fn push_prepared(&mut self, card: PreparedCard, is_added: bool) -> Result<(), String> {
        let retained = prepared_card_retained_bytes(&card)
            .ok_or_else(|| "prepared catalog card size overflow".to_string())?;
        let (capacity, next_capacity) = if is_added {
            let capacity = self.warmed.capacity();
            if self.warmed.len() == capacity {
                self.warmed.reserve(1);
            }
            (capacity, self.warmed.capacity())
        } else {
            let capacity = self.prepared.capacity();
            if self.prepared.len() == capacity {
                self.prepared.reserve(1);
            }
            (capacity, self.prepared.capacity())
        };
        let item_size = std::mem::size_of::<PreparedCard>();
        self.replace_result_bytes(
            capacity
                .checked_mul(item_size)
                .ok_or_else(|| "catalog result size overflow".to_string())?,
            next_capacity
                .checked_mul(item_size)
                .ok_or_else(|| "catalog result size overflow".to_string())?,
        )?;
        self.add_result_bytes(retained)?;
        self.ensure_scratch_bound()?;
        if is_added {
            self.warmed.push(card);
        } else {
            self.prepared.push(card);
        }
        Ok(())
    }

    fn push_failure(&mut self, failure: LoadFailure) -> Result<(), String> {
        let retained = load_failure_retained_bytes(&failure)
            .ok_or_else(|| "catalog failure size overflow".to_string())?;
        let capacity = self.prepare_failed.capacity();
        if self.prepare_failed.len() == capacity {
            self.prepare_failed.reserve(1);
        }
        self.replace_result_bytes(
            capacity
                .checked_mul(std::mem::size_of::<LoadFailure>())
                .ok_or_else(|| "catalog result size overflow".to_string())?,
            self.prepare_failed
                .capacity()
                .checked_mul(std::mem::size_of::<LoadFailure>())
                .ok_or_else(|| "catalog result size overflow".to_string())?,
        )?;
        self.add_result_bytes(retained)?;
        self.ensure_scratch_bound()?;
        self.prepare_failed.push(failure);
        Ok(())
    }

    fn cleanup_next(&mut self, snapshot: &CatalogPreparationSnapshot) -> Result<bool, String> {
        let Some(index) = self.unchanged_changed.get(self.cleanup_index).copied() else {
            return Ok(false);
        };
        self.cleanup_index += 1;
        let name = self
            .diff
            .changed
            .get(index)
            .cloned()
            .ok_or_else(|| "catalog unchanged candidate index is invalid".to_string())?;
        let stored = self.diff.incoming.get(&name).map(|incoming| {
            let identity_key =
                crate::identity::card_identity_key(ScriptSource::Catalog, &incoming.path, &name);
            snapshot.fingerprints.get(&identity_key).cloned()
        });
        let before = self.usage.bytes;
        if let Some(stored) = stored {
            if let Some(fingerprint) = stored {
                if self.diff.current_fingerprints.contains_key(&name) {
                    self.usage
                        .set_fingerprint(&mut self.diff, &name, fingerprint)?;
                }
            } else {
                self.usage.remove_fingerprint(&mut self.diff, &name)?;
            }
            self.usage.remove_incoming(&mut self.diff, &name)?;
        }
        self.adjust_for_diff_usage(before)?;
        Ok(true)
    }

    fn compact_changed(&mut self) -> Result<(), String> {
        let before = self.usage.bytes;
        self.usage
            .remove_changed_names(&mut self.diff, &self.unchanged_changed)?;
        self.adjust_for_diff_usage(before)?;
        self.unchanged_changed = Vec::new();
        self.ensure_result_bound()?;
        self.ensure_scratch_bound()
    }

    fn adjust_for_diff_usage(&mut self, before: usize) -> Result<(), String> {
        self.replace_result_bytes(before, self.usage.bytes)
    }

    fn replace_result_bytes(&mut self, old: usize, new: usize) -> Result<(), String> {
        if new >= old {
            self.add_result_bytes(new - old)
        } else {
            self.result_bytes = self
                .result_bytes
                .checked_sub(old - new)
                .ok_or_else(|| "catalog result size accounting underflow".to_string())?;
            Ok(())
        }
    }

    fn add_result_bytes(&mut self, bytes: usize) -> Result<(), String> {
        add_catalog_bytes(&mut self.result_bytes, bytes)
            .map_err(|_| "catalog result size overflow".to_string())
    }

    fn ensure_result_bound(&self) -> Result<(), String> {
        if self.result_bytes > MAX_CATALOG_RESULT_BYTES {
            Err("prepared catalog result exceeds the 32 MiB result bound".into())
        } else {
            Ok(())
        }
    }

    fn ensure_scratch_bound(&self) -> Result<(), String> {
        let unchanged_bytes = self
            .unchanged_changed
            .capacity()
            .checked_mul(std::mem::size_of::<usize>())
            .ok_or_else(|| "catalog scratch size overflow".to_string())?;
        let scratch = self
            .result_bytes
            .checked_add(unchanged_bytes)
            .ok_or_else(|| "catalog scratch size overflow".to_string())?;
        if scratch > MAX_CATALOG_SCRATCH_BYTES {
            Err("catalog preparation scratch exceeds the 32 MiB bound".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(feature = "load")]
fn catalog_diff_and_usage<'a>(
    diff: &'a mut Option<CatalogDiff>,
    usage: &'a mut Option<CatalogDiffUsage>,
) -> Result<(&'a mut CatalogDiff, &'a mut CatalogDiffUsage), String> {
    match (diff.as_mut(), usage.as_mut()) {
        (Some(diff), Some(usage)) => Ok((diff, usage)),
        _ => Err("catalog diff state is unavailable".into()),
    }
}

#[cfg(feature = "load")]
fn record_catalog_failure(
    diff: &mut Option<CatalogDiff>,
    usage: &mut Option<CatalogDiffUsage>,
    failure: LoadFailure,
) -> Result<(), String> {
    let (Some(diff), Some(usage)) = (diff.as_mut(), usage.as_mut()) else {
        return Err("catalog diff state is unavailable".into());
    };
    usage.push_failure(diff, failure)
}

#[cfg(feature = "load")]
fn add_catalog_bytes(total: &mut usize, bytes: usize) -> Result<(), ()> {
    *total = total.checked_add(bytes).ok_or(())?;
    Ok(())
}

#[cfg(feature = "load")]
fn catalog_map_storage_bytes(capacity: usize, entry_bytes: usize) -> Option<usize> {
    capacity
        .checked_mul(entry_bytes.checked_add(1)?)?
        .checked_mul(2)
}

#[cfg(feature = "load")]
fn ensure_catalog_diff_bound(diff: &CatalogDiff) -> Result<(), String> {
    if diff
        .retained_bytes()
        .is_none_or(|bytes| bytes > MAX_CATALOG_RESULT_BYTES)
    {
        Err("catalog diff exceeds the 32 MiB result bound".into())
    } else {
        Ok(())
    }
}

#[cfg(feature = "load")]
fn registry_card_bytes(card: &crate::rs2b0t_registry::RegistryCard) -> Option<usize> {
    let mut bytes = std::mem::size_of::<crate::rs2b0t_registry::RegistryCard>();
    for value in [
        &card.name,
        &card.rel_path,
        &card.description,
        &card.category,
        &card.version,
    ] {
        add_catalog_bytes(&mut bytes, value.capacity()).ok()?;
    }
    add_catalog_bytes(
        &mut bytes,
        card.tags
            .capacity()
            .checked_mul(std::mem::size_of::<String>())?,
    )
    .ok()?;
    for tag in &card.tags {
        add_catalog_bytes(&mut bytes, tag.capacity()).ok()?;
    }
    add_schema_bytes(
        &mut bytes,
        &card.settings_schema,
        card.settings_schema.capacity(),
    )?;
    Some(bytes)
}

#[cfg(feature = "load")]
fn load_failure_retained_bytes(failure: &LoadFailure) -> Option<usize> {
    let mut bytes = 0usize;
    for retained in [
        failure.identity_key.capacity(),
        failure.name.capacity(),
        failure.path.as_os_str().len(),
        failure.diagnostic.capacity(),
        failure.fingerprint.capacity(),
    ] {
        add_catalog_bytes(&mut bytes, retained).ok()?;
    }
    if let Some(sibling) = &failure.sibling {
        add_catalog_bytes(&mut bytes, sibling.capacity()).ok()?;
    }
    Some(bytes)
}

#[cfg(feature = "load")]
fn prepared_card_retained_bytes(prepared: &PreparedCard) -> Option<usize> {
    let card = &prepared.card;
    let mut bytes = 0usize;
    for retained in [
        card.name.capacity(),
        card.path.as_os_str().len(),
        card.origin.capacity(),
        card.js.capacity(),
        card.sha256.capacity(),
        card.description.capacity(),
        card.category.capacity(),
        prepared.fingerprint.capacity(),
    ] {
        add_catalog_bytes(&mut bytes, retained).ok()?;
    }
    add_catalog_bytes(
        &mut bytes,
        card.tags
            .capacity()
            .checked_mul(std::mem::size_of::<String>())?,
    )
    .ok()?;
    for tag in &card.tags {
        add_catalog_bytes(&mut bytes, tag.capacity()).ok()?;
    }
    add_schema_bytes(
        &mut bytes,
        &card.settings_schema,
        card.settings_schema.capacity(),
    )?;
    if let Some(unloadable) = &card.unloadable {
        add_catalog_bytes(&mut bytes, unloadable.capacity()).ok()?;
    }
    add_catalog_bytes(
        &mut bytes,
        prepared
            .siblings
            .capacity()
            .checked_mul(std::mem::size_of::<(String, String)>())?,
    )
    .ok()?;
    for (relative, source) in &prepared.siblings {
        add_catalog_bytes(&mut bytes, relative.capacity()).ok()?;
        add_catalog_bytes(&mut bytes, source.capacity()).ok()?;
    }
    Some(bytes)
}

#[cfg(feature = "load")]
fn catalog_preparation_result_bytes(
    diff: &CatalogDiff,
    prepared: &[PreparedCard],
    prepared_capacity: usize,
    warmed: &[PreparedCard],
    warmed_capacity: usize,
    prepare_failed: &[LoadFailure],
    prepare_failed_capacity: usize,
) -> Option<usize> {
    let mut bytes = diff.retained_bytes()?;
    let headers = std::mem::size_of::<CatalogPreparationResult>()
        .checked_sub(std::mem::size_of::<CatalogDiff>())?;
    add_catalog_bytes(&mut bytes, headers).ok()?;
    for capacity in [prepared_capacity, warmed_capacity] {
        add_catalog_bytes(
            &mut bytes,
            capacity.checked_mul(std::mem::size_of::<PreparedCard>())?,
        )
        .ok()?;
    }
    for card in prepared.iter().chain(warmed) {
        add_catalog_bytes(&mut bytes, prepared_card_retained_bytes(card)?).ok()?;
    }
    add_catalog_bytes(
        &mut bytes,
        prepare_failed_capacity.checked_mul(std::mem::size_of::<LoadFailure>())?,
    )
    .ok()?;
    for failure in prepare_failed {
        add_catalog_bytes(&mut bytes, load_failure_retained_bytes(failure)?).ok()?;
    }
    Some(bytes)
}
#[cfg(feature = "load")]
fn read_catalog_source(
    path: &Path,
    used: &mut usize,
    limit: usize,
) -> Result<String, CatalogSourceError> {
    let metadata =
        std::fs::metadata(path).map_err(|error| CatalogSourceError::File(error.to_string()))?;
    if !metadata.is_file() {
        return Err(CatalogSourceError::File("not a regular file".into()));
    }
    let size = usize::try_from(metadata.len())
        .map_err(|_| CatalogSourceError::File("catalog source size overflow".into()))?;
    if size > MAX_SOURCE_BYTES as usize {
        return Err(CatalogSourceError::File(
            "catalog source exceeds the 8 MiB file bound".into(),
        ));
    }
    let remaining = limit.saturating_sub(*used);
    if size > remaining {
        return Err(CatalogSourceError::Aggregate);
    }
    let text = vault::read_regular_file(path, remaining as u64)
        .map_err(|error| CatalogSourceError::File(error.to_string()))?;
    add_catalog_bytes(used, text.capacity()).map_err(|_| CatalogSourceError::Aggregate)?;
    if *used > limit {
        return Err(CatalogSourceError::Aggregate);
    }
    Ok(text)
}

#[cfg(feature = "load")]
fn add_catalog_source(
    sources: &mut HashMap<String, String>,
    relative: &str,
    text: String,
    used: &mut usize,
    limit: usize,
) -> Result<(), CatalogSourceError> {
    if sources.contains_key(relative) {
        return Ok(());
    }
    let overhead = relative
        .len()
        .checked_add(std::mem::size_of::<(String, String)>())
        .and_then(|bytes| bytes.checked_add(128))
        .ok_or(CatalogSourceError::Aggregate)?;
    if overhead > limit.saturating_sub(*used) {
        return Err(CatalogSourceError::Aggregate);
    }
    *used += overhead;
    sources.insert(relative.to_string(), text);
    Ok(())
}

#[cfg(feature = "load")]
fn complete_catalog_sources_bounded(
    root: &Path,
    sources: &mut HashMap<String, String>,
    used: &mut usize,
    source_limit: usize,
    source_files: &mut usize,
) -> Result<(), String> {
    let mut sibling_rels = HashSet::new();
    for (relative, text) in sources.iter() {
        let siblings = crate::rs2b0t_registry::same_dir_import_rels(relative, text);
        if siblings.len() > MAX_PREPARATION_SIBLINGS {
            return Err(format!(
                "catalog source {relative} has more than 128 sibling modules"
            ));
        }
        for sibling in siblings {
            sibling_rels.insert(sibling);
            if sibling_rels.len() > MAX_CATALOG_FILES {
                return Err("catalog source graph exceeds 8192 sibling modules".into());
            }
        }
    }
    for sibling in sibling_rels {
        if sources.contains_key(&sibling) {
            continue;
        }
        *source_files = source_files.saturating_add(1);
        if *source_files > MAX_CATALOG_FILES {
            return Err("catalog source graph exceeds 8192 files".into());
        }
        let Some(path) = script_file_path(root, &sibling) else {
            continue;
        };
        match read_catalog_source(&path, used, source_limit) {
            Ok(text) => add_catalog_source(sources, &sibling, text, used, source_limit)
                .map_err(|error| error.message())?,
            Err(CatalogSourceError::Aggregate) => {
                return Err("catalog sources exceed the WorkAdmission byte bound".into());
            }
            Err(CatalogSourceError::File(_)) => {}
        }
    }
    if !sources.contains_key("./src/bot/data/shopdb.ts")
        && !sources.contains_key("./src/bot/data/shopdb.js")
    {
        for (relative, path) in [
            (
                "./src/bot/data/shopdb.ts",
                root.join("src/bot/data/shopdb.ts"),
            ),
            (
                "./src/bot/data/shopdb.js",
                root.join("src/bot/data/shopdb.js"),
            ),
        ] {
            if !path.exists() {
                continue;
            }
            *source_files = source_files.saturating_add(1);
            if *source_files > MAX_CATALOG_FILES {
                return Err("catalog source graph exceeds 8192 files".into());
            }
            match read_catalog_source(&path, used, source_limit) {
                Ok(text) => {
                    add_catalog_source(sources, relative, text, used, source_limit)
                        .map_err(|error| error.message())?;
                    break;
                }
                Err(CatalogSourceError::Aggregate) => {
                    return Err("catalog sources exceed the WorkAdmission byte bound".into());
                }
                Err(CatalogSourceError::File(_)) => continue,
            }
        }
    }
    Ok(())
}
#[cfg(feature = "load")]
pub(super) fn resolve_catalog_settings_schema_bounded(
    catalog_root: Option<&Path>,
    name: &str,
    card_path: &Path,
    candidate_origin: &str,
    source_limit: usize,
) -> Result<Vec<crate::rs2b0t_registry::SettingDef>, String> {
    let root =
        catalog_root.ok_or_else(|| format!("catalog {name}: registry root is unavailable"))?;
    let index = crate::rs2b0t_registry::registry_index_path(root);
    let index_limit = source_limit.min(MAX_CATALOG_INDEX_BYTES);
    let index_ts = vault::read_regular_file(&index, index_limit as u64)
        .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
    let registry_index = crate::rs2b0t_registry::RegistryIndex::parse(&index_ts)
        .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
    let registrations = registry_index
        .registrations()
        .map_err(|error| format!("$RS2B0T registry {}: {error}", index.display()))?;
    if registrations.len() > MAX_CATALOG_CARDS {
        return Err("catalog has more than 4096 registrations".into());
    }
    let registered = registrations
        .iter()
        .find(|card| card.name == name)
        .ok_or_else(|| format!("catalog {name}: registry entry no longer exists"))?;
    let registered_path = script_file_path(root, &registered.rel_path)
        .ok_or_else(|| format!("catalog {name}: registry source path is invalid"))?;
    if registered_path != card_path {
        return Err(format!(
            "catalog {name}: registry source path changed; refresh the catalog before preparing"
        ));
    }

    let mut used = index_ts.capacity();
    add_catalog_bytes(&mut used, candidate_origin.len())
        .map_err(|_| "catalog preparation source bound overflow")?;
    for card in &registrations {
        add_catalog_bytes(
            &mut used,
            registry_card_bytes(card).ok_or("catalog registry metadata size overflow")?,
        )
        .map_err(|_| "catalog registry metadata size overflow")?;
    }
    if used > source_limit {
        return Err("catalog inputs exceed the 8 MiB preparation bound".into());
    }
    let mut sources = HashMap::new();
    let mut source_files = registrations.len();
    for card in &registrations {
        if sources.contains_key(&card.rel_path) {
            continue;
        }
        if card.rel_path == registered.rel_path {
            let text = candidate_origin.to_string();
            add_catalog_bytes(&mut used, text.capacity())
                .map_err(|_| "catalog inputs exceed the 8 MiB preparation bound")?;
            add_catalog_source(&mut sources, &card.rel_path, text, &mut used, source_limit)
                .map_err(|error| error.message())?;
            continue;
        }
        let Some(path) = script_file_path(root, &card.rel_path) else {
            continue;
        };
        match read_catalog_source(&path, &mut used, source_limit) {
            Ok(text) => {
                add_catalog_source(&mut sources, &card.rel_path, text, &mut used, source_limit)
                    .map_err(|error| error.message())?
            }
            Err(CatalogSourceError::Aggregate) => {
                return Err("catalog inputs exceed the 8 MiB preparation bound".into());
            }
            Err(CatalogSourceError::File(_)) => {}
        }
    }
    complete_catalog_sources_bounded(
        root,
        &mut sources,
        &mut used,
        source_limit,
        &mut source_files,
    )?;
    drop(registrations);
    registry_index
        .card_with_sources_bounded(&sources, name, source_limit)
        .map(|card| card.settings_schema)
        .map_err(|error| format!("catalog {name}: {error}"))
}

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

    fn registry_sources(
        root: &Path,
        cards: &[crate::rs2b0t_registry::RegistryCard],
    ) -> HashMap<String, String> {
        let mut sources = HashMap::new();
        for card in cards {
            let Some(path) = script_file_path(root, &card.rel_path) else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(path) {
                sources.insert(card.rel_path.clone(), text);
            }
        }
        sources
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
        let mut sources = Self::registry_sources(root, &registry_cards);
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
        self.catalog_root = Some(root.to_path_buf());
        let _ = persist_rs2b0t_root_at(root, path_file);
        Ok(n)
    }

    pub(crate) fn resolve_catalog_settings_schema(
        &self,
        name: &str,
        card_path: &Path,
        candidate_origin: &str,
    ) -> Result<Vec<crate::rs2b0t_registry::SettingDef>, String> {
        resolve_catalog_settings_schema_bounded(
            self.catalog_root.as_deref(),
            name,
            card_path,
            candidate_origin,
            MAX_PREPARATION_SOURCE_BYTES,
        )
    }
    /// Does not rewrite the persisted catalog path.
    pub fn diff_catalog(&self, root: &Path) -> Result<CatalogDiff, String> {
        self.catalog_preparation_snapshot(root)?.diff()
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
            let Some(incoming) = diff.incoming.get(name) else {
                continue;
            };
            let (Some(origin), Some(addition), Some(fingerprint)) = (
                incoming.origin.as_ref(),
                incoming.addition.as_ref(),
                diff.current_fingerprints.get(name),
            ) else {
                continue;
            };
            let card = &incoming.card;
            let path = &incoming.path;
            let key = crate::identity::card_identity_key(ScriptSource::Catalog, path, name);
            self.cards.retain(|candidate| {
                !(candidate.source == ScriptSource::Catalog && candidate.name == card.name)
            });
            let js_card = JsCard {
                name: card.name.clone(),
                path: path.clone(),
                shape: addition.shape,
                origin: origin.clone(),
                js: String::new(),
                kind: card.kind,
                source: ScriptSource::Catalog,
                sha256: addition.sha256.clone(),
                description: card.description.clone(),
                category: card.category.clone(),
                tags: card.tags.clone(),
                settings_schema: card.settings_schema.clone(),
                unloadable: addition.unloadable.clone(),
                api_family: addition.api_family,
            };
            self.fingerprints.insert(key.clone(), fingerprint.clone());
            if let Some(specifier) = &js_card.unloadable {
                self.record_failure(LoadFailure::capture_with_fingerprint(
                    ScriptSource::Catalog,
                    &js_card.path,
                    &js_card.name,
                    &format!("unloadable import: {specifier}"),
                    fingerprint.clone(),
                    Some(js_card.api_family),
                ));
            } else {
                self.clear_failure(&key);
            }
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
#[derive(Debug)]
pub struct CatalogDiff {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<LoadFailure>,
    #[cfg(feature = "load")]
    current_fingerprints: HashMap<String, String>,
    #[cfg(feature = "load")]
    incoming: HashMap<String, CatalogIncoming>,
}

impl CatalogDiff {
    pub fn is_noop(&self) -> bool {
        self.added.is_empty()
            && self.changed.is_empty()
            && self.removed.is_empty()
            && self.failed.is_empty()
    }

    #[cfg(feature = "load")]
    pub fn current_fingerprint(&self, name: &str) -> Option<&str> {
        self.current_fingerprints.get(name).map(String::as_str)
    }

    #[cfg(feature = "load")]
    fn retained_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>();
        add_catalog_bytes(
            &mut bytes,
            self.added
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )
        .ok()?;
        add_catalog_bytes(
            &mut bytes,
            self.changed
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )
        .ok()?;
        add_catalog_bytes(
            &mut bytes,
            self.removed
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )
        .ok()?;
        for name in self.added.iter().chain(&self.changed).chain(&self.removed) {
            add_catalog_bytes(&mut bytes, name.capacity()).ok()?;
        }
        add_catalog_bytes(
            &mut bytes,
            self.failed
                .capacity()
                .checked_mul(std::mem::size_of::<LoadFailure>())?,
        )
        .ok()?;
        for failure in &self.failed {
            add_catalog_bytes(&mut bytes, load_failure_retained_bytes(failure)?).ok()?;
        }
        add_catalog_bytes(
            &mut bytes,
            catalog_map_storage_bytes(
                self.current_fingerprints.capacity(),
                std::mem::size_of::<(String, String)>(),
            )?,
        )
        .ok()?;
        for (name, fingerprint) in &self.current_fingerprints {
            add_catalog_bytes(&mut bytes, name.capacity()).ok()?;
            add_catalog_bytes(&mut bytes, fingerprint.capacity()).ok()?;
        }
        add_catalog_bytes(
            &mut bytes,
            catalog_map_storage_bytes(
                self.incoming.capacity(),
                std::mem::size_of::<(String, CatalogIncoming)>(),
            )?,
        )
        .ok()?;
        for (name, incoming) in &self.incoming {
            add_catalog_bytes(&mut bytes, name.capacity()).ok()?;
            let card_bytes = registry_card_bytes(&incoming.card)?
                .checked_sub(std::mem::size_of::<crate::rs2b0t_registry::RegistryCard>())?;
            add_catalog_bytes(&mut bytes, card_bytes).ok()?;
            add_catalog_bytes(&mut bytes, incoming.path.as_os_str().len()).ok()?;
            if let Some(origin) = &incoming.origin {
                add_catalog_bytes(&mut bytes, origin.capacity()).ok()?;
            }
            if let Some(addition) = &incoming.addition {
                add_catalog_bytes(&mut bytes, addition.sha256.capacity()).ok()?;
                if let Some(unloadable) = &addition.unloadable {
                    add_catalog_bytes(&mut bytes, unloadable.capacity()).ok()?;
                }
            }
        }
        Some(bytes)
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
