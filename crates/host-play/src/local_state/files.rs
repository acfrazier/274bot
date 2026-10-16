//! The current files behind the state service (T5.2): the operator's
//! `loadouts.json` and `panel-ui.json` preferences, and one
//! `bank-hints/<profile>/<account>.json` per account. Each backend reads the
//! file its old private reader read and writes the shape its old private
//! writer wrote, so a file written before this service is read by it, and a
//! file it writes is read by an older build.
//!
//! No backend touches `HOME` or any default path: every path is derived
//! from the explicit [`StateRoot`] and the durable identity.
//!
//! What a read refuses is never treated as an absent file: a refused file is
//! a [`ReadError`], and the service keeps it refused (no lease, no write)
//! until a later read accepts it. Two refusals the service repairs, under
//! the file's lock, keeping the file's exact bytes first in a new copy
//! beside it ([`quarantine_copy`]). A loadouts file whose rows share a name
//! (an older editor could save one; names key the rows) is renamed apart
//! once, losing nothing, before anything leases or writes it
//! ([`loadouts_sharing_names`], [`rename_apart`]). A preferences file whose
//! content is malformed (not JSON, or JSON that is not an object) is moved
//! aside when a save reads it, and fresh preferences are started
//! ([`corrupt_preferences`]); every other read refuses it. A file whose
//! content the service cannot represent whole (a hint field it does not
//! know, a loadout row it cannot write back as read, a preference it
//! cannot hold as a row) is read but never rewritten ([`Rewrite::Refused`]),
//! the same rule in every domain: for preferences the rule is lossless
//! representability, so a member this build does not know (a version
//! marker among them) is held and written back like any other. What a
//! write publishes is durable before it is acknowledged: the bytes are
//! flushed in their staging file, renamed over the target, and then the
//! directory entries are flushed.
//!
//! Every writer this product runs holds the file's [`WriteLock`] across its
//! whole read-check-publish-verify transaction, so two services (two
//! processes on one `HOME`, or two compositions in one process) never
//! interleave on one file.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::{HintRowsError, MAX_HINT_ROWS};
use api::identity::DurableTimestamp;
use api::loadout::{parse_loadout, Loadout};
use api::local_state::{
    DomainIdentity, FailureReason, LeaseSnapshot, LoadoutEdit, LoadoutOp, LocalStateDomain,
    PreferenceRow,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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

    /// `<root>/panel-ui.json`: the one operator preferences file the panel
    /// and the TUI share.
    pub fn preferences_file(&self) -> PathBuf {
        self.0.join("panel-ui.json")
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

/// The largest preferences file read, and the largest the service writes.
pub const MAX_PREFERENCES_BYTES: u64 = 1024 * 1024;

/// The fields a schema-1 hint document carries. Any other field is content
/// the service cannot write back, so a document carrying one is read but
/// never rewritten.
const HINT_FIELDS: [&str; 7] = [
    "schema_version",
    "profile",
    "account",
    "revision",
    "content",
    "observed_at_unix",
    "rows",
];

/// Which current file a store is, and under which identity it is read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum StoreKey {
    /// The one operator loadouts file: every `loadouts-references`
    /// identity maps to it. The file is the operator's, not an account's
    /// or a content's: its rows are operator-authored presets, the same for
    /// every account and content scope.
    Loadouts,
    /// The one operator preferences file (`panel-ui.json`): every
    /// `preferences` identity maps to it, for the same reason.
    Preferences,
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
    /// has no current file (knowledge).
    pub(super) fn of(identity: &DomainIdentity) -> Option<Self> {
        match identity.domain() {
            LocalStateDomain::LoadoutsReferences => Some(StoreKey::Loadouts),
            LocalStateDomain::Preferences => Some(StoreKey::Preferences),
            LocalStateDomain::BankHints => Some(StoreKey::BankHint {
                profile: identity.account().profile().to_string(),
                account: identity.account().account().to_string(),
                revision: identity.scope().revision().as_i32(),
                content: identity.scope().content().to_string(),
            }),
            LocalStateDomain::Knowledge => None,
        }
    }

    pub(super) fn domain(&self) -> LocalStateDomain {
        match self {
            StoreKey::Loadouts => LocalStateDomain::LoadoutsReferences,
            StoreKey::Preferences => LocalStateDomain::Preferences,
            StoreKey::BankHint { .. } => LocalStateDomain::BankHints,
        }
    }

    pub(super) fn path(&self, root: &StateRoot) -> PathBuf {
        match self {
            StoreKey::Loadouts => root.loadouts_file(),
            StoreKey::Preferences => root.preferences_file(),
            StoreKey::BankHint {
                profile, account, ..
            } => root.bank_hint_file(profile, account),
        }
    }

    /// Whether every identity of the domain shares this one store (the
    /// operator files), rather than one store per account and scope.
    pub(super) fn operator_wide(&self) -> bool {
        matches!(self, StoreKey::Loadouts | StoreKey::Preferences)
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
/// holds are everything the file holds, so a rewrite loses nothing. The
/// rule is the same in every domain: content the service cannot hold as
/// rows (a loadout it cannot parse or write back as read, a hint field it
/// does not know, a preference it cannot represent as a row) makes the file
/// read-only to the service.
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
    /// for the operator files (which carry none).
    pub(super) at: DurableTimestamp,
    /// The digest of the bytes read.
    pub(super) digest: ContentDigest,
    /// How many bytes the rows take as this service writes them: the
    /// pretty-printed array for loadouts (what the size budget projects
    /// from, whatever formatting the file on disk had), the file's length
    /// for a hint or the preferences.
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
    #[error("not a JSON object of preferences")]
    NotAnObject,
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
    /// Loadout rows that share a name with an earlier row: names key the
    /// rows, so no edit of such a file names one row. The service renames
    /// them apart before it leases or writes the file, and refuses it while
    /// that cannot be done.
    #[error("{rows} loadout row(s) share a name with an earlier row")]
    SharedNames { rows: usize },
}

impl ReadError {
    /// The bounded failure classification a lease or write of the refused
    /// file reports.
    pub(super) fn reason(&self) -> FailureReason {
        match self {
            ReadError::Io(_) => FailureReason::Io,
            ReadError::Json(_)
            | ReadError::NotAnArray
            | ReadError::NotAnObject
            | ReadError::Schema { .. }
            | ReadError::Rows(_)
            | ReadError::SharedNames { .. } => FailureReason::Schema,
            ReadError::Identity { .. } | ReadError::Scope { .. } => FailureReason::Identity,
        }
    }

    /// Whether the read was refused for what the file says rather than for
    /// the file itself: not UTF-8 or not JSON, or JSON of the wrong shape.
    /// Never an owner, mode, size or I/O refusal (those read as
    /// `PermissionDenied` or another I/O kind, not `InvalidData`).
    pub(super) fn is_malformed(&self) -> bool {
        match self {
            ReadError::Json(_) | ReadError::NotAnArray | ReadError::NotAnObject => true,
            ReadError::Io(error) => error.kind() == io::ErrorKind::InvalidData,
            ReadError::Schema { .. }
            | ReadError::Rows(_)
            | ReadError::Identity { .. }
            | ReadError::Scope { .. }
            | ReadError::SharedNames { .. } => false,
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
/// file yet, `Ok(Some)` with the checked rows, `Err` for a refused file. A
/// loadouts file whose rows share a name is refused
/// ([`ReadError::SharedNames`]): the service renames them apart first.
pub(super) fn read(
    key: &StoreKey,
    root: &StateRoot,
    read_at: DurableTimestamp,
) -> Result<Option<ReadDocument>, ReadError> {
    let path = key.path(root);
    let max = max_bytes(key);
    let raw = match read_private_file(&path, max) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let digest = digest(raw.as_bytes());
    match key {
        StoreKey::Loadouts => {
            let (rows, rewrite) = loadout_rows(&raw)?;
            let shared = shared_names(&rows);
            if shared > 0 {
                return Err(ReadError::SharedNames { rows: shared });
            }
            let written_len = loadouts_bytes(&rows)?.len();
            Ok(Some(ReadDocument {
                snapshot: LeaseSnapshot::loadouts(rows),
                at: read_at,
                digest,
                written_len,
                rewrite,
            }))
        }
        StoreKey::Preferences => {
            let Value::Object(document) = serde_json::from_str(&raw)? else {
                return Err(ReadError::NotAnObject);
            };
            let (rows, unrepresentable) = preference_rows(&document);
            Ok(Some(ReadDocument {
                snapshot: LeaseSnapshot::preferences(rows),
                at: read_at,
                digest,
                written_len: raw.len(),
                rewrite: if unrepresentable == 0 {
                    Rewrite::Lossless
                } else {
                    Rewrite::Refused {
                        unreadable: unrepresentable,
                        lossy: 0,
                    }
                },
            }))
        }
        StoreKey::BankHint {
            profile,
            account,
            revision,
            content,
        } => {
            let value: Value = serde_json::from_str(&raw)?;
            // A field this build does not know is content a rewrite would
            // drop: the hint is read, but never rewritten (the loadouts
            // rule).
            let unknown = value.as_object().map_or(0, |fields| {
                fields
                    .keys()
                    .filter(|field| !HINT_FIELDS.contains(&field.as_str()))
                    .count()
            });
            let document: HintDocument = serde_json::from_value(value)?;
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
                rewrite: if unknown == 0 {
                    Rewrite::Lossless
                } else {
                    Rewrite::Refused {
                        unreadable: 0,
                        lossy: unknown,
                    }
                },
            }))
        }
    }
}

/// The largest file of `key`'s kind the service reads.
fn max_bytes(key: &StoreKey) -> u64 {
    match key {
        StoreKey::Loadouts => MAX_LOADOUTS_BYTES,
        StoreKey::Preferences => MAX_PREFERENCES_BYTES,
        StoreKey::BankHint { .. } => MAX_HINT_BYTES,
    }
}

/// The digest of what `key`'s file holds now, `None` when there is no
/// file: the checks a write makes immediately before its rename and after
/// its directory flush.
pub(super) fn current_digest(
    key: &StoreKey,
    root: &StateRoot,
) -> io::Result<Option<ContentDigest>> {
    match read_private_file(&key.path(root), max_bytes(key)) {
        Ok(raw) => Ok(Some(digest(raw.as_bytes()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The rows a loadouts document holds, and whether a rewrite keeps all of
/// it. A row the old reader skipped (malformed) is skipped here too, so a
/// file is never refused whole for one bad row; but then the rows read are
/// not everything the file holds, and the service never rewrites it.
fn loadout_rows(raw: &str) -> Result<(Vec<Loadout>, Rewrite), ReadError> {
    let Value::Array(elements) = serde_json::from_str(raw)? else {
        return Err(ReadError::NotAnArray);
    };
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
    Ok((rows, rewrite))
}

/// How many of `rows` have a name an earlier row already has.
fn shared_names(rows: &[Loadout]) -> usize {
    let mut seen = HashSet::with_capacity(rows.len());
    rows.iter()
        .filter(|row| !seen.insert(row.name.as_str()))
        .count()
}

/// The loadouts file at `path`, read again under its lock for the one-time
/// rename of rows that share a name: its exact text and its rows. `None`
/// when there is no file any more or every row has a name of its own
/// (repaired meanwhile): it is read as it is. A file this build refuses,
/// or one holding a row it cannot write back as read (a rewrite would lose
/// that row), is an error: it is never rewritten.
pub(super) fn loadouts_sharing_names(path: &Path) -> io::Result<Option<(String, Vec<Loadout>)>> {
    let text = match read_private_file(path, MAX_LOADOUTS_BYTES) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (rows, rewrite) = loadout_rows(&text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if shared_names(&rows) == 0 {
        return Ok(None);
    }
    if let Rewrite::Refused { unreadable, lossy } = rewrite {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the file also has {unreadable} unreadable row(s) and {lossy} row(s) that would \
                 not write back as read, which a rewrite would lose"
            ),
        ));
    }
    Ok(Some((text, rows)))
}

/// One row [`rename_apart`] renamed: its index, its name before, and its
/// name after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenamedApart {
    pub(super) row: usize,
    pub(super) from: String,
    pub(super) to: String,
}

/// Give every row whose name an earlier row already has a name of its own:
/// the second row of a name becomes `<name> (2)`, the third `<name> (3)`,
/// and so on, skipping a suffix another row (or an earlier rename) already
/// has. The first row of each name keeps it; every row keeps its content
/// and its place. The same rows always get the same names. A new name stays
/// within the bound Remove and Rename accept ([`new_name_apart`]), so every
/// renamed row can still be edited. The renames made, in row order.
pub(super) fn rename_apart(rows: &mut [Loadout]) -> Vec<RenamedApart> {
    let mut taken: HashSet<String> = rows.iter().map(|row| row.name.clone()).collect();
    let mut kept = HashSet::with_capacity(rows.len());
    // Per name, the next suffix to try: each search starts after the last
    // one taken, so a long run of one name costs no more than its length.
    let mut next: HashMap<String, u64> = HashMap::new();
    let mut renamed = Vec::new();
    for (row, loadout) in rows.iter_mut().enumerate() {
        if kept.insert(loadout.name.clone()) {
            continue;
        }
        let suffix = next.entry(loadout.name.clone()).or_insert(2);
        let name = loop {
            let candidate = new_name_apart(&loadout.name, *suffix);
            *suffix += 1;
            if taken.insert(candidate.clone()) {
                break candidate;
            }
        };
        renamed.push(RenamedApart {
            row,
            from: std::mem::replace(&mut loadout.name, name.clone()),
            to: name,
        });
    }
    renamed
}

/// `<name> (<n>)`, with the name's text shortened (on a character boundary)
/// when needed so the result, trimmed as Remove and Rename trim it, fits
/// [`api::local_state::MAX_PREF_KEY_CHARS`] bytes.
pub(super) fn new_name_apart(name: &str, n: u64) -> String {
    let suffix = format!(" ({n})");
    let body = name.trim_start();
    let lead = &name[..name.len() - body.len()];
    let room = api::local_state::MAX_PREF_KEY_CHARS.saturating_sub(suffix.len());
    let mut end = body.len().min(room);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{lead}{}{suffix}", &body[..end])
}

/// The bytes of the preferences file at `path` when its content is
/// malformed (not UTF-8, not JSON, or JSON that is not an object): the
/// file a reader may move aside. `None` when there is no file or it reads
/// as preferences. A file refused for its owner, its mode or its size is
/// an error here too, as it is to the reader: it is never moved.
pub(super) fn corrupt_preferences(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match read_private_file(path, MAX_PREFERENCES_BYTES) {
        Ok(text) => Ok(match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(_)) => None,
            _ => Some(text.into_bytes()),
        }),
        // Not UTF-8. The owner, mode and size checks passed before the
        // bytes were read (they refuse as `PermissionDenied`), so these
        // are the bytes to keep: read them as they are, from a regular
        // file, within the bound.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
            }
            let file = options.open(path)?;
            let refused = |what: &str| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{}: {what}", path.display()),
                )
            };
            if !file.metadata()?.is_file() {
                return Err(refused("not a regular file"));
            }
            let mut bytes = Vec::new();
            file.take(MAX_PREFERENCES_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
                return Err(refused(&format!(
                    "larger than {MAX_PREFERENCES_BYTES} bytes"
                )));
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// How many quarantine names one second offers before giving up.
const QUARANTINE_NAMES: u32 = 100;

/// Keep `bytes`, what `path` holds, in a new file beside it,
/// `<name>.<kind>-<unix seconds>` (`-<n>` added while that name is taken):
/// never over an existing file, read back to be sure it holds exactly
/// `bytes`, and flushed. The caller flushes the directory entry before it
/// replaces `path`. The quarantine's path.
pub(super) fn quarantine_copy(path: &Path, bytes: &[u8], kind: &str) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?
        .to_string_lossy()
        .into_owned();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for attempt in 0..QUARANTINE_NAMES {
        let candidate = path.with_file_name(match attempt {
            0 => format!("{name}.{kind}-{stamp}"),
            n => format!("{name}.{kind}-{stamp}-{n}"),
        });
        match vault::create_private_file(&candidate, bytes) {
            Ok(()) => {
                let mut file = File::open(&candidate)?;
                let mut held = Vec::with_capacity(bytes.len());
                (&mut file)
                    .take(bytes.len() as u64 + 1)
                    .read_to_end(&mut held)?;
                if held != bytes {
                    return Err(io::Error::other(format!(
                        "{}: the copy does not hold the file's bytes",
                        candidate.display()
                    )));
                }
                file.sync_all()?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{name}.{kind}-{stamp}: no free name"),
    ))
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

/// Why [`apply_loadout_edit`] refused a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenameRefusal {
    /// No row is called the rename's `from`.
    MissingSource,
    /// Another row already has the new name.
    NameTaken,
}

/// Apply `edit` to `rows`: an upsert replaces the row of the same name or
/// appends; a removal drops the row of that name; a rename replaces the
/// row called `from` where it stands. A removal that names no row leaves
/// the rows as they are, and the write settles as the idempotent no-op it
/// is. A rename never inserts a row: one whose source is gone, or whose
/// new name another row has, is refused and nothing changes. The rows'
/// names are their own (a read refuses a file whose rows share a name
/// until the service has renamed them apart, and no edit gives two rows
/// one name), so every edit names at most one row.
pub(super) fn apply_loadout_edit(
    rows: &[Loadout],
    edit: &LoadoutEdit,
) -> Result<Vec<Loadout>, RenameRefusal> {
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
        LoadoutOp::Rename { from, loadout } => {
            let at = next
                .iter()
                .position(|row| row.name == *from)
                .ok_or(RenameRefusal::MissingSource)?;
            if next.iter().any(|row| row.name == loadout.name) {
                return Err(RenameRefusal::NameTaken);
            }
            next[at] = loadout.clone();
        }
    }
    Ok(next)
}

/// One preferences document (`panel-ui.json`, a JSON object) as rows. Each
/// top-level member is one row: its name, and its value as compact JSON
/// text. A member whose value is too long for one row and is an object is
/// held as one row per nested member, `<member>.<nested>` (a member name
/// carries no `.`, so the first `.` splits the key). A member that fits
/// neither way (a name outside the row alphabet or carrying a `.`, a nested
/// value still too long) cannot be held: it is counted, and a file with any
/// is read but never rewritten.
pub(super) fn preference_rows(document: &Map<String, Value>) -> (Vec<PreferenceRow>, usize) {
    let mut rows = Vec::with_capacity(document.len());
    let mut unrepresentable = 0;
    for (name, value) in document {
        if name.contains('.') {
            unrepresentable += 1;
            continue;
        }
        if let Ok(row) = PreferenceRow::try_new(name, &value.to_string()) {
            rows.push(row);
            continue;
        }
        let nested = match value {
            Value::Object(members) if !members.is_empty() => members
                .iter()
                .map(|(member, value)| {
                    PreferenceRow::try_new(&format!("{name}.{member}"), &value.to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .ok(),
            _ => None,
        };
        match nested {
            Some(nested) => rows.extend(nested),
            None => unrepresentable += 1,
        }
    }
    (rows, unrepresentable)
}

/// The preferences document `rows` hold: the inverse of
/// [`preference_rows`]. `None` when a row's value is not JSON, or a member
/// is both one row and nested rows (never produced by a read or a write).
pub(super) fn preference_document(rows: &[PreferenceRow]) -> Option<Map<String, Value>> {
    let mut document = Map::new();
    for row in rows {
        let value: Value = serde_json::from_str(row.value()).ok()?;
        match row.key().split_once('.') {
            Some((name, member)) => {
                let Value::Object(members) = document
                    .entry(name.to_string())
                    .or_insert_with(|| Value::Object(Map::new()))
                else {
                    return None;
                };
                members.insert(member.to_string(), value);
            }
            None => {
                if document.insert(row.key().to_string(), value).is_some() {
                    return None;
                }
            }
        }
    }
    Some(document)
}

/// Apply one preferences write to `document`: key `<member>` sets that
/// member to `value` (JSON text); key `<member>.<nested>` sets one member
/// of the object `<member>`, creating it when absent. `None` (nothing
/// changed) when `value` is not JSON or `<member>` is not an object.
pub(super) fn apply_preferences_edit(
    document: &mut Map<String, Value>,
    key: &str,
    value: &str,
) -> Option<()> {
    let value: Value = serde_json::from_str(value).ok()?;
    match key.split_once('.') {
        Some((name, member)) => {
            let Value::Object(members) = document
                .entry(name.to_string())
                .or_insert_with(|| Value::Object(Map::new()))
            else {
                return None;
            };
            members.insert(member.to_string(), value);
        }
        None => {
            document.insert(key.to_string(), value);
        }
    }
    Some(())
}

/// The bytes a preferences write publishes: the pretty-printed object,
/// exactly as the old shared writer serialized it.
pub(super) fn preferences_bytes(
    document: &Map<String, Value>,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(document)
}

/// How long a writer waits for another writer's [`WriteLock`] on a file
/// before it gives the write up as busy.
pub(super) const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// The advisory lock every writer this product runs holds on one current
/// file across its whole read-check-publish-verify transaction: an
/// exclusive lock (`flock` on Unix, `LockFileEx` on Windows, through std's
/// `File::try_lock`) on `<file>.lock` beside it. It is the rule the shared
/// `panel-ui.json` writer already followed (`resource_view.rs` before T5.2
/// r3), so a preference writer of an older build and this service exclude
/// each other too. The lock file is never removed: removing it while
/// another process holds it open would let two writers lock two different
/// files. Released when dropped.
pub(super) struct WriteLock {
    _file: File,
}

/// Why [`lock_for_write`] gave up.
#[derive(Debug)]
pub(super) enum LockError {
    /// Another writer held the lock past the deadline.
    TimedOut,
    /// The caller asked to stop waiting (the service's shutdown bound).
    Abandoned,
    /// The lock file could not be created or locked.
    Io(io::Error),
}

/// `<file>.lock`, beside `path`.
pub(super) fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

/// Take `path`'s [`WriteLock`], waiting until `deadline` while another
/// writer holds it, and giving up early once `abandoned` says so. The
/// directory is created (`0o700`) when missing, as a write would.
pub(super) fn lock_for_write(
    path: &Path,
    deadline: Instant,
    abandoned: &dyn Fn() -> bool,
) -> Result<WriteLock, LockError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        vault::create_private_dir(parent).map_err(LockError::Io)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(lock_path(path)).map_err(LockError::Io)?;
    let mut wait = Duration::from_millis(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(WriteLock { _file: file }),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Err(LockError::Io(error)),
        }
        if abandoned() {
            return Err(LockError::Abandoned);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(LockError::TimedOut);
        }
        std::thread::sleep(wait.min(deadline - now));
        wait = (wait * 2).min(Duration::from_millis(50));
    }
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
