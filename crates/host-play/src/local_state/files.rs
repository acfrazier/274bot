//! The current files behind the state service (T5.2): the operator's
//! `loadouts.json` and one `bank-hints/<profile>/<account>.json` per
//! account. Each backend reads the file exactly as its old private reader
//! did and writes exactly the bytes its old private writer produced, so a
//! file written before this service is read by it, and a file it writes is
//! read by an older build.
//!
//! No backend touches `HOME` or any default path: every path is derived
//! from the explicit [`StateRoot`] and the durable identity.

use std::io;
use std::path::{Path, PathBuf};

use api::bank_memory::{HintRowsError, MAX_HINT_ROWS};
use api::identity::DurableTimestamp;
use api::loadout::{parse_loadout, Loadout};
use api::local_state::{DomainIdentity, LeaseSnapshot, LoadoutEdit, LoadoutOp, LocalStateDomain};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vault::{read_private_file, write_private_file};

/// The explicit state root, `<home>/.274bot`, resolved once by the
/// composition root and handed to the service. Nothing below it is
/// discovered: the service derives every path from this root and a
/// durable identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRoot(PathBuf);

impl StateRoot {
    /// The root at `dir`.
    pub fn new(dir: PathBuf) -> Self {
        Self(dir)
    }

    /// The root directory.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `<root>/loadouts.json`: the one operator loadouts file.
    pub fn loadouts_file(&self) -> PathBuf {
        self.0.join("loadouts.json")
    }

    /// `<root>/bank-hints/<profile>/<account>.json`: the account's hint
    /// file, spelled by the validated identity (the profile is a safe
    /// component, the account the client's login identity).
    pub fn bank_hint_file(&self, profile: &str, account: &str) -> PathBuf {
        self.0
            .join("bank-hints")
            .join(profile)
            .join(format!("{account}.json"))
    }
}

/// The on-disk hint format this build writes and the only one it reads.
pub const HINT_SCHEMA_VERSION: u32 = 1;

/// The largest hint file read: the row cap at the widest JSON a row takes
/// plus the header, with room to spare.
const MAX_HINT_BYTES: u64 = 64 * 1024;

/// The largest loadouts file read.
const MAX_LOADOUTS_BYTES: u64 = 4 * 1024 * 1024;

/// Which current file a store is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum StoreKey {
    /// The one operator loadouts file: every `loadouts-references`
    /// identity maps to it (the file is the operator's, not an account's).
    Loadouts,
    /// One account's hint file.
    BankHint { profile: String, account: String },
}

impl StoreKey {
    /// The store `identity` reads and writes, or `None` for a domain that
    /// has no current file (preferences, knowledge).
    pub(super) fn of(identity: &DomainIdentity) -> Option<Self> {
        match identity.domain() {
            LocalStateDomain::LoadoutsReferences => Some(StoreKey::Loadouts),
            LocalStateDomain::BankHints => Some(StoreKey::BankHint {
                profile: identity.account().profile().to_string(),
                account: identity.account().account().to_string(),
            }),
            LocalStateDomain::Preferences | LocalStateDomain::Knowledge => None,
        }
    }

    pub(super) fn domain(&self) -> LocalStateDomain {
        match self {
            StoreKey::Loadouts => LocalStateDomain::LoadoutsReferences,
            StoreKey::BankHint { .. } => LocalStateDomain::BankHints,
        }
    }

    pub(super) fn path(&self, root: &StateRoot) -> PathBuf {
        match self {
            StoreKey::Loadouts => root.loadouts_file(),
            StoreKey::BankHint { profile, account } => root.bank_hint_file(profile, account),
        }
    }
}

/// The digest of a file's bytes: what a readback compares against the
/// cache to see an external edit, and what an idempotent write compares
/// against to avoid rewriting the same bytes.
pub(super) type ContentDigest = [u8; 32];

pub(super) fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).into()
}

/// What a read found.
#[derive(Debug)]
pub(super) struct ReadDocument {
    /// The rows, under the domain's row contract.
    pub(super) snapshot: LeaseSnapshot,
    /// When the rows were observed: the hint's own stamp, or `read_at`
    /// for a loadouts file (which carries none).
    pub(super) at: DurableTimestamp,
    /// The digest of the bytes read.
    pub(super) digest: ContentDigest,
}

