//! The current files behind the state service (T5.2): the operator's
//! `loadouts.json` and one `bank-hints/<profile>/<account>.json` per
//! account. Each backend reads the file its old private reader read and
//! writes the shape its old private writer wrote, so a file written before
//! this service is read by it, and a file it writes is read by an older
//! build.
//!
//! No backend touches `HOME` or any default path: every path is derived
//! from the explicit [`StateRoot`] and the durable identity.
//!
//! What a read refuses is never treated as an absent file: a refused file is
//! a [`ReadError`], and the service keeps it refused (no lease, no write)
//! until a later read accepts it. What a write publishes is durable before
//! it is acknowledged: the bytes are flushed in their staging file, renamed
//! over the target, and then the directory entries are flushed
//! ([`publish`]).

use std::io;
use std::path::{Path, PathBuf};

use api::bank_memory::{HintRowsError, MAX_HINT_ROWS};
use api::identity::DurableTimestamp;
use api::loadout::{parse_loadout, Loadout};
use api::local_state::{
    DomainIdentity, FailureReason, LeaseSnapshot, LoadoutEdit, LoadoutOp, LocalStateDomain,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vault::read_private_file;

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
/// Since T5.2 r2 a written document also carries the content scope it was
/// observed under (`revision`, `content`); readers before r2 ignore the two
/// fields, and a document without them (written before r2) is read under
/// the scope that asks for it.
pub const HINT_SCHEMA_VERSION: u32 = 1;

/// The largest hint file read: the row cap at the widest JSON a row takes
/// plus the header, with room to spare.
const MAX_HINT_BYTES: u64 = 64 * 1024;

/// The largest loadouts file read, and the largest the service writes.
pub(super) const MAX_LOADOUTS_BYTES: u64 = 4 * 1024 * 1024;

/// The most loadout rows an edit may leave in the file.
pub(super) const MAX_LOADOUT_ROWS: usize = 512;

/// The largest one loadout row may be, as it is written in the file.
pub(super) const MAX_LOADOUT_ROW_BYTES: usize = 64 * 1024;

/// Which current file a store is, and under which identity it is read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum StoreKey {
    /// The one operator loadouts file: every `loadouts-references`
    /// identity maps to it. The file is the operator's, not an account's
    /// or a content's: its rows are operator-authored presets, the same for
    /// every account and content scope.
    Loadouts,
    /// One account's hint file, read and written under one content scope:
    /// two scopes of one account are two stores, and a document stamped
    /// with one scope is refused under the other.
    BankHint {
        profile: String,
        account: String,
        revision: i32,
        content: String,
    },
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
                revision: identity.scope().revision().as_i32(),
                content: identity.scope().content().to_string(),
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
            StoreKey::BankHint {
                profile, account, ..
            } => root.bank_hint_file(profile, account),
        }
    }
}

/// The digest of a file's bytes: what a write's precondition compares
/// against the cache to see a change made since the service last read or
/// wrote the file, and what an idempotent write compares against.
pub(super) type ContentDigest = [u8; 32];

pub(super) fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).into()
}

/// Whether the service may rewrite a file it read: only when the rows it
/// holds are everything the file holds, so a rewrite loses nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rewrite {
    /// Every row was read and writes back as it was read.
    Lossless,
    /// Some rows could not be read (`unreadable`) or would not write back
    /// as they were read (`lossy`: unknown fields, a legacy shape): the
    /// rows read are leased, but the service never rewrites the file.
    Refused { unreadable: usize, lossy: usize },
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
    /// How many bytes the rows take as this service writes them: the
    /// pretty-printed array for loadouts (what the size budget projects
    /// from, whatever formatting the file on disk had), the file's length
    /// for a hint.
    pub(super) written_len: usize,
    /// Whether a write may replace the file.
    pub(super) rewrite: Rewrite,
}

/// Why a current file was refused. A refused file is never an empty store:
/// leases and writes refuse with [`ReadError::reason`] until a later read
/// accepts the file, and the file is never overwritten meanwhile.
#[derive(Debug, thiserror::Error)]
pub(super) enum ReadError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a JSON array of loadouts")]
    NotAnArray,
    #[error("schema_version {found} (this build reads {HINT_SCHEMA_VERSION})")]
    Schema { found: u32 },
    #[error("file names profile {profile:?} account {account:?}")]
    Identity { profile: String, account: String },
    #[error(
        "file was observed under revision {revision:?} content {content:?}; \
         remove it to start a hint for this content"
    )]
    Scope {
        revision: Option<i32>,
        content: Option<String>,
    },
    #[error("rows: {0}")]
    Rows(#[from] HintRowsError),
}

impl ReadError {
    /// The bounded failure classification a lease or write of the refused
    /// file reports.
    pub(super) fn reason(&self) -> FailureReason {
        match self {
            ReadError::Io(_) => FailureReason::Io,
            ReadError::Json(_)
            | ReadError::NotAnArray
            | ReadError::Schema { .. }
            | ReadError::Rows(_) => FailureReason::Schema,
            ReadError::Identity { .. } | ReadError::Scope { .. } => FailureReason::Identity,
        }
    }
}

