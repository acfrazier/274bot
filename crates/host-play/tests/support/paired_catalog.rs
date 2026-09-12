//! Test-only catalog identity helpers. Pair observations and witnesses live in
//! `host_play::paired_core` so headed and headless consumers share one proof.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

pub use host_play::paired_core::*;

pub const SUPPORT_MATRIX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/catalog-support-matrix.json"
));

#[derive(Debug, serde::Deserialize)]
struct SupportMatrix {
    catalogs: Vec<CatalogLedger>,
    rows: Vec<CardLedger>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogLedger {
    pub commit: String,
    pub identity: String,
    pub read_only_path: String,
    pub registry_path: String,
    pub registry_sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CardLedger {
    pub display_name: String,
    pub catalog_commit: String,
    pub source_path: String,
    pub source_sha256: String,
    pub revision: u16,
}

pub struct PreparedCard {
    pub js: String,
    pub shape: script::LoadShape,
    pub siblings: Vec<(String, String)>,
    pub schema: Vec<script::SettingDef>,
    pub identity: Value,
}

fn support_matrix() -> Result<SupportMatrix, String> {
    serde_json::from_str(SUPPORT_MATRIX).map_err(|error| format!("support matrix: {error}"))
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "PAIRED_CATALOG_COMMIT must be the exact lowercase 40-hex frozen commit, got {commit:?}"
        ));
    }
    if commit != CATALOG_COMMIT_A && commit != CATALOG_COMMIT_B {
        return Err(format!(
            "PAIRED_CATALOG_COMMIT {commit} is not a frozen catalog"
        ));
    }
    Ok(())
}

pub fn catalog_ledger(commit: &str) -> Result<CatalogLedger, String> {
    let matrix = support_matrix()?;
    validate_commit(commit)?;
    let rows = matrix
        .catalogs
        .iter()
        .filter(|catalog| catalog.commit == commit)
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [catalog] => Ok(catalog.clone()),
        [] => Err(format!(
            "PAIRED_CATALOG_COMMIT {commit} is not frozen in support-matrix.json"
        )),
        _ => Err(format!(
            "PAIRED_CATALOG_COMMIT {commit} is duplicated in support-matrix.json"
        )),
    }
}

pub fn card_row(commit: &str, revision: u16, display_name: &str) -> Result<CardLedger, String> {
    let matrix = support_matrix()?;
    validate_commit(commit)?;
    let rows = matrix
        .rows
        .iter()
        .filter(|row| {
            row.catalog_commit == commit
                && row.revision == revision
                && row.display_name == display_name
        })
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [row] => Ok(row.clone()),
        [] => Err(format!(
            "support matrix has no {display_name} row for catalog {commit} revision {revision}"
        )),
        _ => Err(format!(
            "support matrix has duplicate {display_name} rows for catalog {commit} revision {revision}"
        )),
    }
}

fn safe_catalog_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe support-matrix catalog path {relative:?}"));
    }
    Ok(root.join(relative))
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(script::JsCache::origin_sha(&bytes))
}

pub fn verify_source_identity(root: &Path, row: &CardLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &row.source_path)?;
    let actual = hash_file(&path)?;
    if actual != row.source_sha256 {
        return Err(format!(
            "source SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            row.source_sha256,
            actual
        ));
    }
    Ok(path)
}

pub fn verify_registry_identity(root: &Path, catalog: &CatalogLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &catalog.registry_path)?;
    let actual = hash_file(&path)?;
    if actual != catalog.registry_sha256 {
        return Err(format!(
            "registry SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            catalog.registry_sha256,
            actual
        ));
    }
    Ok(path)
}

pub fn prepare_card(
    root: &Path,
    temp: &Path,
    row: &CardLedger,
    display_name: &str,
) -> Result<PreparedCard, String> {
    let source_path = verify_source_identity(root, row)?;
    let mut library =
        script::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    let registered = library.register_rs2b0t(root, &temp.join("rs2b0t-path"))?;
    if registered == 0 {
        return Err("frozen catalog registered no script cards".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, display_name)?;
    let card = library
        .get(script::ScriptSource::Catalog, display_name)
        .cloned()
        .ok_or_else(|| format!("catalog registry has no {display_name} card"))?;
    let canonical_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", source_path.display()))?;
    let canonical_card = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", card.path.display()))?;
    if canonical_card != canonical_source {
        return Err(format!(
            "registry path mismatch for {display_name}: ledger {}, loader {}",
            source_path.display(),
            card.path.display()
        ));
    }
    if card.sha256 != row.source_sha256 {
        return Err(format!(
            "loader source hash mismatch for {display_name}: ledger {}, loader {}",
            row.source_sha256, card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{display_name} is unloadable: {import}"));
    }
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
        },
    )?;
    let sibling_hashes = siblings
        .iter()
        .map(|(url, js)| {
            json!({
                "module": url,
                "compiled_sha256": script::JsCache::origin_sha(js.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    Ok(PreparedCard {
        identity: json!({
            "card": display_name,
            "source_path": card.path,
            "source_sha256": card.sha256,
            "compiled_sha256": script::JsCache::origin_sha(card.js.as_bytes()),
            "siblings": sibling_hashes,
        }),
        js: card.js,
        shape: card.shape,
        siblings,
        schema: card.settings_schema,
    })
}

pub fn frozen_card_hashes_match(case: PairCase) -> Result<(), String> {
    for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
        for revision in [274_u16, 289] {
            let row = card_row(commit, revision, case.card_name())?;
            if row.source_sha256 != case.source_sha256() {
                return Err(format!(
                    "{} hash {} on {commit} r{revision} is not the frozen {}",
                    case.card_name(),
                    row.source_sha256,
                    case.source_sha256()
                ));
            }
        }
    }
    Ok(())
}
