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
            add_catalog_bytes(&mut used, registry_card_bytes(card)?)
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
                Ok(text) => add_catalog_source(
                    &mut sources,
                    &card.rel_path,
                    text,
                    &mut used,
                    source_limit,
                )
                .map_err(|error| error.message())?,
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
        self.complete_catalog_sources(
            &mut sources,
            &mut used,
            source_limit,
            &mut source_files,
        )?;
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
            let identity_key = crate::identity::card_identity_key(
                ScriptSource::Catalog,
                &incoming.path,
                name,
            );
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
                        return Err("prepared catalog result exceeds the 32 MiB result bound".into());
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
                        return Err("prepared catalog result exceeds the 32 MiB result bound".into());
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
                        return Err("prepared catalog result exceeds the 32 MiB result bound".into());
                    }
                    prepare_failed.push(failure);
                }
            }
        }
        for index in unchanged_changed.iter().copied() {
            let name = &diff.changed[index];
            if let Some(incoming) = diff.incoming.get(name) {
                let identity_key = crate::identity::card_identity_key(
                    ScriptSource::Catalog,
                    &incoming.path,
                    name,
                );
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

    fn complete_catalog_sources(
        &self,
        sources: &mut HashMap<String, String>,
        used: &mut usize,
        source_limit: usize,
        source_files: &mut usize,
    ) -> Result<(), String> {
        complete_catalog_sources_bounded(
            &self.root,
            sources,
            used,
            source_limit,
            source_files,
        )
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
    add_schema_bytes(&mut bytes, &card.settings_schema)?;
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
    add_schema_bytes(&mut bytes, &card.settings_schema)?;
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
    let metadata = std::fs::metadata(path).map_err(|error| CatalogSourceError::File(error.to_string()))?;
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
    let root = catalog_root
        .ok_or_else(|| format!("catalog {name}: registry root is unavailable"))?;
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
            add_catalog_source(
                &mut sources,
                &card.rel_path,
                text,
                &mut used,
                source_limit,
            )
            .map_err(|error| error.message())?;
            continue;
        }
        let Some(path) = script_file_path(root, &card.rel_path) else {
            continue;
        };
        match read_catalog_source(&path, &mut used, source_limit) {
            Ok(text) => add_catalog_source(
                &mut sources,
                &card.rel_path,
                text,
                &mut used,
                source_limit,
            )
            .map_err(|error| error.message())?,
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
            self.cards
                .retain(|candidate| !(candidate.source == ScriptSource::Catalog && candidate.name == card.name));
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
