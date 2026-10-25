//! Import of rs2b0t's own profile export (`rs2b0t-profiles.json`, written by
//! rs2b0t's Settings → export; rs2b0t `src/bot/multibox/ProfileTransfer.ts`).
//!
//! The file is rs2b0t's unlocked vault plus each account's box storage, in
//! plain text. [`parse_profile_file`] checks it as strictly as rs2b0t's own
//! `parseProfileFile` does (kind, version, shapes; `storage` may be absent,
//! as there). Its errors name a field and the type of value found, never
//! without writing anything: the preview. The front ends stage its planned
//! rows, then the existing CreateProfile and SaveProfile commands write each
//! account through the session's ordinary profile writers. Every item the
//! import does not carry over is listed with its reason in the preview and
//! final report.
//! preview and in the final report.
//!
//! Password handling, and what it does not cover:
//! - The file is read into one buffer sized from the file's length, which
//!   becomes a [`Secret`] (zeroed on drop); a file that is not UTF-8 is
//!   zeroed. The account passwords are copied into [`Secret`]s.
//! - The file is checked to be JSON before any value tree is built, so a
//!   malformed file builds no partial tree. Once the tree exists, every
//!   string value in it is zeroed when it is dropped.
//! - Every non-empty password in the file is registered for log redaction
//!   ([`api::hostlog::register_secret`]) as soon as the file parses, before
//!   any preview text exists: also for an account the import then skips,
//!   and when the import is never confirmed. Every string the importer
//!   builds to show, or keeps in a type that derives `Debug` (preview and
//!   report lines, [`Skipped`] items and reasons, per-parameter lines,
//!   refusals), passes through [`api::hostlog::redact`] when it is built.
//!   So a password that turns up again in any other field (a tab, a stored
//!   key or value, a clue duel partner, even another account's name) shows
//!   as `***` there, and in every copy the panel, the TUI and the log
//!   message keep. The log line's account column (its slot) is not
//!   redacted: it names the account, even one whose name equals a
//!   password. The values written into the vault are the file's, unchanged.
//! - Not covered (best effort, as [`Secret`] itself): object keys; a value
//!   that a repeated key in the same object replaced (serde_json drops it
//!   before this module sees the tree); serde_json's own scratch buffer
//!   for strings with escapes; a buffer the read had to grow because the
//!   file grew while it was read; and the process-lifetime log redaction
//!   list, which keeps a copy of every password of every parsed file (and
//!   of every saved password) so the log never shows one.
//!
//! [`ProfileImport`] is the flow the panel and the TUI both render: path,
//! preview, confirm, report, and the optional deletion of the export file.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(feature = "js")]
use api::host_log;
#[cfg(feature = "js")]
use api::hostlog::{Category, Level};
use script::bank_hints::MAX_ACCOUNT_CHARS;
use serde_json::{Number, Value};
use vault::{Profile, ProfileSettings, RasterMode, ScriptAssignment, Secret, Vault};
use zeroize::{Zeroize, Zeroizing};

#[cfg(feature = "js")]
use crate::command::OperatorCommand;
use crate::operations::OperationId;
#[cfg(feature = "js")]
use crate::scripts::Scripts;
use crate::session::OperatorSession;
#[cfg(feature = "js")]
use crate::session::{ArmMirror, StagedProfileWrite};

/// rs2b0t's `PROFILE_FILE_KIND`.
pub const PROFILE_FILE_KIND: &str = "rs2b0t-multibox-profiles";
/// rs2b0t's `PROFILE_FILE_VERSION`; the only version this importer reads.
pub const PROFILE_FILE_VERSION: u64 = 1;
/// rs2b0t's `PROFILE_FILE_NAME`: the name its export suggests.
pub const PROFILE_FILE_NAME: &str = "rs2b0t-profiles.json";
/// Shown with every preview and report.
pub const PLAINTEXT_WARNING: &str = "This export file holds your rs2b0t account passwords in plain text. Delete it after the import, or keep it somewhere safe.";
/// The label failed writes carry on the banner and status line.
pub const LABEL: &str = "rs2b0t import";
/// The most characters of a login password. The 289 server refuses a
/// longer (or empty) one before it checks anything else (lostcity-289
/// `engine/src/engine/World.ts:2221-2225`, reply 3), and the bound client's
/// title form stops at 20 (`vendor/fr-client-rust`
/// `crates/client/src/client/client.rs:10823-10825`). The login name's
/// limit is [`MAX_ACCOUNT_CHARS`] (`World.ts:2215-2219`, client
/// `client.rs:10807-10809`).
pub const MAX_PASSWORD_CHARS: usize = 20;

/// rs2b0t's tab for accounts without one; it is never stored explicitly.
const MAIN_TAB: &str = "Main";
/// An export of a few hundred accounts is well under a megabyte.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The type of a JSON value an error found where another was expected.
/// Errors carry this, never the value: a malformed header may hold
/// anything, a password included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonType {
    Missing,
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}

impl JsonType {
    fn of(value: Option<&Value>) -> Self {
        match value {
            None => Self::Missing,
            Some(Value::Null) => Self::Null,
            Some(Value::Bool(_)) => Self::Boolean,
            Some(Value::Number(_)) => Self::Number,
            Some(Value::String(_)) => Self::String,
            Some(Value::Array(_)) => Self::Array,
            Some(Value::Object(_)) => Self::Object,
        }
    }

    /// How a found `kind` reads in its error (a string is a different one).
    fn as_kind(self) -> &'static str {
        match self {
            Self::String => "a different string",
            other => other.article(),
        }
    }

    /// How a found `v` reads in its error (a number is a different one).
    fn as_version(self) -> &'static str {
        match self {
            Self::Number => "a number other than 1",
            other => other.article(),
        }
    }

    fn article(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Null => "null",
            Self::Boolean => "true or false",
            Self::Number => "a number",
            Self::String => "a string",
            Self::Array => "a list",
            Self::Object => "an object",
        }
    }
}

/// Why a profile file was refused. Details remain private, and the original
/// I/O cause is available through [`std::error::Error::source`].
pub struct ImportError(ImportFailure);

impl std::fmt::Debug for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ImportFailure::Read { source, .. } => Some(source.as_ref()),
            #[cfg(feature = "js")]
            ImportFailure::Delete { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<ImportFailure> for ImportError {
    fn from(failure: ImportFailure) -> Self {
        Self(failure)
    }
}

/// Why an import failed. No variant carries a password or the file's text:
/// a malformed export can hold anything, so errors name only paths, entry
/// numbers and JSON types.
#[derive(Debug, thiserror::Error)]
enum ImportFailure {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: Box<std::io::Error>,
    },
    #[error("{} is not a regular file, so it is not an rs2b0t profile export", path.display())]
    NotAFile { path: PathBuf },
    #[error("{} is larger than 16 MiB, so it is not an rs2b0t profile export", path.display())]
    TooLarge { path: PathBuf },
    #[error("the profile file is not JSON")]
    NotJson,
    #[error("the profile file has an unrecognized shape")]
    Shape,
    #[error("the profile file's kind is {}, expected the string '{PROFILE_FILE_KIND}': this is not an rs2b0t profile export", .found.as_kind())]
    Kind { found: JsonType },
    #[error("the profile file's version (v) is {}, expected {PROFILE_FILE_VERSION}: this export comes from an rs2b0t version this importer does not read", .found.as_version())]
    Version { found: JsonType },
    #[error("the profile file is missing profiles, tabs, or activeTab")]
    MissingLists,
    #[error("the profile file tabs must be strings")]
    TabsNotStrings,
    #[error("profile entry {index} is invalid: {reason}")]
    Entry { index: usize, reason: &'static str },
    #[error("profile entry {index} has an invalid world: expected 1, 2 or 3")]
    World { index: usize },
    #[error("profile entry {index} has an invalid tab")]
    Tab { index: usize },
    #[error("the profile file storage must be an object")]
    Storage,
    #[error("the profile file storage has an empty username")]
    StorageUser,
    #[error("the profile file storage for {owner} must be an object")]
    StorageBox { owner: StorageOwner },
    #[error("the profile file storage for {owner} has a value that is not a string")]
    StorageValue { owner: StorageOwner },
    #[cfg(feature = "js")]
    #[error("the vault is locked: unlock it before importing")]
    VaultLocked,
    #[cfg(feature = "js")]
    #[error("type the path of the {PROFILE_FILE_NAME} file rs2b0t exported")]
    EmptyPath,
    #[cfg(feature = "js")]
    #[error("the export file was not confirmed for deletion")]
    DeleteNotConfirmed,
    #[cfg(feature = "js")]
    #[error("the export file is no longer available for deletion")]
    DeleteUnavailable,
    #[cfg(feature = "js")]
    #[error("there is no parsed profile preview to confirm")]
    NoPreview,
    #[cfg(feature = "js")]
    #[error("an imported profile account is already staged")]
    AccountAlreadyStaged,
    #[cfg(feature = "js")]
    #[error("profile import account {index} is not a pending write")]
    AccountNotPending { index: usize },
    #[cfg(feature = "js")]
    #[error("profile import account {index} was not submitted")]
    AccountNotSubmitted { index: usize },
    #[cfg(feature = "js")]
    #[error("profile import account {index} requires {expected:?}, not {actual:?}")]
    WrongWriter {
        index: usize,
        expected: OperatorCommand,
        actual: OperatorCommand,
    },
    #[cfg(feature = "js")]
    #[error("cannot delete {}: {source}", path.display())]
    Delete {
        path: PathBuf,
        #[source]
        source: Box<std::io::Error>,
    },
}

/// Whose storage a storage error is about: an account the file lists (its
/// 1-based number in the list) or another key. Neither shows the key: a
/// damaged file can put any text there, a password included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageOwner {
    Listed(usize),
    Unlisted,
}