/// The persisted hint document, schema 1:
///
/// ```json
/// {"schema_version":1,"profile":"local-289","account":"alice",
///  "revision":289,"content":"lostcity-289",
///  "observed_at_unix":1759700000,"rows":[[379,12],[995,3400]]}
/// ```
#[derive(Debug, Serialize, Deserialize)]
struct HintDocument {
    schema_version: u32,
    profile: String,
    account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
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
            let serde_json::Value::Array(elements) = serde_json::from_str(&raw)? else {
                return Err(ReadError::NotAnArray);
            };
            // A row the old reader skipped (malformed) is skipped here too,
            // so a file is never refused whole for one bad row; but then the
            // rows read are not everything the file holds, and the service
            // never rewrites it.
            let mut rows = Vec::with_capacity(elements.len());
            let (mut unreadable, mut lossy) = (0, 0);
            for element in &elements {
                match parse_loadout(element) {
                    Some(row) => {
                        if serde_json::to_value(&row).ok().as_ref() != Some(element) {
                            lossy += 1;
                        }
                        rows.push(row);
                    }
                    None => unreadable += 1,
                }
            }
            let rewrite = if unreadable == 0 && lossy == 0 {
                Rewrite::Lossless
            } else {
                Rewrite::Refused { unreadable, lossy }
            };
            let written_len = loadouts_bytes(&rows)?.len();
            Ok(Some(ReadDocument {
                snapshot: LeaseSnapshot::loadouts(rows),
                at: read_at,
                digest,
                written_len,
                rewrite,
            }))
        }
        StoreKey::BankHint {
            profile,
            account,
            revision,
            content,
        } => {
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
            match (&document.revision, &document.content) {
                (Some(found_revision), Some(found_content))
                    if found_revision == revision && found_content == content => {}
                // Written before r2: no scope recorded. Read under the scope
                // that asks; the first write stamps it.
                (None, None) => {}
                _ => {
                    return Err(ReadError::Scope {
                        revision: document.revision,
                        content: document.content,
                    })
                }
            }
            if document.rows.len() > MAX_HINT_ROWS {
                return Err(HintRowsError::TooManyRows.into());
            }
            Ok(Some(ReadDocument {
                snapshot: LeaseSnapshot::bank_hints(document.rows)?,
                at: DurableTimestamp::from_unix_seconds(document.observed_at_unix),
                digest,
                written_len: raw.len(),
                rewrite: Rewrite::Lossless,
            }))
        }
    }
}

/// The bytes a hint write publishes: the schema-1 document, stamped with
/// the store's content scope.
pub(super) fn hint_bytes(
    key: &StoreKey,
    rows: &[(i32, i32)],
    observed_at: DurableTimestamp,
) -> Result<Vec<u8>, serde_json::Error> {
    let StoreKey::BankHint {
        profile,
        account,
        revision,
        content,
    } = key
    else {
        unreachable!("a bank hint writes a bank-hint store");
    };
    serde_json::to_vec(&HintDocument {
        schema_version: HINT_SCHEMA_VERSION,
        profile: profile.clone(),
        account: account.clone(),
        revision: Some(*revision),
        content: Some(content.clone()),
        observed_at_unix: observed_at.unix_seconds(),
        rows: rows.to_vec(),
    })
}

/// The bytes a loadouts write publishes: the pretty-printed array, exactly
/// as the old store serialized it.
pub(super) fn loadouts_bytes(rows: &[Loadout]) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_string_pretty(rows).map(String::into_bytes)
}

/// How many bytes `row` adds to the pretty-printed loadouts array: the row
/// as it is printed one level deep, plus its indentation and separator.
/// The array's own brackets take two bytes whatever it holds, so a file of
/// rows `r` is exactly `2 + Σ row_bytes(r)` bytes long.
pub(super) fn row_bytes(row: &Loadout) -> Result<usize, serde_json::Error> {
    // A one-row array prints as `[\n  ` + the row one level deep + `\n]`,
    // the row plus six bytes. In any array a row costs itself plus four
    // (its `[\n  ` or `,\n  ` lead-in), and the closing `\n]` with nothing
    // before it is the two the brackets take: so the one-row array minus
    // two is the row's share.
    Ok(serde_json::to_string_pretty(std::slice::from_ref(row))?.len() - 2)
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

/// The directories whose entries make `path` reachable after a crash,
/// innermost first: its parent up to the state root, and the root's own
/// parent (where the root's entry lives when this write created it).
pub(super) fn entry_chain(path: &Path, root: &StateRoot) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut dir = path.parent();
    while let Some(current) = dir {
        chain.push(current.to_path_buf());
        if current == root.path() {
            if let Some(parent) = current.parent().filter(|p| !p.as_os_str().is_empty()) {
                chain.push(parent.to_path_buf());
            }
            break;
        }
        dir = current.parent();
    }
    chain
}

/// Flush `dir`'s entries to disk, so a name just renamed into it survives a
/// crash. A filesystem that cannot sync a directory says so with
/// `Unsupported`/`InvalidInput`; its renames are as durable as it makes
/// them, and that is not an error (the map cache's rule, `map_cache.rs`).
#[cfg(not(windows))]
pub(super) fn sync_directory(dir: &Path) -> io::Result<()> {
    match std::fs::File::open(dir).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
pub(super) fn sync_directory(_dir: &Path) -> io::Result<()> {
    // Rust's std directory open lacks FILE_FLAG_BACKUP_SEMANTICS; the staged
    // file is synced before the rename, and NTFS journals the rename itself.
    Ok(())
}
