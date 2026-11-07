//! Checked family assets. A selected family file (`<revision>/<name>.json`) is admitted only when the manifest the
//! pin was bound to names it with the same size, digest, extractor schema and revision. Families run this once
//! while they are prepared off-pump, never on a tick.

use crate::selected::{ClientRevision, FactError, FactKey, SelectedPin};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Manifest {
    schema_version: u16,
    revisions: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    revision: i32,
    #[serde(default)]
    families: HashMap<String, Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    path: String,
    schema: u16,
    bytes: u64,
    sha256: String,
}

/// Descriptor values returned only after manifest and byte verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerifiedFamily {
    pub schema: u16,
    pub sha256: [u8; 32],
}

/// Source-only identity used during artifact production, before navigation
/// exists. This cannot authorize a runtime operation or become a SelectedPin.
pub(crate) struct FamilySource<'a> {
    pub revision: ClientRevision,
    pub engine_commit: &'a str,
    pub content_commit: &'a str,
    pub cache_id: &'a str,
    pub content_id: &'a str,
    pub schema: u16,
    pub manifest_sha256: [u8; 32],
}

impl<'a> From<&'a SelectedPin> for FamilySource<'a> {
    fn from(pin: &'a SelectedPin) -> Self {
        Self {
            revision: pin.revision,
            engine_commit: &pin.engine_commit,
            content_commit: &pin.content_commit,
            cache_id: &pin.cache_id,
            content_id: &pin.content_id,
            schema: pin.schema,
            manifest_sha256: pin.manifest_sha256,
        }
    }
}

/// Refuse `family` bytes unless the manifest the pin was bound to describes exactly them.
///
/// `PinMismatch`: the manifest is not the pinned one, or the bytes differ from the described size/digest/path.
/// `Schema`: the manifest describes a different extractor schema than the decoder reads.
/// `FamilyUnavailable`: the manifest row carries no such family.
pub(crate) fn verify_family(
    pin: &SelectedPin,
    manifest_bytes: &[u8],
    family_key: &LazyLock<FactKey>,
    schema: u16,
    family: &[u8],
) -> Result<VerifiedFamily, FactError> {
    verify_family_source(
        &FamilySource::from(pin),
        manifest_bytes,
        family_key,
        schema,
        family,
    )
}

/// The same descriptor checks for an authenticated, source-only bake identity.
pub(crate) fn verify_family_source(
    source: &FamilySource<'_>,
    manifest_bytes: &[u8],
    family_key: &LazyLock<FactKey>,
    schema: u16,
    family: &[u8],
) -> Result<VerifiedFamily, FactError> {
    let name: &str = &family_key.0;
    if <[u8; 32]>::from(Sha256::digest(manifest_bytes)) != source.manifest_sha256 {
        return Err(FactError::PinMismatch);
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| FactError::PinMismatch)?;
    if manifest.schema_version != source.schema {
        return Err(FactError::Schema {
            expected: source.schema,
            actual: manifest.schema_version,
        });
    }
    let revision = source.revision.as_i32();
    let mut rows = manifest
        .revisions
        .iter()
        .filter(|row| row.revision == revision);
    let row = rows.next().ok_or(FactError::PinMismatch)?;
    if rows.next().is_some() {
        return Err(FactError::PinMismatch);
    }
    let descriptor = row
        .families
        .get(name)
        .ok_or_else(|| FactError::FamilyUnavailable(FactKey::clone(family_key)))?;
    if descriptor.schema != schema {
        return Err(FactError::Schema {
            expected: schema,
            actual: descriptor.schema,
        });
    }
    let digest = Sha256::digest(family);
    let digest_hex = format!("{digest:x}");
    let sha256 = digest.into();
    if descriptor.path != format!("{revision}/{name}.json")
        || descriptor.bytes != family.len() as u64
        || descriptor.sha256 != digest_hex
    {
        return Err(FactError::PinMismatch);
    }
    Ok(VerifiedFamily {
        schema: descriptor.schema,
        sha256,
    })
}