impl std::fmt::Display for StorageOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Listed(number) => write!(f, "account {number}"),
            Self::Unlisted => f.write_str("an account the file does not list"),
        }
    }
}

/// One account from the file. The password stays in a [`Secret`].
pub struct ExportAccount {
    username: String,
    password: Secret,
    tab: Option<String>,
    world: Option<u8>,
}

impl ExportAccount {
    pub fn username(&self) -> &str {
        &self.username
    }

    /// rs2b0t tab; `None` is its Main tab.
    pub fn tab(&self) -> Option<&str> {
        self.tab.as_deref()
    }

    pub fn world(&self) -> Option<u8> {
        self.world
    }
}

/// File text stays out of `Debug`: a damaged file can put a password in
/// any field, so only the shape is shown.
impl std::fmt::Debug for ExportAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportAccount")
            .field("username", &"<hidden>")
            .field("password", &"<redacted>")
            .field("tab", &self.tab.as_ref().map(|_| "<hidden>"))
            .field("world", &self.world)
            .finish()
    }
}

/// One account's rs2b0t box storage: key suffix (after `rs2b0t:<user>:`)
/// to the stored string. The `creds` value (rs2b0t's per-tab login copy,
/// which holds the password) is never kept: its key maps to an empty value.
pub type BoxStorage = BTreeMap<String, String>;

/// rs2b0t's box key holding a per-tab copy of the login.
const CREDS_KEY: &str = "creds";

/// A parsed rs2b0t profile export.
pub struct ExportFile {
    accounts: Vec<ExportAccount>,
    tabs: Vec<String>,
    active_tab: String,
    storage: BTreeMap<String, BoxStorage>,
}

impl std::fmt::Debug for ExportFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportFile")
            .field("accounts", &self.accounts)
            .field("tabs", &self.tabs.len())
            .field("storage_boxes", &self.storage.len())
            .finish_non_exhaustive()
    }
}

impl ExportFile {
    pub fn accounts(&self) -> &[ExportAccount] {
        &self.accounts
    }

    pub fn tabs(&self) -> &[String] {
        &self.tabs
    }

    pub fn active_tab(&self) -> &str {
        &self.active_tab
    }

    pub fn storage(&self) -> &BTreeMap<String, BoxStorage> {
        &self.storage
    }
}

/// Zero every string value in a parsed JSON tree when it goes out of scope,
/// so the passwords it held do not stay in freed memory (best effort, as
/// [`Secret`]; what this does not reach is listed in the module docs).
struct WipeOnDrop(Value);

impl Drop for WipeOnDrop {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

fn wipe(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(map) => map.values_mut().for_each(wipe),
        _ => {}
    }
}

/// Open `path` for reading without waiting: a FIFO with no writer would
/// otherwise block the caller's frame in `open`. A directory opens too, so
/// the check on the handle refuses it as not a file on every platform.
fn open_for_reading(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    // Without `FILE_FLAG_BACKUP_SEMANTICS` Windows refuses to open a
    // directory at all (access denied).
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    }
    options.open(path)
}

/// Read at most `limit` bytes from `reader` into a buffer that is zeroed
/// when dropped; `Ok(None)` when there is more. `expected` (the file's
/// length) sizes the buffer, so reading a file that does not grow meanwhile
/// never reallocates it and leaves no unzeroed copy behind.
fn read_bounded(
    reader: impl Read,
    limit: u64,
    expected: u64,
) -> std::io::Result<Option<Zeroizing<Vec<u8>>>> {
    let capacity = usize::try_from(expected.min(limit).saturating_add(1)).unwrap_or(0);
    let mut bytes = Zeroizing::new(Vec::with_capacity(capacity));
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// Read and parse the export at `path`: a regular file of at most 16 MiB,
/// bounded on the opened handle (whatever its metadata said). The file
/// text is wiped once parsed.
pub fn read_profile_file(path: &Path) -> Result<ExportFile, ImportError> {
    read_profile_file_inner(path).map_err(ImportError::from)
}

fn read_profile_file_inner(path: &Path) -> Result<ExportFile, ImportFailure> {
    let read_error = |source| ImportFailure::Read {
        path: path.to_path_buf(),
        source: Box::new(source),
    };
    let file = open_for_reading(path).map_err(read_error)?;
    let metadata = file.metadata().map_err(read_error)?;
    if !metadata.is_file() {
        return Err(ImportFailure::NotAFile {
            path: path.to_path_buf(),
        });
    }
    let Some(mut bytes) = read_bounded(file, MAX_FILE_BYTES, metadata.len()).map_err(read_error)?
    else {
        return Err(ImportFailure::TooLarge {
            path: path.to_path_buf(),
        });
    };
    let text = match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(text) => Secret::from(text),
        Err(error) => {
            error.into_bytes().zeroize();
            return Err(ImportFailure::NotJson);
        }
    };
    parse_profile_file_inner(&text)
}

/// Parse an rs2b0t profile export, with the checks of rs2b0t's
/// `parseProfileFile`. No error quotes the file: entries are named by
/// index, header values by their JSON type. A file that parses has every
/// password registered for log redaction before it is returned, so no
/// text built from it later (preview, report, log line) can show one,
/// whichever account it came from and whichever field repeats it.
pub fn parse_profile_file(text: &str) -> Result<ExportFile, ImportError> {
    parse_profile_file_inner(text).map_err(ImportError::from)
}

fn parse_profile_file_inner(text: &str) -> Result<ExportFile, ImportFailure> {
    // Check the syntax first without building values, so a malformed file
    // never leaves a partly built tree (and its strings) unwiped.
    serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|_| ImportFailure::NotJson)?;
    let parsed = WipeOnDrop(serde_json::from_str(text).map_err(|_| ImportFailure::NotJson)?);
    let obj = parsed.0.as_object().ok_or(ImportFailure::Shape)?;
    let kind = obj.get("kind");
    if kind.and_then(Value::as_str) != Some(PROFILE_FILE_KIND) {
        return Err(ImportFailure::Kind {
            found: JsonType::of(kind),
        });
    }
    let version = obj.get("v");
    if version.and_then(Value::as_f64) != Some(PROFILE_FILE_VERSION as f64) {
        return Err(ImportFailure::Version {
            found: JsonType::of(version),
        });
    }
    let (Some(profiles), Some(tabs), Some(active_tab)) = (
        obj.get("profiles").and_then(Value::as_array),
        obj.get("tabs").and_then(Value::as_array),
        obj.get("activeTab").and_then(Value::as_str),
    ) else {
        return Err(ImportFailure::MissingLists);
    };
    let tabs = tabs
        .iter()
        .map(|tab| tab.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or(ImportFailure::TabsNotStrings)?;
    let accounts = profiles
        .iter()
        .enumerate()
        .map(|(index, raw)| parse_account(index + 1, raw))
        .collect::<Result<Vec<_>, _>>()?;
    let storage = parse_storage(obj.get("storage"), &accounts)?;
    for account in &accounts {
        api::hostlog::register_secret(account.password.as_str());
    }
    Ok(ExportFile {
        accounts,
        tabs,
        active_tab: active_tab.to_owned(),
        storage,
    })
}

fn parse_account(index: usize, raw: &Value) -> Result<ExportAccount, ImportFailure> {
    let entry = |reason| ImportFailure::Entry { index, reason };
    let p = raw.as_object().ok_or(entry("not an object"))?;
    let username = p
        .get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or(entry("username must be a non-empty string"))?;
    let password = p
        .get("password")
        .and_then(Value::as_str)
        .ok_or(entry("password must be a string"))?;
    let world = match p.get("world") {
        None => None,
        Some(world) => match world.as_f64() {
            Some(n) if n == 1.0 || n == 2.0 || n == 3.0 => Some(n as u8),
            _ => return Err(ImportFailure::World { index }),
        },
    };
    let tab = match p.get("tab") {
        None => None,
        Some(Value::String(tab)) if tab == MAIN_TAB => None,
        Some(Value::String(tab)) => Some(tab.clone()),
        Some(_) => return Err(ImportFailure::Tab { index }),
    };
    Ok(ExportAccount {
        username: username.to_owned(),
        password: Secret::from(password),
        tab,
        world,
    })
}

fn parse_storage(
    raw: Option<&Value>,
    accounts: &[ExportAccount],
) -> Result<BTreeMap<String, BoxStorage>, ImportFailure> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let users = raw.as_object().ok_or(ImportFailure::Storage)?;
    let owner = |user: &str| match accounts.iter().position(|a| a.username == user) {
        Some(index) => StorageOwner::Listed(index + 1),
        None => StorageOwner::Unlisted,
    };
    let mut out = BTreeMap::new();
    for (user, entries) in users {
        if user.is_empty() {
            return Err(ImportFailure::StorageUser);
        }
        let entries = entries
            .as_object()
            .ok_or_else(|| ImportFailure::StorageBox { owner: owner(user) })?;
        let mut storage = BoxStorage::new();
        for (key, value) in entries {
            let value = value
                .as_str()
                .ok_or_else(|| ImportFailure::StorageValue { owner: owner(user) })?;
            let kept = if key == CREDS_KEY {
                String::new()
            } else {
                value.to_owned()
            };
            storage.insert(key.clone(), kept);
        }
        out.insert(user.clone(), storage);
    }
    Ok(out)
}

// ---- mapping --------------------------------------------------------------

/// What happens to an account that is already in the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Keep the vault's profile as it is.
    #[default]
    Skip,
    /// Write the export's password and world and the settings it carries
    /// into the profile; every other setting and parameter is kept.
    Overwrite,
}