/// Why a current file was refused. The store then reads as empty (the old
/// readers' behaviour: an unreadable loadouts file was an empty store, a
/// rejected hint file loaded nothing), and the refusal is logged once.
#[derive(Debug, thiserror::Error)]
pub(super) enum ReadError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema_version {found} (this build reads {HINT_SCHEMA_VERSION})")]
    Schema { found: u32 },
    #[error("file names profile {profile:?} account {account:?}")]
    Identity { profile: String, account: String },
    #[error("rows: {0}")]
    Rows(#[from] HintRowsError),
}

/// The persisted hint document, schema 1:
///
/// ```json
/// {"schema_version":1,"profile":"local-289","account":"alice",
///  "observed_at_unix":1759700000,"rows":[[379,12],[995,3400]]}
/// ```
#[derive(Debug, Serialize, Deserialize)]
struct HintDocument {
    schema_version: u32,
    profile: String,
    account: String,
    observed_at_unix: u64,
    rows: Vec<(i32, i32)>,
}

/// Read `key`'s current file under `root`: `Ok(None)` when there is no
/// file yet, `Ok(Some)` with the checked rows, `Err` for a refused file.
pub(super) fn read(
    key: &StoreKey,
    root: &StateRoot,
    read_at: DurableTimestamp,
) -> Result<Option<ReadDocument>, ReadError> {
    let path = key.path(root);
    let max = match key {
        StoreKey::Loadouts => MAX_LOADOUTS_BYTES,
        StoreKey::BankHint { .. } => MAX_HINT_BYTES,
    };
    let raw = match read_private_file(&path, max) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let digest = digest(raw.as_bytes());
    match key {
        StoreKey::Loadouts => {
            let payload: serde_json::Value = serde_json::from_str(&raw)?;
            // Rows the old reader skipped (malformed entries) are skipped
            // here too: a file is never refused whole for one bad row.
            let rows: Vec<Loadout> = payload
                .as_array()
                .map(|rows| rows.iter().filter_map(parse_loadout).collect())
                .unwrap_or_default();
            Ok(Some(ReadDocument {
                snapshot: LeaseSnapshot::loadouts(rows),
                at: read_at,
                digest,
            }))
        }
        StoreKey::BankHint { profile, account } => {
            let document: HintDocument = serde_json::from_str(&raw)?;
            if document.schema_version != HINT_SCHEMA_VERSION {
                return Err(ReadError::Schema {
                    found: document.schema_version,
                });
            }
            if document.profile != *profile || document.account != *account {
                return Err(ReadError::Identity {
                    profile: document.profile,
                    account: document.account,
                });
            }
            if document.rows.len() > MAX_HINT_ROWS {
                return Err(HintRowsError::TooManyRows.into());
            }
            Ok(Some(ReadDocument {
                snapshot: LeaseSnapshot::bank_hints(document.rows)?,
                at: DurableTimestamp::from_unix_seconds(document.observed_at_unix),
                digest,
            }))
        }
    }
}

/// The bytes a hint write publishes: the schema-1 document, exactly as
/// the old writer serialized it.
pub(super) fn hint_bytes(
    key: &StoreKey,
    rows: &[(i32, i32)],
    observed_at: DurableTimestamp,
) -> Result<Vec<u8>, serde_json::Error> {
    let StoreKey::BankHint { profile, account } = key else {
        unreachable!("a bank hint writes a bank-hint store");
    };
    serde_json::to_vec(&HintDocument {
        schema_version: HINT_SCHEMA_VERSION,
        profile: profile.clone(),
        account: account.clone(),
        observed_at_unix: observed_at.unix_seconds(),
        rows: rows.to_vec(),
    })
}

/// The bytes a loadouts write publishes: the pretty-printed array, exactly
/// as the old store serialized it.
pub(super) fn loadouts_bytes(rows: &[Loadout]) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_string_pretty(rows).map(String::into_bytes)
}

/// Apply `edit` to `rows`: an upsert replaces the row of the same name or
/// appends; a removal drops the row of that name. A removal that names no
/// row leaves the rows as they are, and the write settles as the
/// idempotent no-op it is.
pub(super) fn apply_loadout_edit(rows: &[Loadout], edit: &LoadoutEdit) -> Vec<Loadout> {
    let mut next = rows.to_vec();
    match edit.op() {
        LoadoutOp::Upsert(loadout) => {
            if let Some(existing) = next.iter_mut().find(|row| row.name == loadout.name) {
                *existing = loadout.clone();
            } else {
                next.push(loadout.clone());
            }
        }
        LoadoutOp::Remove { name } => next.retain(|row| row.name != *name),
    }
    next
}

/// Publish `bytes` at `path` (`0o600`, atomic replace, parents `0o700`).
pub(super) fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_private_file(path, bytes)
}
