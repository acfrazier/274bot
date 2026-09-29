//! Checked family assets. A selected family file (`<revision>/<name>.json`) is admitted only when the manifest the
//! pin was bound to names it with the same size, digest, extractor schema and revision. Families run this once
//! while they are prepared off-pump, never on a tick.

use crate::selected::{FactError, FactKey, SelectedPin};
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
) -> Result<(), FactError> {
    let name: &str = &family_key.0;
    if <[u8; 32]>::from(Sha256::digest(manifest_bytes)) != pin.manifest_sha256 {
        return Err(FactError::PinMismatch);
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| FactError::PinMismatch)?;
    if manifest.schema_version != pin.schema {
        return Err(FactError::Schema {
            expected: pin.schema,
            actual: manifest.schema_version,
        });
    }
    let revision = pin.revision.as_i32();
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
    if descriptor.path != format!("{revision}/{name}.json")
        || descriptor.bytes != family.len() as u64
        || descriptor.sha256 != format!("{:x}", Sha256::digest(family))
    {
        return Err(FactError::PinMismatch);
    }
    Ok(())
}