/// The operator's choices for one preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportChoices {
    pub policy: ConflictPolicy,
    /// Import accounts the 289 client cannot log in with as well
    /// ([`CredentialProblem`]); off by default.
    pub include_unusable: bool,
}

impl From<ConflictPolicy> for ImportChoices {
    fn from(policy: ConflictPolicy) -> Self {
        Self {
            policy,
            include_unusable: false,
        }
    }
}

/// Why the 289 client cannot log in with an account's credentials as the
/// export holds them. Never carries the credential; the import never
/// shortens one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialProblem {
    UsernameTooLong,
    UsernameCharacters,
    PasswordEmpty,
    PasswordTooLong,
    PasswordCharacters,
}

impl CredentialProblem {
    pub fn reason(self) -> &'static str {
        match self {
            Self::UsernameTooLong => {
                "the username is longer than 12 characters, which the 289 server refuses at login"
            }
            Self::UsernameCharacters => {
                "the username has a character the 289 login cannot carry (a line break, or a character beyond Latin-1)"
            }
            Self::PasswordEmpty => "the password is empty, which the 289 server refuses at login",
            Self::PasswordTooLong => {
                "the password is longer than 20 characters, which the 289 server refuses at login"
            }
            Self::PasswordCharacters => {
                "the password has a character the 289 login cannot carry (a line break, or a character beyond Latin-1)"
            }
        }
    }
}

/// Whether one login field survives the 289 login: the client writes each
/// character as one byte ending at a line break (`vendor/fr-client-rust`
/// `crates/client/src/io/packet.rs:319-326`), and the server reads each
/// byte back as one character (lostcity-289 `engine/src/io/Packet.ts:267-276`).
fn carried_by_login(field: &str) -> bool {
    field.chars().all(|c| c != '\n' && u32::from(c) <= 0xff)
}

/// What keeps the 289 client from logging in with `username` (already
/// trimmed, not empty) and `password`.
fn credential_problem(username: &str, password: &str) -> Option<CredentialProblem> {
    if username.chars().count() > MAX_ACCOUNT_CHARS {
        Some(CredentialProblem::UsernameTooLong)
    } else if !carried_by_login(username) {
        Some(CredentialProblem::UsernameCharacters)
    } else if password.is_empty() {
        Some(CredentialProblem::PasswordEmpty)
    } else if password.chars().count() > MAX_PASSWORD_CHARS {
        Some(CredentialProblem::PasswordTooLong)
    } else if !carried_by_login(password) {
        Some(CredentialProblem::PasswordCharacters)
    } else {
        None
    }
}

/// A catalog card the import can assign and fill.
pub struct CatalogCard<'a> {
    schema: &'a [script::SettingDef],
    unloadable: Option<&'a str>,
    assignment: ScriptAssignment,
}

impl<'a> CatalogCard<'a> {
    pub fn new(
        schema: &'a [script::SettingDef],
        unloadable: Option<&'a str>,
        assignment: ScriptAssignment,
    ) -> Self {
        Self {
            schema,
            unloadable,
            assignment,
        }
    }

    pub fn schema(&self) -> &'a [script::SettingDef] {
        self.schema
    }

    /// Why the card cannot run in 274bot (it is dimmed in Browse).
    pub fn unloadable(&self) -> Option<&'a str> {
        self.unloadable
    }

    pub fn assignment(&self) -> &ScriptAssignment {
        &self.assignment
    }
}

/// The script catalog as the import sees it: rs2b0t script names are
/// catalog register names.
pub trait ImportCatalog {
    /// Whether the rs2b0t catalog is registered in this session.
    fn loaded(&self) -> bool;
    fn card(&self, name: &str) -> Option<CatalogCard<'_>>;
}

#[cfg(feature = "js")]
impl ImportCatalog for script_js::JsLibrary {
    fn loaded(&self) -> bool {
        self.cards()
            .iter()
            .any(|card| card.source == script::ScriptSource::Catalog)
    }

    fn card(&self, name: &str) -> Option<CatalogCard<'_>> {
        self.get(script::ScriptSource::Catalog, name).map(|card| {
            CatalogCard::new(
                &card.settings_schema,
                card.unloadable.as_deref(),
                card.assignment(),
            )
        })
    }
}

/// `text` as the importer may show, log or keep it: every registered
/// password ([`parse_profile_file`] registers the file's) replaced by
/// [`api::hostlog::REDACTED`]. Applied where each displayed string is
/// built, so every later copy is already clean. No copy when nothing
/// matches.
fn shown(text: String) -> String {
    let redacted = match api::hostlog::redact(&text) {
        Cow::Owned(clean) => Some(clean),
        Cow::Borrowed(_) => None,
    };
    redacted.unwrap_or(text)
}

/// One rs2b0t item the import does not carry over, and why. Both strings
/// are redacted when they are built: an item or reason can quote file
/// text, and a password can sit in any field.
#[derive(Debug, PartialEq, Eq)]
pub struct Skipped {
    item: String,
    reason: String,
}

impl Skipped {
    pub fn item(&self) -> &str {
        &self.item
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

fn skipped(item: impl Into<String>, reason: impl Into<String>) -> Skipped {
    Skipped {
        item: shown(item.into()),
        reason: shown(reason.into()),
    }
}

/// What the import does with one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    /// Not in the vault: created.
    New,
    /// In the vault and Overwrite chosen: the export's values written in.
    Overwrite,
    /// In the vault and already equal to what Overwrite would write.
    Unchanged,
    /// In the vault and Skip chosen: kept as it is.
    Exists,
    /// The file lists this username again; its first entry is used.
    Duplicate,
    /// The username is only spaces.
    BlankName,
    /// The 289 client cannot log in with it ([`AccountPlan::problem`]), and
    /// importing such accounts was not chosen.
    Unusable,
}

impl AccountStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "new profile",
            Self::Overwrite => {
                "overwrite existing profile (settings the export does not carry are kept)"
            }
            Self::Unchanged => "already matches the export: nothing to write",
            Self::Exists => "already in the vault: kept as it is (choose Overwrite to replace it)",
            Self::Duplicate => "listed twice in the file: only its first entry is imported",
            Self::BlankName => "the username is only spaces: not imported",
            Self::Unusable => {
                "unusable: the 289 client cannot log in with it, so it is not imported"
            }
        }
    }
}

/// One script parameter the export carries, converted.
struct ParamImport {
    /// The card's bag key in the profile.
    card: String,
    /// `<Script>.<key>` as rs2b0t names it.
    item: String,
    setting: String,
    value: Value,
    note: Option<String>,
}

/// One script parameter of a known card the export carries but the import
/// refused; `skip` indexes [`Mapped::skipped`].
struct ParamRefused {
    card: String,
    setting: String,
    skip: usize,
}

/// The 274bot side of one account's mapped fields.
#[derive(Default)]
struct Mapped {
    world: Option<u16>,
    lamp_skill: Option<String>,
    clue_duel_partner: Option<String>,
    renderer_on: Option<bool>,
    assignment: Option<ScriptAssignment>,
    params: Vec<ParamImport>,
    refused: Vec<ParamRefused>,
    imported: Vec<String>,
    skipped: Vec<Skipped>,
}

impl Mapped {
    /// Write what the export carries into `settings`. Nothing the export
    /// lacks is changed: a card's parameters are merged key by key, and a
    /// refused value never reaches its bag.
    fn apply(&self, settings: &mut ProfileSettings) {
        if let Some(world) = self.world {
            settings.world = Some(world);
        }
        if let Some(skill) = &self.lamp_skill {
            settings.lamp_skill.clone_from(skill);
        }
        if let Some(partner) = &self.clue_duel_partner {
            settings.clue_duel_partner.clone_from(partner);
        }
        match self.renderer_on {
            Some(false) => settings.raster = RasterMode::Off,
            Some(true) if settings.raster == RasterMode::Off => settings.raster = RasterMode::Gpu,
            _ => {}
        }
        if let Some(assignment) = &self.assignment {
            settings.script_assignment = Some(assignment.clone());
        }
        for param in &self.params {
            settings
                .script_settings
                .entry(param.card.clone())
                .or_default()
                .insert(param.setting.clone(), param.value.clone());
        }
    }

    /// The preview lines for what is imported and kept, and the skips,
    /// against the profile's settings `before` (an existing profile): per
    /// parameter, its new value against the current one; every other
    /// parameter of an imported card that stays; and, for a refused value,
    /// the current value that stays. Every line is redacted as it is built.
    fn describe(mut self, before: Option<&ProfileSettings>) -> Described {
        let current = |card: &str, setting: &str| {
            before
                .and_then(|s| s.script_settings.get(card))
                .and_then(|bag| bag.get(setting))
        };
        let mut imported: Vec<String> = std::mem::take(&mut self.imported)
            .into_iter()
            .map(shown)
            .collect();
        for param in &self.params {
            let mut line = format!(
                "{} = {}",
                param.item,
                script::format_setting_value(&param.value)
            );
            if let Some(note) = &param.note {
                let _ = write!(line, " ({note})");
            }
            if before.is_some() {
                match current(&param.card, &param.setting) {
                    None => line.push_str(" [new]"),
                    Some(old) if *old == param.value => line.push_str(" [unchanged]"),
                    Some(old) => {
                        let _ = write!(line, " [was {}]", script::format_setting_value(old));
                    }
                }
            }
            imported.push(shown(line));
        }
        for refused in &self.refused {
            let Some(old) = current(&refused.card, &refused.setting) else {
                continue;
            };
            let skip = &mut self.skipped[refused.skip];
            let _ = write!(
                skip.reason,
                "; the profile keeps its current value {}",
                script::format_setting_value(old)
            );
            skip.reason = shown(std::mem::take(&mut skip.reason));
        }
        let mut kept = Vec::new();
        if let Some(before) = before {
            let mut cards = BTreeMap::<&str, &str>::new();
            for param in &self.params {
                let script = param.item.split_once('.').map_or("", |(s, _)| s);
                cards.insert(&param.card, script);
            }
            for (card, script) in cards {
                let Some(bag) = before.script_settings.get(card) else {
                    continue;
                };
                for (key, value) in bag {
                    let carried = self
                        .params
                        .iter()
                        .map(|p| (&p.card, &p.setting))
                        .chain(self.refused.iter().map(|r| (&r.card, &r.setting)))
                        .any(|(c, s)| c == card && s == key);
                    if !carried {
                        kept.push(shown(format!(
                            "{script}.{key} = {} (not in the export)",
                            script::format_setting_value(value)
                        )));
                    }
                }
            }
        }
        Described {
            imported,
            kept,
            skipped: self.skipped,
        }
    }
}

/// [`Mapped::describe`]'s lines.
struct Described {
    imported: Vec<String>,
    kept: Vec<String>,
    skipped: Vec<Skipped>,
}

/// One account in the preview.
pub struct AccountPlan {
    username: String,
    status: AccountStatus,
    /// Why the 289 client cannot log in with it, if it cannot.
    problem: Option<CredentialProblem>,
    /// The row to write (New and Overwrite only).
    row: Option<Profile>,
    /// The vault row it replaces (Overwrite only), for the log line.
    old: Option<Profile>,
    imported: Vec<String>,
    kept: Vec<String>,
    skipped: Vec<Skipped>,
}

impl AccountPlan {
    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn status(&self) -> AccountStatus {
        self.status
    }

    /// Why the 289 client cannot log in with this account, if it cannot.
    pub fn problem(&self) -> Option<CredentialProblem> {
        self.problem
    }

    /// What the import carries over, one line per item (no password). On
    /// an existing profile each parameter says `[new]`, `[unchanged]` or
    /// `[was <value>]`.
    pub fn imported(&self) -> &[String] {
        &self.imported
    }

    /// The existing profile's parameters of the imported cards that the
    /// export does not carry: they stay as they are.
    pub fn kept(&self) -> &[String] {
        &self.kept
    }

    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// The settings the import writes (New and Overwrite only).
    pub fn settings(&self) -> Option<&ProfileSettings> {
        self.row.as_ref().map(|row| &row.settings)
    }

    /// The detail lines under the account, in the preview and the report.
    fn detail_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(problem) = self.problem {
            out.push(format!(
                "  cannot log in on the 289 server: {}",
                problem.reason()
            ));
        }
        if self.row.is_some() {
            out.push("  password: imported into the vault".into());
            out.extend(self.imported.iter().map(|line| format!("  {line}")));
            out.extend(self.kept.iter().map(|line| format!("  kept: {line}")));
        }
        out
    }
}

/// The preview: every account and file-level item, nothing written.
pub struct ImportPlan {
    choices: ImportChoices,
    accounts: Vec<AccountPlan>,
    file_skipped: Vec<Skipped>,
}

impl ImportPlan {
    pub fn policy(&self) -> ConflictPolicy {
        self.choices.policy
    }

    pub fn choices(&self) -> ImportChoices {
        self.choices
    }

    pub fn accounts(&self) -> &[AccountPlan] {
        &self.accounts
    }

    /// Items of the file that belong to no single account.
    pub fn file_skipped(&self) -> &[Skipped] {
        &self.file_skipped
    }

    pub fn count(&self, status: AccountStatus) -> usize {
        self.accounts.iter().filter(|a| a.status == status).count()
    }

    /// Accounts the 289 client cannot log in with, imported or not.
    pub fn unusable(&self) -> usize {
        self.accounts.iter().filter(|a| a.problem.is_some()).count()
    }

    /// Whether confirming would write anything.
    pub fn writes(&self) -> bool {
        self.accounts.iter().any(|a| a.row.is_some())
    }

    /// The preview text, shared by the panel and the TUI. Each line is
    /// redacted whole: an account's name, or parts joined into one line,
    /// can spell a password no single part held.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!(
                "{} accounts: {} new, {} to overwrite, {} unchanged, {} kept as they are, {} duplicate, {} unusable.",
                self.accounts.len(),
                self.count(AccountStatus::New),
                self.count(AccountStatus::Overwrite),
                self.count(AccountStatus::Unchanged),
                self.count(AccountStatus::Exists),
                self.count(AccountStatus::Duplicate),
                self.count(AccountStatus::Unusable),
            ),
            format!(
                "Existing profiles: {}.",
                match self.choices.policy {
                    ConflictPolicy::Skip => "skip (kept as they are)",
                    ConflictPolicy::Overwrite =>
                        "overwrite (settings and parameters the export does not carry are kept)",
                }
            ),
        ];
        let unusable = self.unusable();
        if unusable > 0 {
            let (accounts, are) = if unusable == 1 {
                ("account", "is")
            } else {
                ("accounts", "are")
            };
            out.push(if self.choices.include_unusable {
                format!("{unusable} {accounts} the 289 client cannot log in with {are} imported anyway, as chosen.")
            } else {
                format!("{unusable} {accounts} the 289 client cannot log in with {are} not imported (you can choose to import them anyway).")
            });
        }
        for account in &self.accounts {
            out.push(String::new());
            out.push(format!("{}: {}", account.username, account.status.label()));
            out.extend(account.detail_lines());
            for skip in &account.skipped {
                out.push(format!("  not imported: {}: {}", skip.item, skip.reason));
            }
        }
        if !self.file_skipped.is_empty() {
            out.push(String::new());
            for skip in &self.file_skipped {
                out.push(format!("not imported: {}: {}", skip.item, skip.reason));
            }
        }
        out.push(String::new());
        out.push(PLAINTEXT_WARNING.into());
        out.into_iter().map(shown).collect()
    }
}

/// Map the file onto the vault: the preview. Writes nothing. A new
/// profile takes `first_uid` ([`crate::OperatorSession::fresh_uid`]) and
/// the uids after it.
pub fn plan_import(
    file: &ExportFile,
    vault: &Vault,
    first_uid: i32,
    catalog: &impl ImportCatalog,
    choices: impl Into<ImportChoices>,
) -> ImportPlan {
    let choices = choices.into();
    let mut next_uid = first_uid;
    let mut seen = BTreeSet::new();
    let mut accounts = Vec::with_capacity(file.accounts.len());
    for account in &file.accounts {
        let username = account.username.trim().to_owned();
        let status_only = |username: String, status, problem| AccountPlan {
            username,
            status,
            problem,
            row: None,
            old: None,
            imported: Vec::new(),
            kept: Vec::new(),
            skipped: Vec::new(),
        };
        if username.is_empty() {
            accounts.push(status_only(
                account.username.clone(),
                AccountStatus::BlankName,
                None,
            ));
            continue;
        }
        if !seen.insert(username.clone()) {
            accounts.push(status_only(username, AccountStatus::Duplicate, None));
            continue;
        }
        let existing = vault.get(&username);
        if existing.is_some() && choices.policy == ConflictPolicy::Skip {
            accounts.push(status_only(username, AccountStatus::Exists, None));
            continue;
        }
        let problem = credential_problem(&username, account.password.as_str());
        if problem.is_some() && !choices.include_unusable {
            accounts.push(status_only(username, AccountStatus::Unusable, problem));
            continue;
        }
        let mapped = map_account(account, file.storage.get(&account.username), catalog);
        let (status, row, old) = match existing {
            None => {
                let mut settings = ProfileSettings::default();
                mapped.apply(&mut settings);
                let row = Profile {
                    username: username.clone(),
                    password: account.password.clone(),
                    uid: next_uid,
                    settings,
                };
                next_uid += 1;
                (AccountStatus::New, Some(row), None)
            }
            Some(current) => {
                let mut row = current.clone();
                row.password = account.password.clone();
                mapped.apply(&mut row.settings);
                if row == *current {
                    (AccountStatus::Unchanged, None, None)
                } else {
                    (AccountStatus::Overwrite, Some(row), Some(current.clone()))
                }
            }
        };
        let described = mapped.describe(existing.map(|p| &p.settings));
        accounts.push(AccountPlan {
            username,
            status,
            problem,
            row,
            old,
            imported: described.imported,
            kept: described.kept,
            skipped: described.skipped,
        });
    }
    ImportPlan {
        choices,
        accounts,
        file_skipped: file_skipped(file),
    }
}

fn file_skipped(file: &ExportFile) -> Vec<Skipped> {
    let mut out = Vec::new();
    if !file.tabs.is_empty() {
        out.push(skipped(
            format!("tabs ({})", file.tabs.join(", ")),
            "274bot has no profile tabs or groups",
        ));
    }
    if file.active_tab != MAIN_TAB {
        out.push(skipped(
            format!("active tab '{}'", file.active_tab),
            "274bot has no profile tabs or groups",
        ));
    }
    let listed: BTreeSet<&str> = file.accounts.iter().map(|a| a.username.as_str()).collect();
    for (user, storage) in &file.storage {
        if !listed.contains(user.as_str()) {
            out.push(skipped(
                format!("stored settings for '{user}' ({} items)", storage.len()),
                "the file has no account with this name",
            ));
        }
    }
    out
}

/// rs2b0t `Global` settings 274bot has no per-account equivalent for, with
/// the reason each is skipped (rs2b0t `src/bot/runtime/Settings.ts`,
/// `GLOBAL_SETTINGS_CORE` and `NAV_SETTINGS`). `lampSkill` and
/// `clueDuelPartner` are mapped in [`map_global`]. The route-drawing ones
/// have app-wide panel equivalents (`panel` `NavSettings`), which one
/// account's import does not change.
const GLOBAL_NOT_SUPPORTED: [(&str, &str); 20] = [
    (
        "bankCommonJunk",
        GLOBAL_SCRIPT_DEFAULT,
    ),
    (
        "useZanarisBank",
        GLOBAL_SCRIPT_DEFAULT,
    ),
    (
        "useMageBank",
        GLOBAL_SCRIPT_DEFAULT,
    ),
    (
        "runAuto",
        "274bot turns run back on by itself (scripts may change that); there is no per-account setting",
    ),
    (
        "runEnergyMin",
        "274bot turns run back on at 20% energy (scripts may change that); there is no per-account setting",
    ),
    (
        "navTeleports",
        "274bot's teleport permission is one app-wide switch (Nav config: allow teleports), not per account",
    ),
    ("navPathStallTicks", "274bot's walker repaths by its own rules; there is no setting"),
    ("navPathDeviation", "274bot's walker repaths by its own rules; there is no setting"),
    (
        "navCameraFollow",
        "app-wide in 274bot (panel Nav settings: camera follows path), not per account",
    ),
    ("showNavPath", PANEL_PATH_PAINT),
    ("navPathShowText", PANEL_PATH_PAINT),
    ("navPathTextSize", PANEL_PATH_PAINT),
    ("navPathColorPath", PANEL_PATH_PAINT),
    ("navPathColorTransport", PANEL_PATH_PAINT),
    ("navPathColorClick", PANEL_PATH_PAINT),
    ("navPathColorText", PANEL_PATH_PAINT),
    (
        "navPathSceneExpand",
        "rs2b0t experimental route-drawing option; 274bot has no equivalent",
    ),
    ("navPathClientSegment", PANEL_PATH_PAINT),
    ("navPathColorClient", PANEL_PATH_PAINT),
    ("navPathColorClientRunAlt", PANEL_PATH_PAINT),
];

/// rs2b0t `Settings.ts` `winningRaw`: a script parameter with no saved value
/// of its own falls back to the `Global` value of the same key.
const GLOBAL_SCRIPT_DEFAULT: &str = "in rs2b0t this is the account-wide default for scripts that have this parameter; 274bot has no account-wide default, so set it in each script's parameters";

const PANEL_PATH_PAINT: &str =
    "route drawing is app-wide in 274bot (panel Nav settings), not per account";

fn map_account(
    account: &ExportAccount,
    storage: Option<&BoxStorage>,
    catalog: &impl ImportCatalog,
) -> Mapped {
    let mut mapped = Mapped {
        world: account.world.map(u16::from),
        ..Mapped::default()
    };
    if let Some(world) = account.world {
        mapped.imported.push(format!(
            "world: w{world} (used when the server's world list offers it)"
        ));
    }
    if let Some(tab) = &account.tab {
        mapped.skipped.push(skipped(
            format!("tab '{tab}'"),
            "274bot has no profile tabs or groups",
        ));
    }
    let Some(storage) = storage else {
        return mapped;
    };
    for (key, value) in storage {
        if key == "selectedScript" {
            map_selected_script(&mut mapped, value, catalog);
        } else if key == "rendererEnabled" {
            match value.as_str() {
                "0" => {
                    mapped.renderer_on = Some(false);
                    mapped
                        .imported
                        .push("display: off (rs2b0t renderer disabled)".into());
                }
                "1" => {
                    mapped.renderer_on = Some(true);
                    mapped
                        .imported
                        .push("display: on (rs2b0t renderer enabled)".into());
                }
                other => mapped.skipped.push(skipped(
                    "rendererEnabled",
                    format!("unrecognized value '{other}'"),
                )),
            }
        } else if key == CREDS_KEY {
            mapped.skipped.push(skipped(
                "creds",
                "rs2b0t's per-tab copy of the login; the password comes from the account list (value not shown)",
            ));
        } else if let Some(rest) = key.strip_prefix("set:") {
            match rest.split_once(':') {
                Some(("Global", setting)) => map_global(&mut mapped, setting, value),
                Some(("MapPicker", setting)) => mapped.skipped.push(skipped(
                    format!("MapPicker.{setting}"),
                    "rs2b0t tile-map picker display option; 274bot's map has its own settings",
                )),
                Some(("Loadouts", setting)) => mapped.skipped.push(skipped(
                    format!("Loadouts.{setting}"),
                    "rs2b0t loadouts are not imported; 274bot keeps one loadout list for all accounts in its Loadouts window",
                )),
                Some(("PriceBooks", setting)) => mapped.skipped.push(skipped(
                    format!("PriceBooks.{setting}"),
                    "order books are not supported in 274bot",
                )),
                Some((script, setting)) => {
                    map_script_setting(&mut mapped, script, setting, value, catalog)
                }
                None => mapped.skipped.push(skipped(
                    key.clone(),
                    "unrecognized rs2b0t storage key",
                )),
            }
        } else {
            mapped
                .skipped
                .push(skipped(key.clone(), "unrecognized rs2b0t storage key"));
        }
    }
    mapped
}

const CATALOG_NOT_LOADED: &str = "the rs2b0t script catalog is not loaded in 274bot (import it from Scripts first, then preview again)";

fn map_selected_script(mapped: &mut Mapped, name: &str, catalog: &impl ImportCatalog) {
    let item = format!("selected script '{name}'");
    if name.is_empty() {
        mapped.skipped.push(skipped(item, "no script selected"));
        return;
    }
    if !catalog.loaded() {
        mapped.skipped.push(skipped(item, CATALOG_NOT_LOADED));
        return;
    }
    match catalog.card(name) {
        None => mapped
            .skipped
            .push(skipped(item, "274bot's catalog has no script of this name")),
        Some(card) => match card.unloadable {
            Some(reason) => mapped.skipped.push(skipped(
                item,
                format!("dimmed in 274bot, it cannot run here ({reason}); its parameters are still imported"),
            )),
            None => {
                mapped.imported.push(format!("script: {name}"));
                mapped.assignment = Some(card.assignment);
            }
        },
    }
}

fn map_global(mapped: &mut Mapped, setting: &str, value: &str) {
    let item = format!("Global.{setting}");
    match setting {
        "lampSkill" => {
            let skill = value.trim().to_ascii_lowercase();
            if host::is_lamp_skill(&skill) {
                mapped.imported.push(format!("lamp skill: {skill}"));
                mapped.lamp_skill = Some(skill);
            } else {
                mapped.skipped.push(skipped(
                    item,
                    format!("'{value}' is not a skill the genie lamp offers"),
                ));
            }
        }
        "clueDuelPartner" => {
            let partner = value.trim().to_owned();
            mapped.imported.push(if partner.is_empty() {
                "clue duel partner: (none)".to_string()
            } else {
                format!("clue duel partner: {partner}")
            });
            mapped.clue_duel_partner = Some(partner);
        }
        _ => {
            let reason = GLOBAL_NOT_SUPPORTED
                .iter()
                .find(|(key, _)| *key == setting)
                .map_or("not a Global setting this importer knows", |(_, reason)| {
                    reason
                });
            mapped.skipped.push(skipped(item, reason));
        }
    }
}

fn map_script_setting(
    mapped: &mut Mapped,
    script: &str,
    setting: &str,
    raw: &str,
    catalog: &impl ImportCatalog,
) {
    let item = format!("{script}.{setting}");
    if !catalog.loaded() {
        mapped.skipped.push(skipped(item, CATALOG_NOT_LOADED));
        return;
    }
    let Some(card) = catalog.card(script) else {
        mapped.skipped.push(skipped(
            item,
            format!("274bot's catalog has no script named '{script}'"),
        ));
        return;
    };
    let refuse = |mapped: &mut Mapped, reason: String| {
        mapped.refused.push(ParamRefused {
            card: card.assignment.key(),
            setting: setting.to_owned(),
            skip: mapped.skipped.len(),
        });
        mapped.skipped.push(skipped(item.clone(), reason));
    };
    let Some(def) = card.schema.iter().find(|def| def.id == setting) else {
        refuse(
            mapped,
            format!("'{script}' has no parameter '{setting}' in 274bot's catalog"),
        );
        return;
    };
    match convert_setting(def, raw) {
        Ok(converted) => {
            for entry in converted.dropped {
                mapped.skipped.push(skipped(
                    format!("{item} entry '{entry}'"),
                    "not one of this parameter's options",
                ));
            }
            let value = script::migrate_legacy_setting_value(script, setting, &converted.value);
            mapped.params.push(ParamImport {
                card: card.assignment.key(),
                item,
                setting: setting.to_owned(),
                value,
                note: converted.note,
            });
        }
        Err(reason) => refuse(mapped, reason),
    }
}

/// An rs2b0t setting string converted to 274bot's stored value.
struct Converted {
    value: Value,
    /// How the value differs from the stored text, when it does.
    note: Option<String>,
    /// List entries rs2b0t itself drops (not one of the options).
    dropped: Vec<String>,
}

/// ECMAScript's whitespace and line terminators (`StrWhiteSpaceChar`):
/// what JavaScript's `trim()` and `Number()` strip.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JavaScript's `Number(text)` for a string (ECMAScript `StringToNumber`),
/// which rs2b0t uses to read stored numbers and tile parts
/// (`src/bot/runtime/Settings.ts:77-82,123-128`): surrounding whitespace is
/// trimmed; empty is 0; `0x`/`0o`/`0b` read an unsigned integer in base
/// 16/8/2; otherwise an optionally signed decimal with optional fraction
/// and exponent, or `Infinity`. Anything else is NaN (so is `-0x10`).
fn js_string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(js_space);
    if text.is_empty() {
        return 0.0;
    }
    let bytes = text.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return js_radix_integer(&text[2..], radix);
        }
    }
    let (negative, body) = match bytes[0] {
        b'+' => (false, &text[1..]),
        b'-' => (true, &text[1..]),
        _ => (false, text),
    };
    let magnitude = if body == "Infinity" {
        f64::INFINITY
    } else if js_decimal_literal(body) {
        // Rust reads this grammar, correctly rounded.
        body.parse().unwrap_or(f64::NAN)
    } else {
        return f64::NAN;
    };
    if negative {
        -magnitude
    } else {
        magnitude
    }
}

/// ECMAScript `StrUnsignedDecimalLiteral` without `Infinity`: digits with
/// an optional fraction (at least one digit in all) and an optional
/// exponent with at least one digit.
fn js_decimal_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let whole = digits(0);
    let mut at = whole;
    let mut fraction = 0;
    if bytes.get(at) == Some(&b'.') {
        fraction = digits(at + 1);
        at += 1 + fraction;
    }
    if whole + fraction == 0 {
        return false;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = digits(at);
        if exponent == 0 {
            return false;
        }
        at += exponent;
    }
    at == bytes.len()
}

/// The value of `digits` in base 2, 8 or 16, rounded to the nearest double
/// (ties to even) as ECMAScript does; NaN without digits or with another
/// character.
fn js_radix_integer(digits: &str, radix: u32) -> f64 {
    let bits_per_digit = radix.trailing_zeros();
    let mut top: u64 = 0;
    let mut top_bits = 0u32;
    let mut dropped_bits = 0i32;
    let mut sticky = false;
    for c in digits.chars() {
        let Some(digit) = c.to_digit(radix) else {
            return f64::NAN;
        };
        for shift in (0..bits_per_digit).rev() {
            let bit = u64::from((digit >> shift) & 1);
            if top_bits == 0 && bit == 0 {
                continue;
            }
            if top_bits < 64 {
                top = (top << 1) | bit;
                top_bits += 1;
            } else {
                sticky |= bit == 1;
                dropped_bits = dropped_bits.saturating_add(1);
            }
        }
    }
    if digits.is_empty() {
        return f64::NAN;
    }
    // 64 kept bits are more than a double's 53 plus its rounding bit, so
    // folding every dropped 1 into the lowest bit rounds exactly once.
    let value = (top | u64::from(sticky)) as f64;
    value * 2f64.powi(dropped_bits)
}

/// rs2b0t's `parseValue` (`Settings.ts`) for one stored string: the value
/// the account actually ran with. A string rs2b0t would replace with the
/// default is refused instead, so the skip is reported (and an existing
/// profile keeps its own value).
fn convert_setting(def: &script::SettingDef, raw: &str) -> Result<Converted, String> {
    let plain = |value| {
        Ok(Converted {
            value,
            note: None,
            dropped: Vec::new(),
        })
    };
    let option = |wanted: &str| {
        def.options
            .iter()
            .find(|option| option.eq_ignore_ascii_case(wanted))
            .cloned()
    };
    match def.ty.as_str() {
        "boolean" => {
            let normalized = raw.trim().to_ascii_lowercase();
            plain(Value::Bool(matches!(
                normalized.as_str(),
                "true" | "1" | "yes"
            )))
        }
        "number" => {
            let parsed = js_string_to_number(raw);
            if !parsed.is_finite() {
                return Err(format!(
                    "'{raw}' is not a finite number (rs2b0t used the parameter's default instead)"
                ));
            }
            let bound = |text: &Option<String>| text.as_deref().and_then(|t| t.parse::<f64>().ok());
            let mut clamped = parsed;
            if let Some(min) = bound(&def.min) {
                clamped = clamped.max(min);
            }
            if let Some(max) = bound(&def.max) {
                clamped = clamped.min(max);
            }
            let number = Number::from_f64(clamped)
                .ok_or_else(|| format!("'{raw}' is not a finite number"))?;
            let mut notes = Vec::new();
            if raw.trim().parse::<f64>().ok() != Some(parsed) {
                notes.push(format!("'{raw}' read as {parsed}, as rs2b0t does"));
            }
            if clamped != parsed {
                notes.push(format!(
                    "{parsed} limited to the parameter's range, as rs2b0t does"
                ));
            }
            Ok(Converted {
                value: script::coerce_setting_value("number", &Value::Number(number)),
                note: (!notes.is_empty()).then(|| notes.join("; ")),
                dropped: Vec::new(),
            })
        }
        "string" if !def.options.is_empty() => {
            let chosen = option(raw.trim())
                .ok_or_else(|| format!("'{raw}' is not one of this parameter's options"))?;
            plain(Value::String(chosen))
        }
        "string" => plain(Value::String(raw.trim().to_owned())),
        "string[]" | "list" => {
            let entries = raw.split(',').map(str::trim).filter(|s| !s.is_empty());
            if def.options.is_empty() {
                return plain(Value::Array(
                    entries.map(|s| Value::String(s.to_owned())).collect(),
                ));
            }
            let mut values = Vec::new();
            let mut dropped = Vec::new();
            for entry in entries {
                match option(entry) {
                    Some(chosen) => values.push(Value::String(chosen)),
                    None => dropped.push(entry.to_owned()),
                }
            }
            Ok(Converted {
                value: Value::Array(values),
                note: None,
                dropped,
            })
        }
        "tile" => {
            // rs2b0t `parseTile`: `Number(part.trim())` for every part, at
            // least two, all finite; the level defaults to 0.
            let parts: Vec<f64> = raw.split(',').map(js_string_to_number).collect();
            if parts.len() < 2 || parts.iter().any(|part| !part.is_finite()) {
                return Err(format!("'{raw}' is not a tile (x,z or x,z,level)"));
            }
            let whole =
                |n: f64| (n.fract() == 0.0 && n.abs() <= i64::MAX as f64).then_some(n as i64);
            let level = parts.get(2).copied().unwrap_or(0.0);
            match (whole(parts[0]), whole(parts[1]), whole(level)) {
                (Some(x), Some(z), Some(level)) => plain(serde_json::json!({
                    "x": x,
                    "z": z,
                    "level": level,
                })),
                _ => Err(format!(
                    "'{raw}' has a coordinate that is not a whole number; 274bot tiles are whole tiles"
                )),
            }
        }
        other => Err(format!("parameter type '{other}' is not supported")),
    }
}

#[cfg(feature = "js")]
/// A prepared account write; the profile is consumed by its writer row.
pub(crate) struct StagedImportAccount {
    pub(crate) index: usize,
    pub(crate) username: String,
    pub(crate) write: StagedProfileWrite,
}

#[cfg(feature = "js")]
struct ImportWrite {
    index: usize,
    username: String,
    command: OperatorCommand,
    profile: Option<Profile>,
    summary: String,
}
/// One account's write.
struct RunEntry {
    username: String,
    status: AccountStatus,
    op: Option<OperationId>,
    /// Refused before anything was written.
    refused: Option<String>,
    /// The preview's detail lines for a written account: per parameter
    /// what was written and what stays.
    details: Vec<String>,
}

/// A confirmed import: its queued writes, then its final report.
///
/// Each write's result is kept here the first time it is seen settled, so
/// it never depends on the session's bounded operation history again. A
/// result is read from the session (and kept) whenever the run is asked
/// for one; a write whose record is gone before it was seen settled is
/// [`AccountResult::Unknown`], never a success.
pub struct ImportRun {
    entries: Vec<RunEntry>,
    skipped: Vec<(String, Skipped)>,
    /// Per entry, its settled result once seen.
    seen: Vec<OnceCell<StoredAccountResult>>,
}

/// One account's result, borrowing any failure text from its import run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountResult<'a> {
    /// The import's own write of a new profile is durable.
    Created,
    /// The import's own write into an existing profile is durable.
    Overwritten,
    /// Nothing written: unchanged, kept, duplicate, blank or unusable.
    NotWritten(AccountStatus),
    Failed(&'a str),
    /// A later save of the same profile was written in place of this
    /// import's write, so the import's own data is not confirmed durable.
    Superseded,
    /// The session no longer knows what became of the write (its record
    /// was dropped before the import saw it settle): not confirmed.
    Unknown,
    /// The write is still being saved.
    Saving,
}

impl AccountResult<'_> {
    /// The import's own data for this account is confirmed durable.
    pub fn is_durable(&self) -> bool {
        matches!(self, Self::Created | Self::Overwritten)
    }
}

/// Only terminal results enter the cache; their owning payload never leaves it.
enum StoredAccountResult {
    Created,
    Overwritten,
    NotWritten(AccountStatus),
    Failed(String),
    Superseded,
    Unknown,
}

impl StoredAccountResult {
    fn view(&self) -> AccountResult<'_> {
        match self {
            Self::Created => AccountResult::Created,
            Self::Overwritten => AccountResult::Overwritten,
            Self::NotWritten(status) => AccountResult::NotWritten(*status),
            Self::Failed(error) => AccountResult::Failed(error),
            Self::Superseded => AccountResult::Superseded,
            Self::Unknown => AccountResult::Unknown,
        }
    }
}

// ---- apply and report ------------------------------------------------------

#[cfg(feature = "js")]
/// `old -> new` for every field the import writes; the password only as
/// set/changed/unchanged.
fn change_summary(old: Option<&Profile>, new: &Profile) -> String {
    let default = ProfileSettings::default();
    let before = old.map_or(&default, |p| &p.settings);
    let after = &new.settings;
    let world = |w: Option<u16>| w.map_or_else(|| "auto".to_string(), |n| format!("w{n}"));
    let mut parts = vec![match old {
        None => "password set".to_string(),
        Some(old) if old.password == new.password => "password unchanged".into(),
        Some(_) => "password changed".into(),
    }];
    parts.push(format!(
        "world {} -> {}",
        world(before.world),
        world(after.world)
    ));
    if before.lamp_skill != after.lamp_skill {
        parts.push(format!(
            "lamp skill {} -> {}",
            before.lamp_skill, after.lamp_skill
        ));
    }
    if before.clue_duel_partner != after.clue_duel_partner {
        parts.push(format!(
            "clue duel partner '{}' -> '{}'",
            before.clue_duel_partner, after.clue_duel_partner
        ));
    }
    if before.raster != after.raster {
        parts.push(format!("display {:?} -> {:?}", before.raster, after.raster));
    }
    let key = |s: &ProfileSettings| {
        s.script_assignment
            .as_ref()
            .map_or_else(|| "None".to_string(), ScriptAssignment::key)
    };
    parts.push(format!("script {} -> {}", key(before), key(after)));
    for (card, bag) in &after.script_settings {
        if before.script_settings.get(card) != Some(bag) {
            parts.push(format!("{card} parameters: {} saved", bag.len()));
        }
    }
    parts.join("; ")
}

impl ImportRun {
    /// One entry's result: kept once settled; otherwise read from the
    /// session's record of its own write.
    fn result<Io>(&self, index: usize, core: &OperatorSession<Io>) -> AccountResult<'_> {
        if let Some(result) = self.seen[index].get() {
            return result.view();
        }
        let entry = &self.entries[index];
        let result = if let Some(error) = &entry.refused {
            StoredAccountResult::Failed(error.clone())
        } else if let Some(op) = entry.op {
            match core.operation(op).and_then(|r| r.outcome(&entry.username)) {
                Some(outcome) if outcome.is_pending() => return AccountResult::Saving,
                Some(outcome) if outcome.is_completed() && entry.status == AccountStatus::New => {
                    StoredAccountResult::Created
                }
                Some(outcome) if outcome.is_completed() => StoredAccountResult::Overwritten,
                Some(outcome) if outcome.is_failed() || outcome.is_skipped() => {
                    StoredAccountResult::Failed(shown(
                        outcome.reason().unwrap_or_default().to_owned(),
                    ))
                }
                // A later write of the same profile in the same commit:
                // whether that commit held this import's data is not known.
                Some(outcome) if outcome.is_cancelled() => StoredAccountResult::Superseded,
                // The record was dropped before it was seen settled.
                None | Some(_) => StoredAccountResult::Unknown,
            }
        } else {
            StoredAccountResult::NotWritten(entry.status)
        };
        self.seen[index].get_or_init(|| result).view()
    }

    /// Every account's result now, borrowing failure text from this run.
    /// A settled result is kept from the first time it is seen.
    pub fn results<Io>(&self, core: &OperatorSession<Io>) -> Vec<(&str, AccountResult<'_>)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.username.as_str(), self.result(index, core)))
            .collect()
    }

    /// Whether every write has settled.
    pub fn settled<Io>(&self, core: &OperatorSession<Io>) -> bool {
        !self
            .results(core)
            .iter()
            .any(|(_, result)| *result == AccountResult::Saving)
    }

    /// Whether every account the import chose to write is confirmed
    /// durable with the import's own data (accounts it did not write, by
    /// choice or because they are unusable, do not count).
    pub fn succeeded<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.results(core).iter().all(|(_, result)| {
            result.is_durable() || matches!(result, AccountResult::NotWritten(_))
        })
    }

    /// The final report, shared by the panel and the TUI; each line is
    /// redacted whole, as the preview's.
    pub fn lines<Io>(&self, core: &OperatorSession<Io>) -> Vec<String> {
        let results = self.results(core);
        let count =
            |want: fn(&AccountResult) -> bool| results.iter().filter(|(_, r)| want(r)).count();
        let mut out = vec![format!(
            "Import report: {} created, {} overwritten, {} not written, {} failed, {} not confirmed, {} still saving.",
            count(|r| *r == AccountResult::Created),
            count(|r| *r == AccountResult::Overwritten),
            count(|r| matches!(r, AccountResult::NotWritten(_))),
            count(|r| matches!(r, AccountResult::Failed(_))),
            count(|r| matches!(r, AccountResult::Superseded | AccountResult::Unknown)),
            count(|r| *r == AccountResult::Saving),
        )];
        for ((name, result), entry) in results.iter().zip(&self.entries) {
            out.push(match result {
                AccountResult::Created => format!("{name}: created"),
                AccountResult::Overwritten => format!("{name}: overwritten"),
                AccountResult::NotWritten(status) => format!("{name}: {}", status.label()),
                AccountResult::Failed(error) => format!("{name}: failed: {error}"),
                AccountResult::Superseded => format!(
                    "{name}: not confirmed: a later save of this profile was written in place of the import's; check the profile"
                ),
                AccountResult::Unknown => format!(
                    "{name}: not confirmed: what became of the import's write is no longer known; check the profile"
                ),
                AccountResult::Saving => format!("{name}: saving…"),
            });
            if result.is_durable() || matches!(result, AccountResult::NotWritten(_)) {
                out.extend(entry.details.iter().cloned());
            }
        }
        if !self.skipped.is_empty() {
            out.push(String::new());
            out.push(format!("Not imported ({} items):", self.skipped.len()));
            for (owner, skip) in &self.skipped {
                out.push(format!("{owner}: {}: {}", skip.item, skip.reason));
            }
        }
        out.push(String::new());
        if self.settled(core) && !self.succeeded(core) {
            out.push("Not every account was confirmed saved, so the export file is not offered for deletion: keep it until they are.".into());
        }
        out.push("Loaded bots use an imported password and world at their next login, and imported script parameters at their next Start.".into());
        out.into_iter().map(shown).collect()
    }
}

#[cfg(feature = "js")]
/// Delete the export file once the import succeeded (explicit confirmation
/// in the front end). Logs one line.
fn delete_export_file(path: &Path) -> Result<(), ImportError> {
    std::fs::remove_file(path).map_err(|source| ImportFailure::Delete {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "{LABEL}: deleted the export file {}",
        path.display()
    );
    Ok(())
}

// ---- the shared front-end flow ---------------------------------------------

#[cfg(feature = "js")]
/// Where the import flow is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for a file path.
    Path,
    /// A preview is shown; nothing is written until Confirm.
    Preview,
    /// Confirmed: the writes are being saved, then the report shows.
    Report,
}

#[cfg(feature = "js")]
/// The import flow the panel window and the TUI overlay render: a file path,
/// then a preview with the existing-profile choice, then Confirm, then the
/// report with the offer to delete the export file.
pub struct ProfileImport {
    path: String,
    error: Option<String>,
    file: Option<(PathBuf, ExportFile)>,
    choices: ImportChoices,
    plan: Option<ImportPlan>,
    run: Option<(PathBuf, ImportRun)>,
    writes: Vec<ImportWrite>,
    staged_account: Option<usize>,
    delete_asked: bool,
    deleted: Option<PathBuf>,
}

#[cfg(feature = "js")]
impl Default for ProfileImport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "js")]
impl ProfileImport {
    pub fn new() -> Self {
        Self {
            path: String::new(),
            error: None,
            file: None,
            choices: ImportChoices::default(),
            plan: None,
            run: None,
            writes: Vec::new(),
            staged_account: None,
            delete_asked: false,
            deleted: None,
        }
    }

    pub fn stage(&self) -> Stage {
        if self.run.is_some() {
            Stage::Report
        } else if self.plan.is_some() {
            Stage::Preview
        } else {
            Stage::Path
        }
    }

    /// The file path being typed.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn path_mut(&mut self) -> &mut String {
        &mut self.path
    }

    /// The last refusal (unreadable file, wrong kind, locked vault, …).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Keep `message` as the refusal, redacted like every shown line.
    fn refuse(&mut self, message: String) {
        self.error = Some(shown(message));
    }

    pub fn policy(&self) -> ConflictPolicy {
        self.choices.policy
    }

    /// Whether accounts the 289 client cannot log in with are imported too.
    pub fn include_unusable(&self) -> bool {
        self.choices.include_unusable
    }

    pub fn plan(&self) -> Option<&ImportPlan> {
        self.plan.as_ref()
    }

    /// Read and map the file at the typed path. Writes nothing.
    pub(crate) fn preview<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        scripts: &Scripts,
    ) -> Result<(), ImportError> {
        self.error = None;
        self.plan = None;
        self.file = None;
        self.run = None;
        self.writes.clear();
        self.staged_account = None;
        self.deleted = None;
        self.delete_asked = false;
        let path = PathBuf::from(self.path.trim());
        if path.as_os_str().is_empty() {
            let error: ImportError = ImportFailure::EmptyPath.into();
            self.refuse(error.to_string());
            return Err(error);
        }
        let Some(vault) = core.vault() else {
            let error: ImportError = ImportFailure::VaultLocked.into();
            self.refuse(error.to_string());
            return Err(error);
        };
        match read_profile_file(&path) {
            Ok(file) => {
                self.plan = Some(plan_import(
                    &file,
                    vault,
                    core.fresh_uid(),
                    scripts.library(),
                    self.choices,
                ));
                self.file = Some((path, file));
                Ok(())
            }
            Err(error) => {
                self.refuse(error.to_string());
                Err(error)
            }
        }
    }

    /// Choose what happens to existing profiles; the preview follows.
    pub(crate) fn set_policy<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        scripts: &Scripts,
        policy: ConflictPolicy,
    ) {
        self.choices.policy = policy;
        self.replan(core, scripts);
    }

    /// Choose whether accounts the 289 client cannot log in with are
    /// imported anyway; the preview follows.
    pub(crate) fn set_include_unusable<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        scripts: &Scripts,
        include: bool,
    ) {
        self.choices.include_unusable = include;
        self.replan(core, scripts);
    }

    fn replan<Io>(&mut self, core: &OperatorSession<Io>, scripts: &Scripts) {
        if let (Some((_, file)), Some(vault)) = (self.file.as_ref(), core.vault()) {
            self.plan = Some(plan_import(
                file,
                vault,
                core.fresh_uid(),
                scripts.library(),
                self.choices,
            ));
        }
    }

    /// The preview text.
    pub fn preview_lines(&self) -> Vec<String> {
        self.plan
            .as_ref()
            .map(ImportPlan::lines)
            .unwrap_or_default()
    }

    /// Replan against the current vault and stage the report and account
    /// writes. The front end submits one CreateProfile or SaveProfile row
    /// for each item from [`Self::planned_writes`].
    pub fn prepare_confirmation<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        scripts: &Scripts,
    ) -> Result<(), ImportError> {
        if self.run.is_some() || self.staged_account.is_some() {
            let error: ImportError = ImportFailure::NoPreview.into();
            self.refuse(error.to_string());
            return Err(error);
        }
        let Some(vault) = core.vault() else {
            let error: ImportError = ImportFailure::VaultLocked.into();
            self.refuse(error.to_string());
            return Err(error);
        };
        let Some((path, file)) = self.file.take() else {
            let error: ImportError = ImportFailure::NoPreview.into();
            self.refuse(error.to_string());
            return Err(error);
        };
        let plan = plan_import(
            &file,
            vault,
            core.fresh_uid(),
            scripts.library(),
            self.choices,
        );
        drop(file);

        self.start_run(path, plan);
        Ok(())
    }
    fn start_run(&mut self, path: PathBuf, plan: ImportPlan) {
        let mut entries = Vec::with_capacity(plan.accounts.len());
        let mut skipped: Vec<(String, Skipped)> = Vec::new();
        let mut writes = Vec::new();
        for (index, account) in plan.accounts.into_iter().enumerate() {
            let details = account.detail_lines();
            let username = account.username;
            let status = account.status;
            let profile = account.row;
            for skip in account.skipped {
                skipped.push((username.clone(), skip));
            }
            let entry = RunEntry {
                username: username.clone(),
                status,
                op: None,
                refused: None,
                details,
            };
            if let Some(profile) = profile {
                let summary = change_summary(account.old.as_ref(), &profile);
                let command = if status == AccountStatus::New {
                    OperatorCommand::CreateProfile
                } else {
                    OperatorCommand::SaveProfile
                };
                writes.push(ImportWrite {
                    index,
                    username,
                    command,
                    profile: Some(profile),
                    summary,
                });
            }
            entries.push(entry);
        }
        skipped.extend(
            plan.file_skipped
                .into_iter()
                .map(|skip| ("file".to_owned(), skip)),
        );
        self.plan = None;
        self.writes = writes;
        self.staged_account = None;
        self.run = Some((
            path,
            ImportRun {
                seen: (0..entries.len()).map(|_| OnceCell::new()).collect(),
                entries,
                skipped,
            },
        ));
    }

    /// Planned account writes in file order: account index, profile name,
    /// and the matching writer command.
    pub fn planned_writes(&self) -> impl Iterator<Item = (usize, &str, OperatorCommand)> + '_ {
        self.writes
            .iter()
            .filter(|write| write.profile.is_some())
            .map(|write| (write.index, write.username.as_str(), write.command))
    }

    /// Stage a planned new account for `CreateProfile`.
    pub fn stage_create_account(&mut self, index: usize) -> Result<(), ImportError> {
        self.stage_account_for(index, OperatorCommand::CreateProfile)
    }

    /// Stage a planned existing account for `SaveProfile`.
    pub fn stage_save_account(&mut self, index: usize) -> Result<(), ImportError> {
        self.stage_account_for(index, OperatorCommand::SaveProfile)
    }

    fn stage_account_for(
        &mut self,
        index: usize,
        command: OperatorCommand,
    ) -> Result<(), ImportError> {
        if self.staged_account.is_some() {
            return Err(ImportFailure::AccountAlreadyStaged.into());
        }
        let Some(expected) = self
            .writes
            .iter()
            .find(|write| write.index == index && write.profile.is_some())
            .map(|write| write.command)
        else {
            return Err(ImportFailure::AccountNotPending { index }.into());
        };
        if expected != command {
            return Err(ImportFailure::WrongWriter {
                index,
                expected,
                actual: command,
            }
            .into());
        }
        self.staged_account = Some(index);
        Ok(())
    }

    /// Clear a staged input after its submit did not dispatch it.
    pub fn cancel_staged_account(&mut self, index: usize) {
        if self.staged_account == Some(index) {
            self.staged_account = None;
            self.record_write_result(
                index,
                Err(&ImportFailure::AccountNotSubmitted { index }.to_string()),
            );
        }
    }

    /// Take the staged imported account for its writer row.
    pub(crate) fn take_staged_account(
        &mut self,
        command: OperatorCommand,
    ) -> Result<Option<StagedImportAccount>, ImportError> {
        let Some(index) = self.staged_account.take() else {
            return Ok(None);
        };
        let Some(write) = self.writes.iter_mut().find(|write| write.index == index) else {
            return Err(ImportFailure::AccountNotPending { index }.into());
        };
        if write.command != command {
            let error = ImportFailure::WrongWriter {
                index,
                expected: write.command,
                actual: command,
            };
            self.record_write_result(index, Err(&error.to_string()));
            return Err(error.into());
        }
        let Some(profile) = write.profile.take() else {
            return Err(ImportFailure::AccountNotPending { index }.into());
        };
        Ok(Some(StagedImportAccount {
            index,
            username: write.username.clone(),
            write: StagedProfileWrite::new(
                profile,
                None,
                command == OperatorCommand::CreateProfile,
                ArmMirror::remember(None),
                LABEL,
            ),
        }))
    }

    /// Save this writer's result in the import report.
    pub(crate) fn record_write_result(&mut self, index: usize, result: Result<OperationId, &str>) {
        let Some((_, run)) = self.run.as_mut() else {
            return;
        };
        let Some(entry) = run.entries.get_mut(index) else {
            return;
        };
        match result {
            Ok(op) => {
                entry.op = Some(op);
                if let Some(write) = self.writes.iter().find(|write| write.index == index) {
                    let action = if write.command == OperatorCommand::CreateProfile {
                        "create"
                    } else {
                        "overwrite"
                    };
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        slot = &write.username,
                        "{LABEL}: {action} profile {} op#{}: {}",
                        write.username,
                        op.0,
                        write.summary
                    );
                }
            }
            Err(reason) => entry.refused = Some(shown(reason.to_owned())),
        }
    }

    /// Back to the path field, dropping the parsed file and its passwords.
    pub(crate) fn back(&mut self) {
        self.file = None;
        self.plan = None;
        self.error = None;
    }

    /// The report text (while writes settle and after), every line redacted.
    pub fn report_lines<Io>(&self, core: &OperatorSession<Io>) -> Vec<String> {
        let Some((path, run)) = self.run.as_ref() else {
            return Vec::new();
        };
        let mut out = run.lines(core);
        match &self.deleted {
            Some(deleted) => out.push(shown(format!("Deleted {}.", deleted.display()))),
            None => {
                out.push(shown(PLAINTEXT_WARNING.into()));
                if run.succeeded(core) && path.exists() {
                    out.push(shown(format!("You can delete {} now.", path.display())));
                }
            }
        }
        out
    }

    /// Whether a confirmed import's writes have all settled.
    pub fn settled<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.run.as_ref().is_some_and(|(_, run)| run.settled(core))
    }

    /// Whether a confirmed import's writes are still being saved.
    pub fn writing<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.run.as_ref().is_some_and(|(_, run)| !run.settled(core))
    }

    /// Whether the flow holds nothing: no run, no parsed file (so no
    /// password) and no refusal. A front end resets a flow it stopped
    /// showing.
    pub fn is_fresh(&self) -> bool {
        self.run.is_none() && self.file.is_none() && self.error.is_none()
    }

    /// Whether the report may offer to delete the export file: every
    /// account the import chose to write is confirmed durable with the
    /// import's own data, and the file is still there.
    pub fn can_delete<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.deleted.is_none()
            && self
                .run
                .as_ref()
                .is_some_and(|(path, run)| run.succeeded(core) && path.is_file())
    }

    /// Ask to delete the export file; [`Self::confirm_delete`] deletes it.
    pub(crate) fn ask_delete(&mut self) {
        self.delete_asked = true;
    }

    pub fn delete_asked(&self) -> bool {
        self.delete_asked
    }

    pub(crate) fn cancel_delete(&mut self) {
        self.delete_asked = false;
    }

    /// The export file the report offers to delete.
    pub fn export_path(&self) -> Option<&Path> {
        self.run.as_ref().map(|(path, _)| path.as_path())
    }

    /// Delete the export file after the operator confirmed.
    pub(crate) fn confirm_delete<Io>(
        &mut self,
        core: &OperatorSession<Io>,
    ) -> Result<(), ImportError> {
        if !std::mem::take(&mut self.delete_asked) {
            let error: ImportError = ImportFailure::DeleteNotConfirmed.into();
            self.refuse(error.to_string());
            return Err(error);
        }
        if !self.can_delete(core) {
            let error: ImportError = ImportFailure::DeleteUnavailable.into();
            self.refuse(error.to_string());
            return Err(error);
        }
        let Some((path, _)) = self.run.as_ref() else {
            let error: ImportError = ImportFailure::DeleteUnavailable.into();
            self.refuse(error.to_string());
            return Err(error);
        };
        if let Err(error) = delete_export_file(path) {
            self.refuse(error.to_string());
            return Err(error);
        }
        self.deleted = Some(path.clone());
        Ok(())
    }
}

#[cfg(test)]
#[path = "rs2b0t_import_tests.rs"]
mod tests;
