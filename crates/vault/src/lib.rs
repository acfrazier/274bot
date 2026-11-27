//! Encrypted credential vault.
//!
//! A vault file is sealed with AES-256-GCM under a key derived from the
//! passphrase via PBKDF2-HMAC-SHA256. The file stores only the KDF salt,
//! the round count, the nonce and the ciphertext; passphrases and passwords
//! never appear in plaintext. A wrong passphrase fails the unlock without
//! modifying the file.
//!
//! # Formats
//!
//! - Format 1 (legacy, 0.1.x to 0.2.9): the ciphertext holds every profile
//!   with its `uid` and [`ProfileSettings`]. This build only reads it, for
//!   the first conversion at unlock ([`authenticate`]).
//! - Format 2 (credential-only): the ciphertext holds one exact `username`
//!   and `password` per account ([`Credential`]). The header carries the
//!   canonical server profile and the configured vault component
//!   ([`VaultBinding`]) as non-secret metadata, authenticated as GCM
//!   associated data, so a file sealed for another server or component
//!   never authenticates as this one. The 0.2.9 decoder refuses this
//!   version byte before it derives a key (`unsupported format version`).
//!
//! The `uid` and every setting of a profile live in the state database;
//! `host-play` joins the two stores into the in-memory [`Vault`].
//!
//! # Passphrase policy
//!
//! A **new** vault needs a passphrase that is not empty after surrounding
//! whitespace is trimmed ([`check_new_passphrase`]). Passphrase strength is
//! the user's choice. An **existing** vault accepts any passphrase that
//! decrypts the file, so vaults created under an earlier policy keep opening,
//! saving, and retaining their data. The KDF is untouched by an upgrade: the
//! round count stored in the file header is kept on every save, and a header
//! outside `1..=MAX_PBKDF2_ROUNDS` is rejected before any key derivation runs.
//!
//! Secrets are held in [`Secret`] (zeroed on drop) and the derived key and the
//! serialized profiles in `zeroize` buffers, so a copy does not outlive its
//! use in freed memory. That is best effort: see [`Secret`].

mod private_file;
mod secret;

pub use private_file::{
    check_private_path, create_private_dir, create_private_file, read_private_file,
    read_regular_file, valid_component, write_private_file, write_private_file_checked,
    PrivateKind,
};
pub use secret::Secret;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use pbkdf2::pbkdf2_hmac;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

mod compiled_settings;
pub use compiled_settings::CompiledSettingsRecord;

const MAGIC: &[u8; 8] = b"274VAULT";
/// The 0.1.x–0.2.9 format: every profile with its settings.
const LEGACY_FORMAT_VERSION: u8 = 1;
/// The credential-only format with its authenticated binding.
const SEALED_FORMAT_VERSION: u8 = 2;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// PBKDF2 iterations used when creating a new vault. Unlock reads the round
/// count from the file header and every save keeps it, so a file written at a
/// different count never breaks.
const PBKDF2_ROUNDS: u32 = 100_000;
/// Highest round count a vault file may declare. Unlock derives the key before
/// it can tell a wrong passphrase from a crafted file, so this bounds how long
/// a hostile file can stall it: at most twenty times the default work.
const MAX_PBKDF2_ROUNDS: u32 = 2_000_000;
const _: () = assert!(PBKDF2_ROUNDS <= MAX_PBKDF2_ROUNDS);
/// Magic, version, rounds, salt and nonce: every format's fixed header.
const HEADER_LEN: usize = MAGIC.len() + 1 + 4 + SALT_LEN + NONCE_LEN;

/// Per-profile settings. Low-memory is the default for headless clients;
/// auto-login defaults off so v1 blobs (which only carried `lowmem`)
/// deserialize with the box unchecked.
/// How this slot paints the scene. Off is `set_draw` only. Gpu↔Cpu on a
/// live slot reattaches the `Renderer` without a logout. Memory mode is a
/// separate setting applied at the next login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RasterMode {
    Off,
    #[default]
    Gpu,
    Cpu,
}

/// Last successfully started script for a profile. Identity is source kind
/// plus canonical catalog/file/compiled id — never a display-name fallback.
/// A missing source stays present with [`ScriptAssignment::unavailable`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScriptAssignment {
    /// `catalog`, `file`, `builtin`, or `compiled`.
    pub source_kind: String,
    /// Canonical identity: register name, stored file path, or compiled id.
    pub identity: String,
    /// Picker label only. Never used to look up or substitute a card.
    #[serde(default)]
    pub display_name: String,
    /// Present when the stored identity cannot currently be resolved.
    #[serde(default)]
    pub unavailable: Option<String>,
}

impl ScriptAssignment {
    pub fn key(&self) -> String {
        assignment_key(&self.source_kind, &self.identity)
    }
}

/// Stable persistence key for a script identity.
pub fn assignment_key(source_kind: &str, identity: &str) -> String {
    format!("{source_kind}:{identity}")
}

/// Per-event response policy. `Flee` is the default and the only supported
/// response; a stored `Fight` value is saved as `Fight` and currently treated
/// as `Flee` by the Guardian.
///
/// A 0.2.9 file's `"ignore"` value reads as `Flee` through the
/// deserialize-only alias below; serialization writes only `flee` and
/// `fight`, so the next ordinary save drops the legacy spelling. Remove the
/// alias when the preceding release's format no longer needs reading.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RandomEventPolicy {
    #[default]
    #[serde(alias = "ignore")]
    Flee,
    Fight,
}

/// When a script drinks a held antipoison. The stored form of the
/// profile's Cure poison setting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurePoison {
    /// Drink when the poison's worst case would take the bot below its
    /// eating line or kill it.
    #[default]
    WhenNeeded,
    /// Drink as soon as the game reports the poison.
    Always,
    /// Never drink.
    Never,
}

/// The profile's Survival and supplies group. Phase 1 stores one field;
/// later phases extend the group without a migration (`#[serde(default)]`
/// loads a row that predates each new field).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct SurvivalSettings {
    /// When a script drinks a held antipoison.
    pub cure_poison: CurePoison,
}

/// Stable event keys and their profile-settings labels, shared by the panel
/// and TUI. Keys are lowercase NPC names so profile files remain readable.
pub const RANDOM_EVENT_POLICY_EVENTS: [(&str, &str); 8] = [
    ("river troll", "River troll"),
    ("swarm", "Swarm"),
    ("rock golem", "Rock golem"),
    ("zombie", "Zombie"),
    ("shade", "Shade"),
    ("watchman", "Watchman"),
    ("tree spirit", "Tree spirit"),
    ("strange plant", "Strange plant"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub lowmem: bool,
    #[serde(default)]
    pub auto_login: bool,
    /// Public world number, or automatic selection when absent.
    #[serde(default)]
    pub world: Option<u16>,
    /// Cached TutSkip. `None` = never read (`getvar tutorial` pending).
    /// `Some(true)` = skipped (`>= 1000` or TutSkip pressed). `Some(false)`
    /// = engine reported the tutorial still open.
    #[serde(default)]
    pub tutorial_skipped: Option<bool>,
    #[serde(default)]
    pub raster: RasterMode,
    /// Incoming random events (sandwich lady, genie, …). On by default so
    /// pre-0.1.2 vaults keep them flowing.
    #[serde(default = "default_random_events")]
    pub random_events: bool,
    /// Per-hostile-random NPC response; absent entries default to Flee.
    #[serde(default)]
    pub random_event_policies: BTreeMap<String, RandomEventPolicy>,
    /// Lamp skill set from the lamp dialogue.
    #[serde(default = "default_lamp_skill")]
    pub lamp_skill: String,
    /// Lamp auto-use: claim the reward without a confirmation click.
    #[serde(default = "default_lamp_auto")]
    pub lamp_auto: bool,
    /// Account-wide partner used by clue 3554's Duel Arena traversal.
    #[serde(default)]
    pub clue_duel_partner: String,
    /// Last successfully started assignment. Absence is a legacy unassigned profile.
    #[serde(default)]
    pub script_assignment: Option<ScriptAssignment>,
    /// Per-profile parameter overrides keyed by [`assignment_key`].
    #[serde(default)]
    pub script_settings: BTreeMap<String, Map<String, Value>>,
    /// The Survival and supplies group. Absent in pre-0.2.6 vault rows,
    /// which load with the default (cure when needed).
    #[serde(default)]
    pub survival: SurvivalSettings,
}

fn default_random_events() -> bool {
    true
}

fn default_lamp_skill() -> String {
    "strength".into()
}

fn default_lamp_auto() -> bool {
    true
}

impl Default for ProfileSettings {
    fn default() -> Self {
        Self {
            lowmem: true,
            world: None,
            auto_login: false,
            tutorial_skipped: None,
            raster: RasterMode::Gpu,
            random_events: true,
            random_event_policies: BTreeMap::new(),
            lamp_skill: "strength".into(),
            lamp_auto: true,
            clue_duel_partner: String::new(),
            script_assignment: None,
            script_settings: BTreeMap::new(),
            survival: SurvivalSettings::default(),
        }
    }
}

impl ProfileSettings {
    pub fn random_event_policy(&self, event: &str) -> RandomEventPolicy {
        self.random_event_policies
            .get(&event.trim().to_ascii_lowercase())
            .copied()
            .unwrap_or_default()
    }

    pub fn set_random_event_policy(&mut self, event: &str, policy: RandomEventPolicy) {
        self.random_event_policies
            .insert(event.trim().to_ascii_lowercase(), policy);
    }
}

/// A stored login profile, keyed by username. The password is a [`Secret`]:
/// zeroed when the profile is dropped and never printed by `Debug`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub username: String,
    pub password: Secret,
    pub uid: i32,
    pub settings: ProfileSettings,
}

/// The password never appears in `{:?}` output, so a profile formatted into
/// a log line or a panic message cannot leak it.
impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("uid", &self.uid)
            .field("settings", &self.settings)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("passphrase must not be empty")]
    EmptyPassphrase,
    #[error("vault already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("no vault at {0}")]
    NotFound(PathBuf),
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("invalid compiled settings envelope")]
    InvalidCompiledSettingsEnvelope,
    #[error("invalid compiled settings schema version")]
    InvalidCompiledSettingsSchemaVersion,
    #[error("compiled settings values must be an object")]
    CompiledSettingsValuesNotObject,
    /// The vault is in a format older than the previous release's. 0.2.9
    /// reads every vault shape 0.2.8 read, so no 0.2.9 reader returns this.
    /// It stays for the one move-aside path that the next release uses when
    /// it stops reading a 0.2.8 vault shape.
    #[error("{path}: saved data is in a format from before 0.2.8", path = .path.display())]
    UnsupportedSavedFormat { path: PathBuf },
    /// A server profile or vault component that cannot bind a vault.
    #[error("invalid vault binding: {0}")]
    InvalidBinding(&'static str),
    /// The file at the path no longer holds the image a checked write or
    /// removal expected; nothing was written or removed.
    #[error("{path}: the vault file changed; nothing was written", path = .0.display())]
    Changed(PathBuf),
    /// A checked replacement renamed the new image into place, but syncing
    /// its directory failed: the new image may or may not survive a crash.
    /// The caller keeps its pending record and settles it against whichever
    /// image the file holds at the next unlock.
    #[error("{path}: the vault was replaced, but the replacement is not yet durable: {source}", path = .path.display())]
    Unsynced {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("corrupt vault file: {0}")]
    Corrupt(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// The longest canonical server profile name a binding carries.
const MAX_BINDING_SERVER: usize = 64;
/// The longest vault component a binding carries (one header length byte).
const MAX_BINDING_COMPONENT: usize = 255;

/// The non-secret metadata a credential-only vault authenticates: the
/// canonical (lowercase, alias-resolved) server profile and the configured
/// vault component (lowercase file name). A file sealed under another
/// binding fails the comparison even when it holds the same usernames.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultBinding {
    server: String,
    component: String,
}

impl VaultBinding {
    /// Bind `server` and `component`, both compared and stored lowercase.
    pub fn new(server: &str, component: &str) -> Result<Self, VaultError> {
        let server = server.to_ascii_lowercase();
        let component = component.to_ascii_lowercase();
        if server.len() > MAX_BINDING_SERVER || !valid_component(&server) {
            return Err(VaultError::InvalidBinding("server profile"));
        }
        if component.len() > MAX_BINDING_COMPONENT || !valid_component(&component) {
            return Err(VaultError::InvalidBinding("vault component"));
        }
        Ok(Self { server, component })
    }

    /// The canonical server profile.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The configured vault component.
    pub fn component(&self) -> &str {
        &self.component
    }

    fn header_bytes(&self, out: &mut Vec<u8>) {
        out.push(self.server.len() as u8);
        out.extend_from_slice(self.server.as_bytes());
        out.push(self.component.len() as u8);
        out.extend_from_slice(self.component.as_bytes());
    }
}

/// One account's credential: its exact stored username and its password.
/// The username matches the account's profile row in the state database.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub username: String,
    pub password: Secret,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// The credentials of `profiles`: exact usernames and passwords only.
pub fn credentials_of(profiles: &BTreeMap<String, Profile>) -> BTreeMap<String, Credential> {
    profiles
        .iter()
        .map(|(name, profile)| {
            (
                name.clone(),
                Credential {
                    username: profile.username.clone(),
                    password: profile.password.clone(),
                },
            )
        })
        .collect()
}

/// The SHA-256 of a whole vault file image.
pub type FileDigest = [u8; 32];

/// The digest of `bytes`.
pub fn file_digest(bytes: &[u8]) -> FileDigest {
    Sha256::digest(bytes).into()
}

/// The derived key of one vault and the KDF inputs it was derived with.
/// Every replacement of that vault keeps the salt and round count, so a
/// file converted from an old KDF keeps opening with the same passphrase.
#[derive(Clone)]
pub struct VaultKey {
    salt: [u8; SALT_LEN],
    key: Zeroizing<[u8; KEY_LEN]>,
    rounds: u32,
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultKey")
            .field("rounds", &self.rounds)
            .finish_non_exhaustive()
    }
}

impl VaultKey {
    /// A key for a **new** vault: a fresh salt and the current round count.
    /// The passphrase must satisfy [`check_new_passphrase`].
    pub fn create(passphrase: &str) -> Result<Self, VaultError> {
        check_new_passphrase(passphrase)?;
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        Ok(Self {
            salt,
            key: derive_key(passphrase, &salt, PBKDF2_ROUNDS),
            rounds: PBKDF2_ROUNDS,
        })
    }
}

/// What an authenticated vault file holds.
pub enum Opened {
    /// A format-1 file: every profile with its `uid` and settings, keyed by
    /// the exact username the old writer stored.
    Legacy(BTreeMap<String, Profile>),
    /// A credential-only file and the binding it authenticated.
    Sealed {
        binding: VaultBinding,
        credentials: BTreeMap<String, Credential>,
    },
}

/// Upper bound on the vault file read into memory. A real vault is tens of
/// KiB; this only stops a crafted or wrong file from being read whole.
pub const MAX_VAULT_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Read the whole vault file at `path` and its digest. A missing file is
/// [`VaultError::NotFound`]; a file above [`MAX_VAULT_FILE_BYTES`] is
/// corrupt.
pub fn read_file(path: &Path) -> Result<(Vec<u8>, FileDigest), VaultError> {
    let bytes = read_vault_file(path)?;
    let digest = file_digest(&bytes);
    Ok((bytes, digest))
}

/// The digest of the file at `path`, or `None` when nothing is there.
pub fn current_digest(path: &Path) -> Result<Option<FileDigest>, VaultError> {
    match read_vault_file(path) {
        Ok(bytes) => Ok(Some(file_digest(&bytes))),
        Err(VaultError::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Authenticate a vault file image with `passphrase`. Any non-empty
/// passphrase that decrypts the file is accepted, including one of only
/// whitespace: releases through 0.1.9.1 created vaults under that rule, and
/// the new-vault trim check must never lock them out.
pub fn authenticate(bytes: &[u8], passphrase: &str) -> Result<(VaultKey, Opened), VaultError> {
    if passphrase.is_empty() {
        return Err(VaultError::EmptyPassphrase);
    }
    let parsed = parse(bytes)?;
    let key = VaultKey {
        salt: parsed.salt,
        key: derive_key(passphrase, &parsed.salt, parsed.rounds),
        rounds: parsed.rounds,
    };
    let opened = open_parsed(&parsed, &key)?;
    Ok((key, opened))
}

/// Authenticate a vault file image with an already derived `key`: the
/// check of a replacement just published. A file under another salt or
/// round count is a wrong key.
pub fn authenticate_with_key(bytes: &[u8], key: &VaultKey) -> Result<Opened, VaultError> {
    let parsed = parse(bytes)?;
    if parsed.salt != key.salt || parsed.rounds != key.rounds {
        return Err(VaultError::WrongPassphrase);
    }
    open_parsed(&parsed, key)
}

fn open_parsed(parsed: &Parsed<'_>, key: &VaultKey) -> Result<Opened, VaultError> {
    let plaintext = Zeroizing::new(
        decrypt(&key.key, parsed.nonce, parsed.ciphertext, parsed.aad)
            .map_err(|_| VaultError::WrongPassphrase)?,
    );
    match &parsed.binding {
        None => {
            let profiles: BTreeMap<String, Profile> = serde_json::from_slice(&plaintext)
                .map_err(|error| serde_corruption("deserialize profiles", error))?;
            Ok(Opened::Legacy(profiles))
        }
        Some(binding) => {
            let credentials: BTreeMap<String, Credential> = serde_json::from_slice(&plaintext)
                .map_err(|error| serde_corruption("deserialize credentials", error))?;
            if credentials
                .iter()
                .any(|(name, credential)| *name != credential.username)
            {
                return Err(VaultError::Corrupt(
                    "a credential is stored under another username".into(),
                ));
            }
            Ok(Opened::Sealed {
                binding: binding.clone(),
                credentials,
            })
        }
    }
}

/// Seal `credentials` under `key` and `binding` as a credential-only image.
/// The caller publishes it ([`replace_checked`], [`create_new`]).
pub fn seal(
    key: &VaultKey,
    binding: &VaultBinding,
    credentials: &BTreeMap<String, Credential>,
) -> Result<Vec<u8>, VaultError> {
    let mut out = SecretBuf(Zeroizing::new(Vec::with_capacity(4096)));
    serde_json::to_writer(&mut out, credentials)
        .map_err(|error| serde_corruption("serialize credentials", error))?;
    build_blob(key, Some(binding), &out.0)
}

/// Publish `blob` at `path` by private same-directory replacement, only
/// while the file there still has the digest `expected`, then sync the
/// file's directory. A different file is [`VaultError::Changed`] and is
/// left in place. Every other error before the rename published nothing;
/// a failed directory sync after it is [`VaultError::Unsynced`].
pub fn replace_checked(path: &Path, blob: &[u8], expected: &FileDigest) -> Result<(), VaultError> {
    let mut changed = false;
    let result = write_private_file_checked(path, blob, || match current_digest(path) {
        Ok(Some(found)) if found == *expected => Ok(()),
        Ok(_) => {
            changed = true;
            Err(std::io::Error::other("the vault file changed"))
        }
        Err(VaultError::Io(error)) => Err(error),
        Err(error) => Err(std::io::Error::other(error.to_string())),
    });
    match result {
        Ok(()) => sync_published(path),
        Err(_) if changed => Err(VaultError::Changed(path.to_path_buf())),
        Err(error) => Err(VaultError::Io(error)),
    }
}

/// Sync the directory of a replacement that is already in place.
fn sync_published(path: &Path) -> Result<(), VaultError> {
    #[cfg(any(test, feature = "test-support"))]
    if test_hooks::FAIL_NEXT_SYNC.with(|fail| fail.replace(false)) {
        return Err(VaultError::Unsynced {
            path: path.to_path_buf(),
            source: std::io::Error::other("injected directory sync failure"),
        });
    }
    sync_parent(path).map_err(|error| VaultError::Unsynced {
        path: path.to_path_buf(),
        source: match error {
            VaultError::Io(source) => source,
            other => std::io::Error::other(other.to_string()),
        },
    })
}

/// Fault injection for the two-store recovery tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_hooks {
    use std::cell::Cell;

    thread_local! {
        pub(crate) static FAIL_NEXT_SYNC: Cell<bool> = const { Cell::new(false) };
    }

    /// Make this thread's next checked replacement fail its directory sync
    /// after the new image is in place.
    pub fn fail_next_replace_sync() {
        FAIL_NEXT_SYNC.with(|fail| fail.set(true));
    }
}

/// Create `path` with `blob` (refused when anything is there) and sync its
/// directory.
pub fn create_new(path: &Path, blob: &[u8]) -> Result<(), VaultError> {
    create_private_file(path, blob).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => VaultError::AlreadyExists(path.to_path_buf()),
        _ => VaultError::Io(e),
    })?;
    sync_parent(path)
}

/// Delete the vault file at `path` only while it has the digest `expected`
/// (forgotten-passphrase recovery confirmed that file), then sync its
/// directory. A different file is [`VaultError::Changed`] and is kept.
pub fn remove_checked(path: &Path, expected: &FileDigest) -> Result<(), VaultError> {
    match current_digest(path)? {
        Some(found) if found == *expected => {}
        Some(_) => return Err(VaultError::Changed(path.to_path_buf())),
        None => return Err(VaultError::NotFound(path.to_path_buf())),
    }
    std::fs::remove_file(path)?;
    sync_parent(path)
}

/// Make a rename or removal in `path`'s directory durable.
fn sync_parent(path: &Path) -> Result<(), VaultError> {
    #[cfg(unix)]
    {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The joined in-memory profiles of one unlocked vault: each account's
/// credential with its `uid` and settings from the state database. The
/// AES-256 key lives in RAM (zeroized on drop) until the vault is dropped.
/// Durable writes go through the owner's profile store; staging here only
/// changes the in-memory view.
pub struct Vault {
    path: PathBuf,
    key: VaultKey,
    /// `None` only for a test fixture vault in the legacy format.
    binding: Option<VaultBinding>,
    profiles: BTreeMap<String, Profile>,
    /// A test fixture's writer for a joined vault (both stores).
    #[cfg(any(test, feature = "test-support"))]
    fixture_writer: Option<FixtureWriter>,
}

impl Vault {
    /// A joined vault at `path`: its key, its authenticated binding and the
    /// profiles both stores agree on.
    pub fn joined(
        path: PathBuf,
        key: VaultKey,
        binding: VaultBinding,
        profiles: BTreeMap<String, Profile>,
    ) -> Self {
        Self {
            path,
            key,
            binding: Some(binding),
            profiles,
            #[cfg(any(test, feature = "test-support"))]
            fixture_writer: None,
        }
    }

    /// The vault file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The authenticated binding; `None` for a legacy test fixture.
    pub fn binding(&self) -> Option<&VaultBinding> {
        self.binding.as_ref()
    }

    /// Looks up a profile by username.
    pub fn get(&self, username: &str) -> Option<&Profile> {
        self.profiles.get(username)
    }

    /// All stored profiles (sorted by username; the map is a `BTreeMap`).
    pub fn profiles(&self) -> impl Iterator<Item = &Profile> {
        self.profiles.values()
    }

    /// A detached durable copy of this vault (same file, key and binding)
    /// for a writer that persists off the caller's thread. Once one exists,
    /// every write must go through it: [`Vault::stage_upsert`] /
    /// [`Vault::stage_remove`] change only this in-memory view.
    pub fn store(&self) -> VaultStore {
        VaultStore {
            path: self.path.clone(),
            key: self.key.clone(),
            binding: self.binding.clone(),
            profiles: self.profiles.clone(),
        }
    }

    /// Replace a profile in memory only; its durable write is the store's.
    pub fn stage_upsert(&mut self, profile: Profile) {
        self.profiles.insert(profile.username.clone(), profile);
    }

    /// Remove a profile in memory only. Returns whether it existed.
    pub fn stage_remove(&mut self, username: &str) -> bool {
        self.profiles.remove(username).is_some()
    }

    /// Put back the durable value of one profile after its write failed.
    pub fn restore(&mut self, username: &str, durable: Option<Profile>) {
        match durable {
            Some(profile) => {
                self.profiles.insert(username.to_string(), profile);
            }
            None => {
                self.profiles.remove(username);
            }
        }
    }
}

/// Legacy-format fixtures. Tests build the vaults a 0.2.9 root holds with
/// the old format's own writer; product code only reads that format.
#[cfg(any(test, feature = "test-support"))]
impl Vault {
    /// Creates a new empty legacy-format vault at `path`. Fails if the file
    /// already exists or the passphrase is refused.
    pub fn create(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        let key = VaultKey::create(passphrase)?;
        if path.exists() {
            return Err(VaultError::AlreadyExists(path.to_path_buf()));
        }
        let blob = legacy_blob(&key, &BTreeMap::new())?;
        create_private_file(path, &blob).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => VaultError::AlreadyExists(path.to_path_buf()),
            _ => VaultError::Io(e),
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            key,
            binding: None,
            profiles: BTreeMap::new(),
            fixture_writer: None,
        })
    }

    /// Opens the legacy-format vault at `path`. A credential-only vault is
    /// refused here: it only opens joined with its state database.
    pub fn unlock(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        let (bytes, _) = read_file(path)?;
        match authenticate(&bytes, passphrase)? {
            (key, Opened::Legacy(profiles)) => Ok(Self {
                path: path.to_path_buf(),
                key,
                binding: None,
                profiles,
                fixture_writer: None,
            }),
            (_, Opened::Sealed { .. }) => Err(VaultError::Corrupt(
                "a credential-only vault opens only joined with its state root".into(),
            )),
        }
    }

    /// Give a joined vault the writer its fixture edits go through (the
    /// owner's profile store over both stores).
    pub fn set_fixture_writer(&mut self, writer: FixtureWriter) {
        self.fixture_writer = Some(writer);
    }

    /// Inserts or replaces a profile and writes it: a legacy fixture
    /// rewrites its file, a joined vault goes through its fixture writer.
    /// On error the vault is unchanged both on disk and in memory.
    pub fn upsert(&mut self, profile: Profile) -> Result<(), VaultError> {
        let mut next = self.profiles.clone();
        next.insert(profile.username.clone(), profile.clone());
        self.persist_fixture(&next, &[VaultChange::Upsert(profile)])?;
        self.profiles = next;
        Ok(())
    }

    /// Removes a profile by username and writes it like [`Self::upsert`].
    /// Returns false when no such profile exists.
    pub fn remove(&mut self, username: &str) -> Result<bool, VaultError> {
        let mut next = self.profiles.clone();
        if next.remove(username).is_none() {
            return Ok(false);
        }
        self.persist_fixture(&next, &[VaultChange::Remove(username.to_string())])?;
        self.profiles = next;
        Ok(true)
    }

    fn persist_fixture(
        &self,
        profiles: &BTreeMap<String, Profile>,
        changes: &[VaultChange],
    ) -> Result<(), VaultError> {
        if self.binding.is_some() {
            let writer = self.fixture_writer.as_ref().ok_or_else(|| {
                VaultError::Corrupt("a joined vault writes through its profile store".into())
            })?;
            return writer(self.store(), changes).map_err(VaultError::Corrupt);
        }
        write_private_file(&self.path, &legacy_blob(&self.key, profiles)?).map_err(VaultError::Io)
    }
}

/// A test fixture's write of changes to both stores of a joined vault,
/// given the vault's durable copy.
#[cfg(any(test, feature = "test-support"))]
pub type FixtureWriter =
    std::sync::Arc<dyn Fn(VaultStore, &[VaultChange]) -> Result<(), String> + Send + Sync>;

// Change batches are short-lived and small; boxing `Upsert` would add an
// allocation to every profile save only to shrink this transient enum.
#[expect(
    clippy::large_enum_variant,
    reason = "Upsert is transient and small; boxing it would allocate on every profile save"
)]
/// One profile change for a profile store commit.
#[derive(Debug, Clone, PartialEq)]
pub enum VaultChange {
    Upsert(Profile),
    Remove(String),
}

impl VaultChange {
    pub fn username(&self) -> &str {
        match self {
            Self::Upsert(profile) => &profile.username,
            Self::Remove(username) => username,
        }
    }
}

/// The durable side of a [`Vault`]: the joined profiles last made durable
/// plus the key and binding to write more. Owned by one writer; the key is
/// zeroized on drop.
pub struct VaultStore {
    path: PathBuf,
    key: VaultKey,
    binding: Option<VaultBinding>,
    profiles: BTreeMap<String, Profile>,
}

impl VaultStore {
    /// The durable value of one profile.
    pub fn get(&self, username: &str) -> Option<&Profile> {
        self.profiles.get(username)
    }

    /// Every durable profile, keyed by exact username.
    pub fn profiles(&self) -> &BTreeMap<String, Profile> {
        &self.profiles
    }

    /// The vault file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The vault key.
    pub fn key(&self) -> &VaultKey {
        &self.key
    }

    /// The authenticated binding; `None` for a legacy test fixture.
    pub fn binding(&self) -> Option<&VaultBinding> {
        self.binding.as_ref()
    }

    /// Record `profiles` as durable after the owner's commit of both stores.
    pub fn adopt(&mut self, profiles: BTreeMap<String, Profile>) {
        self.profiles = profiles;
    }

    /// `changes` applied in order to the durable profiles, in memory only.
    pub fn applied(&self, changes: &[VaultChange]) -> BTreeMap<String, Profile> {
        let mut next = self.profiles.clone();
        for change in changes {
            match change {
                VaultChange::Upsert(profile) => {
                    next.insert(profile.username.clone(), profile.clone());
                }
                VaultChange::Remove(username) => {
                    next.remove(username);
                }
            }
        }
        next
    }

    /// Apply `changes` to a legacy test fixture and rewrite its file once.
    /// On error the store is unchanged both on disk and in memory.
    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_legacy(&mut self, changes: &[VaultChange]) -> Result<(), VaultError> {
        if self.binding.is_some() {
            return Err(VaultError::Corrupt(
                "a joined vault writes through its profile store".into(),
            ));
        }
        let next = self.applied(changes);
        write_private_file(&self.path, &legacy_blob(&self.key, &next)?).map_err(VaultError::Io)?;
        self.profiles = next;
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
fn legacy_blob(
    key: &VaultKey,
    profiles: &BTreeMap<String, Profile>,
) -> Result<Vec<u8>, VaultError> {
    let data = serialize_profiles(profiles)?;
    build_blob(key, None, &data)
}

/// The policy for a passphrase that will protect a **new** vault: it must not
/// be empty after surrounding whitespace is trimmed. Passphrase strength is
/// the user's choice. [`VaultKey::create`] applies it; a caller that prompts
/// can apply it first and ask again instead of failing late. [`authenticate`]
/// does not apply it (see there).
pub fn check_new_passphrase(passphrase: &str) -> Result<(), VaultError> {
    if passphrase.trim().is_empty() {
        Err(VaultError::EmptyPassphrase)
    } else {
        Ok(())
    }
}

fn read_vault_file(path: &Path) -> Result<Vec<u8>, VaultError> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => VaultError::NotFound(path.to_path_buf()),
        _ => VaultError::Io(e),
    })?;
    let mut blob = Vec::new();
    file.take(MAX_VAULT_FILE_BYTES + 1).read_to_end(&mut blob)?;
    if blob.len() as u64 > MAX_VAULT_FILE_BYTES {
        return Err(VaultError::Corrupt(format!(
            "file is larger than {MAX_VAULT_FILE_BYTES} bytes"
        )));
    }
    Ok(blob)
}

/// Serializes the profiles, every password in the clear, into a buffer that is
/// zeroed when dropped.
#[cfg(any(test, feature = "test-support"))]
fn serialize_profiles(
    profiles: &BTreeMap<String, Profile>,
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let mut out = SecretBuf(Zeroizing::new(Vec::with_capacity(4096)));
    serde_json::to_writer(&mut out, profiles)
        .map_err(|error| serde_corruption("serialize profiles", error))?;
    Ok(out.0)
}

fn serde_corruption(context: &'static str, error: serde_json::Error) -> VaultError {
    VaultError::Corrupt(format!(
        "{context}: {:?} at line {} column {}",
        error.classify(),
        error.line(),
        error.column()
    ))
}

/// A `Vec` writer that grows by hand: `Vec`'s own doubling would free the old
/// allocation, plaintext and all, without zeroing it.
struct SecretBuf(Zeroizing<Vec<u8>>);

impl Write for SecretBuf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.0.capacity() - self.0.len() < data.len() {
            let want = (self.0.len() + data.len()).max(self.0.capacity() * 2);
            let mut bigger = Zeroizing::new(Vec::with_capacity(want));
            bigger.extend_from_slice(&self.0);
            // The old buffer is zeroed as it is dropped here.
            self.0 = bigger;
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn derive_key(passphrase: &str, salt: &[u8; SALT_LEN], rounds: u32) -> Zeroizing<[u8; KEY_LEN]> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, rounds, key.as_mut());
    key
}

/// Header bytes (magic, version, rounds, salt, nonce and, in format 2, the
/// binding) followed by ciphertext || 16-byte GCM tag. Format 2
/// authenticates the whole header as associated data.
fn build_blob(
    key: &VaultKey,
    binding: Option<&VaultBinding>,
    plaintext: &[u8],
) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.key.as_ref()));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut blob = Vec::with_capacity(HEADER_LEN + 2 * 256 + plaintext.len() + TAG_LEN);
    blob.extend_from_slice(MAGIC);
    blob.push(if binding.is_some() {
        SEALED_FORMAT_VERSION
    } else {
        LEGACY_FORMAT_VERSION
    });
    blob.extend_from_slice(&key.rounds.to_le_bytes());
    blob.extend_from_slice(&key.salt);
    blob.extend_from_slice(&nonce);
    if let Some(binding) = binding {
        binding.header_bytes(&mut blob);
    }
    let aad: &[u8] = if binding.is_some() { &blob } else { &[] };
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| VaultError::Corrupt("aes-gcm encrypt failed".into()))?;
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// One parsed vault file image.
struct Parsed<'a> {
    salt: [u8; SALT_LEN],
    rounds: u32,
    nonce: &'a [u8],
    /// The binding of a format-2 file.
    binding: Option<VaultBinding>,
    /// The authenticated header of a format-2 file; empty for format 1.
    aad: &'a [u8],
    ciphertext: &'a [u8],
}

fn parse(blob: &[u8]) -> Result<Parsed<'_>, VaultError> {
    if blob.len() < HEADER_LEN {
        return Err(VaultError::Corrupt("file too short".into()));
    }
    if &blob[..MAGIC.len()] != MAGIC {
        return Err(VaultError::Corrupt("bad magic".into()));
    }
    let version = blob[MAGIC.len()];
    if version != LEGACY_FORMAT_VERSION && version != SEALED_FORMAT_VERSION {
        return Err(VaultError::Corrupt("unsupported format version".into()));
    }
    let rounds_bytes: [u8; 4] = blob[MAGIC.len() + 1..MAGIC.len() + 5]
        .try_into()
        .map_err(|_| VaultError::Corrupt("short rounds field".into()))?;
    let rounds = u32::from_le_bytes(rounds_bytes);
    if rounds == 0 || rounds > MAX_PBKDF2_ROUNDS {
        return Err(VaultError::Corrupt("invalid pbkdf2 rounds".into()));
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&blob[MAGIC.len() + 5..HEADER_LEN - NONCE_LEN]);
    let nonce = &blob[HEADER_LEN - NONCE_LEN..HEADER_LEN];
    let (binding, header_end) = if version == SEALED_FORMAT_VERSION {
        let mut at = HEADER_LEN;
        let mut field = |what: &'static str| -> Result<&str, VaultError> {
            let len = *blob
                .get(at)
                .ok_or_else(|| VaultError::Corrupt(format!("short {what} field")))?
                as usize;
            let bytes = blob
                .get(at + 1..at + 1 + len)
                .ok_or_else(|| VaultError::Corrupt(format!("short {what} field")))?;
            at += 1 + len;
            std::str::from_utf8(bytes).map_err(|_| VaultError::Corrupt(format!("invalid {what}")))
        };
        let server = field("server")?;
        let component = field("component")?;
        let binding = VaultBinding::new(server, component)
            .ok()
            .filter(|binding| binding.server == server && binding.component == component)
            .ok_or_else(|| VaultError::Corrupt("invalid binding".into()))?;
        (Some(binding), at)
    } else {
        (None, HEADER_LEN)
    };
    if blob.len() < header_end + TAG_LEN {
        return Err(VaultError::Corrupt("ciphertext too short".into()));
    }
    Ok(Parsed {
        salt,
        rounds,
        nonce,
        aad: if binding.is_some() {
            &blob[..header_end]
        } else {
            &[]
        },
        binding,
        ciphertext: &blob[header_end..],
    })
}

fn decrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, aes_gcm::Error> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher.decrypt(
        Nonce::from_slice(nonce),
        Payload {
            msg: ciphertext,
            aad,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{
        build_blob, derive_key, parse, serialize_profiles, Profile, ProfileSettings,
        RandomEventPolicy, Vault, VaultChange, VaultError, VaultKey, MAX_PBKDF2_ROUNDS,
        MAX_VAULT_FILE_BYTES, SALT_LEN,
    };

    /// A key derived as `authenticate` derives it, for hand-built images.
    fn key_of(passphrase: &str, salt: [u8; SALT_LEN], rounds: u32) -> VaultKey {
        VaultKey {
            salt,
            key: derive_key(passphrase, &salt, rounds),
            rounds,
        }
    }

    /// A passphrase used by the vaults these tests create.
    const PASS: &str = "test-passphrase-01";

    /// A real vault written by the code that shipped before this policy (0.1.9:
    /// passphrase `bot`, 100,000 rounds, two synthetic profiles). It is that
    /// writer's own output, not something this build produced.
    const LEGACY_VAULT: &[u8] = include_bytes!("../tests/fixtures/legacy-0.1.9.vault");

    /// A real vault written by 0.2.9.1's writer (tag `0.2.9.1`,
    /// `a245c8355b911463f614d821f980bb355d64fc5e`; `Vault::create` and
    /// `Vault::upsert` as tagged): one profile with three `ignore` entries,
    /// one `fight`, one `flee`, and no entry for Tree spirit or Strange
    /// plant. Passphrase `v11c-legacy-fixture`.
    const LEGACY_0_2_9_VAULT: &[u8] =
        include_bytes!("../tests/fixtures/legacy-0.2.9-policies.vault");
    const LEGACY_0_2_9_PASS: &str = "v11c-legacy-fixture";

    /// The profiles a vault file holds, decrypted for a test that checks the
    /// stored spellings.
    fn decrypted_profiles(file: &[u8], passphrase: &str) -> serde_json::Value {
        let parsed = parse(file).unwrap();
        let key = derive_key(passphrase, &parsed.salt, parsed.rounds);
        let plaintext = super::decrypt(&key, parsed.nonce, parsed.ciphertext, parsed.aad).unwrap();
        serde_json::from_slice(&plaintext).unwrap()
    }

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("274bot-vault-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p).unwrap();
        }
        p
    }

    fn profile(username: &str, password: &str) -> Profile {
        Profile {
            username: username.into(),
            password: password.into(),
            uid: 42,
            settings: ProfileSettings {
                lowmem: false,
                ..ProfileSettings::default()
            },
        }
    }

    /// The error `Vault::create` refuses with; a created vault is a test failure.
    fn create_err(path: &std::path::Path, passphrase: &str) -> VaultError {
        match Vault::create(path, passphrase) {
            Err(e) => e,
            Ok(_) => panic!("created a vault with passphrase {passphrase:?}"),
        }
    }

    fn header_rounds(file: &[u8]) -> u32 {
        u32::from_le_bytes(file[9..13].try_into().unwrap())
    }

    #[test]
    fn profile_debug_never_prints_the_password() {
        let text = format!("{:?}", profile("alice", "hunter22"));
        assert!(!text.contains("hunter22"), "{text}");
        assert!(text.contains("alice"));
    }

    #[test]
    fn auto_login_defaults_false_and_old_json_unlocks_off() {
        assert!(!ProfileSettings::default().auto_login);
        let path = tmp_path("old-settings.vault");
        let mut v = Vault::create(&path, PASS).unwrap();
        v.upsert(Profile {
            username: "a".into(),
            password: "a".into(),
            uid: 1,
            settings: ProfileSettings {
                lowmem: true,
                ..ProfileSettings::default()
            },
        })
        .unwrap();
        drop(v);
        // Simulate a v1 blob that only had lowmem: rewrite profiles JSON via unlock+file is
        // enough if Deserialize default works. Also assert missing field:
        let missing: ProfileSettings = serde_json::from_str(r#"{"lowmem":true}"#).unwrap();
        assert!(missing.lowmem);
        assert!(!missing.auto_login);
        assert_eq!(missing.tutorial_skipped, None);
        assert_eq!(missing.raster, super::RasterMode::Gpu);
        assert_eq!(missing.world, None);
    }

    #[test]
    fn pre_0_1_2_profile_defaults_random_events_and_lamp_on() {
        // A pre-0.1.2 JSON profile carries none of the new keys; randoms and
        // the lamp helper must stay on so old vaults behave as before.
        let missing: ProfileSettings = serde_json::from_str(r#"{"lowmem":true}"#).unwrap();
        assert!(missing.random_events, "old vaults keep random events on");
        assert_eq!(missing.lamp_skill, "strength");
        assert!(missing.lamp_auto, "old vaults keep lamp auto on");
        let d = ProfileSettings::default();
        assert!(d.random_events);
        assert_eq!(d.lamp_skill, "strength");
        assert!(d.lamp_auto);
    }

    #[test]
    fn hostile_random_event_policy_defaults_to_flee_and_roundtrips() {
        let missing: ProfileSettings = serde_json::from_str(r#"{"lowmem":true}"#).unwrap();
        assert_eq!(
            missing.random_event_policy("River Troll"),
            super::RandomEventPolicy::Flee
        );

        let mut settings = ProfileSettings::default();
        assert!(
            !super::RANDOM_EVENT_POLICY_EVENTS
                .iter()
                .any(|(event, _)| *event == "drunken dwarf"),
            "the Talk-to dwarf is not a hostile policy choice"
        );
        settings.set_random_event_policy(" River Troll ", super::RandomEventPolicy::Fight);
        assert_eq!(
            settings.random_event_policies.get("river troll"),
            Some(&super::RandomEventPolicy::Fight)
        );
        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: ProfileSettings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded.random_event_policy("river troll"),
            super::RandomEventPolicy::Fight
        );
        assert_eq!(
            decoded.random_event_policy("Swarm"),
            super::RandomEventPolicy::Flee
        );

        // 0.2.9 wrote `ignore`; it reads as Flee and the next encode writes
        // `flee`, not the legacy spelling.
        let legacy: ProfileSettings = serde_json::from_str(
            r#"{"lowmem":true,"random_event_policies":{"river troll":"ignore","swarm":"fight"}}"#,
        )
        .unwrap();
        assert_eq!(
            legacy.random_event_policy("river troll"),
            RandomEventPolicy::Flee
        );
        assert_eq!(
            legacy.random_event_policy("swarm"),
            RandomEventPolicy::Fight
        );
        let canonical = serde_json::to_string(&legacy).unwrap();
        assert!(canonical.contains(r#""river troll":"flee""#), "{canonical}");
        assert!(!canonical.contains("ignore"), "{canonical}");

        // An unknown value is a typed read error, never a silent reset.
        let unknown: Result<ProfileSettings, _> =
            serde_json::from_str(r#"{"lowmem":true,"random_event_policies":{"shade":"taunt"}}"#);
        assert!(unknown.is_err(), "an unknown policy is rejected");
    }

    #[test]
    fn legacy_0_2_9_ignore_reads_as_flee_and_the_next_save_writes_canonical() {
        let path = tmp_path("legacy-0.2.9-policies.vault");
        std::fs::write(&path, LEGACY_0_2_9_VAULT).unwrap();

        let mut vault = Vault::unlock(&path, LEGACY_0_2_9_PASS).unwrap();
        let settings = &vault.get("alice").unwrap().settings;
        for event in ["river troll", "rock golem", "watchman"] {
            assert_eq!(
                settings.random_event_policy(event),
                RandomEventPolicy::Flee,
                "{event}: a 0.2.9 ignore reads as Flee"
            );
        }
        assert_eq!(
            settings.random_event_policy("swarm"),
            RandomEventPolicy::Fight,
            "a saved Fight stays Fight"
        );
        assert_eq!(
            settings.random_event_policy("zombie"),
            RandomEventPolicy::Flee
        );
        assert_eq!(
            settings.random_event_policy("tree spirit"),
            RandomEventPolicy::Flee,
            "a missing entry keeps the default"
        );
        assert_eq!(
            settings.random_event_policy("strange plant"),
            RandomEventPolicy::Flee,
            "a missing entry keeps the default"
        );
        assert_eq!(
            settings.random_event_policies.len(),
            5,
            "every stored key survives the conversion"
        );

        // The next ordinary save (an unchanged profile) rewrites the file in
        // the current format.
        let profile = vault.get("alice").unwrap().clone();
        vault.upsert(profile).unwrap();

        let reopened = Vault::unlock(&path, LEGACY_0_2_9_PASS).unwrap();
        let saved = &reopened.get("alice").unwrap().settings;
        assert_eq!(
            saved.random_event_policy("river troll"),
            RandomEventPolicy::Flee
        );
        assert_eq!(saved.random_event_policy("swarm"), RandomEventPolicy::Fight);
        assert_eq!(saved.random_event_policies.len(), 5);

        let raw = decrypted_profiles(&std::fs::read(&path).unwrap(), LEGACY_0_2_9_PASS);
        let stored = raw["alice"]["settings"]["random_event_policies"]
            .as_object()
            .expect("the policy map");
        assert_eq!(
            stored.len(),
            5,
            "each map value is converted on its own key"
        );
        assert_eq!(stored["river troll"], "flee");
        assert_eq!(stored["rock golem"], "flee");
        assert_eq!(stored["watchman"], "flee");
        assert_eq!(stored["swarm"], "fight");
        assert_eq!(stored["zombie"], "flee");
        assert!(
            !raw.to_string().contains("ignore"),
            "the save wrote no legacy spelling: {raw}"
        );
    }

    #[test]
    fn unknown_policy_is_rejected_while_ignore_still_reads_as_flee() {
        // The legacy spelling is the one accepted non-current value.
        let accepted: ProfileSettings = serde_json::from_str(
            r#"{"lowmem":false,"random_event_policies":{"river troll":"ignore"}}"#,
        )
        .unwrap();
        assert_eq!(
            accepted.random_event_policy("river troll"),
            RandomEventPolicy::Flee,
            "the deserialize-only alias reads ignore as Flee"
        );

        let path = tmp_path("unknown-policy.vault");
        let salt = [0x5au8; SALT_LEN];
        let key = key_of(PASS, salt, super::PBKDF2_ROUNDS);
        let plaintext = serde_json::json!({
            "alice": {
                "username": "alice",
                "password": "pw",
                "uid": 1,
                "settings": {
                    "lowmem": false,
                    "random_event_policies": {"river troll": "taunt"}
                }
            }
        })
        .to_string();
        let blob = build_blob(&key, None, plaintext.as_bytes()).unwrap();
        std::fs::write(&path, &blob).unwrap();

        match Vault::unlock(&path, PASS) {
            Err(VaultError::Corrupt(reason)) => {
                assert!(
                    reason.contains("deserialize profiles"),
                    "a typed read error from the profile decode: {reason}"
                );
                assert!(
                    reason.contains("line 1 column"),
                    "the reader reports where decoding failed: {reason}"
                );
            }
            other => panic!(
                "an unknown policy opened: {:?}",
                other.err().map(|error| error.to_string())
            ),
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            blob,
            "the reader leaves an unsupported file untouched"
        );
    }

    #[test]
    fn profile_json_round_trip_keeps_survival_group() {
        // A vault row written by a newer binary carries the survival
        // group. Loading and saving it must keep the group in the JSON
        // text: `ProfileSettings` has no `deny_unknown_fields`, so a
        // build without the field silently drops the key on this round
        // trip.
        let row: ProfileSettings =
            serde_json::from_str(r#"{"lowmem":true,"survival":{"cure_poison":"never"}}"#).unwrap();
        let saved = serde_json::to_string(&row).unwrap();
        assert!(
            saved.contains(r#""survival":{"cure_poison":"never"}"#),
            "the saved row keeps the survival group: {saved}"
        );
    }

    #[test]
    fn profile_settings_survival_defaults_from_legacy_json() {
        // A pre-0.2.6 row has no survival key. It loads with the default:
        // cure when needed.
        let missing: ProfileSettings = serde_json::from_str(r#"{"lowmem":true}"#).unwrap();
        assert_eq!(
            missing.survival.cure_poison,
            super::CurePoison::WhenNeeded,
            "old vault rows cure when needed"
        );
        assert_eq!(
            ProfileSettings::default().survival.cure_poison,
            super::CurePoison::WhenNeeded
        );
        // A row with an empty survival object also takes the defaults.
        let empty: ProfileSettings =
            serde_json::from_str(r#"{"lowmem":true,"survival":{}}"#).unwrap();
        assert_eq!(empty.survival, super::SurvivalSettings::default());
    }

    #[test]
    fn profile_settings_round_trip_unchanged() {
        // Regression pin: encoding and decoding a settings row returns
        // the same row, field for field.
        let settings = ProfileSettings {
            lowmem: false,
            auto_login: true,
            world: Some(2),
            tutorial_skipped: Some(true),
            random_events: false,
            lamp_skill: "magic".into(),
            lamp_auto: false,
            clue_duel_partner: "partner".into(),
            ..ProfileSettings::default()
        };
        let raw = serde_json::to_string(&settings).unwrap();
        let back: ProfileSettings = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn create_unlock_roundtrip() {
        let path = tmp_path("roundtrip.vault");

        let mut v = Vault::create(&path, PASS).unwrap();
        v.upsert(profile("zezima", "hunter2")).unwrap();
        drop(v);

        let v = Vault::unlock(&path, PASS).unwrap();
        let p = v.get("zezima").expect("profile present after unlock");
        assert_eq!(p.password, "hunter2");
        assert_eq!(p.uid, 42);
        assert!(!p.settings.lowmem);
        assert!(v.get("nobody").is_none());
    }
    #[test]
    fn selected_world_survives_encrypted_vault_roundtrip() {
        let path = tmp_path("selected-world.vault");
        let mut v = Vault::create(&path, PASS).unwrap();
        let mut p = profile("alice", "secret");
        p.settings.world = Some(2);
        v.upsert(p).unwrap();
        drop(v);
        assert_eq!(
            Vault::unlock(&path, PASS)
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .world,
            Some(2)
        );
    }

    #[test]
    fn empty_or_whitespace_passphrase_rejected() {
        let path = tmp_path("empty.vault");

        for passphrase in ["", "   ", "\t\n"] {
            assert!(matches!(
                create_err(&path, passphrase),
                VaultError::EmptyPassphrase
            ));
            assert!(!path.exists(), "nothing written for {passphrase:?}");
        }

        Vault::create(&path, PASS).unwrap();
        assert!(matches!(
            Vault::unlock(&path, ""),
            Err(VaultError::EmptyPassphrase)
        ));
        for passphrase in ["   ", "\t\n"] {
            assert!(matches!(
                Vault::unlock(&path, passphrase),
                Err(VaultError::WrongPassphrase)
            ));
        }
    }

    /// Through 0.1.9.1 only an empty passphrase was refused, so a vault may
    /// be sealed under whitespace alone. It must keep opening.
    #[test]
    fn a_vault_sealed_under_whitespace_still_unlocks() {
        let path = tmp_path("whitespace.vault");
        let _ = std::fs::remove_file(&path);
        let passphrase = "   ";
        let key = key_of(passphrase, [7u8; SALT_LEN], super::PBKDF2_ROUNDS);
        let data = super::serialize_profiles(&std::collections::BTreeMap::new()).unwrap();
        let blob = super::build_blob(&key, None, &data).unwrap();
        std::fs::write(&path, blob).unwrap();

        Vault::unlock(&path, passphrase).unwrap();
        assert!(matches!(
            Vault::unlock(&path, "x"),
            Err(VaultError::WrongPassphrase)
        ));
    }

    #[test]
    fn short_passphrase_creates_and_unlocks() {
        let path = tmp_path("short.vault");
        Vault::create(&path, "x").unwrap();
        Vault::unlock(&path, "x").unwrap();
    }

    #[test]
    fn a_legacy_vault_opens_saves_and_keeps_its_data() {
        let path = tmp_path("legacy.vault");
        std::fs::write(&path, LEGACY_VAULT).unwrap();

        let mut v = Vault::unlock(&path, "bot").expect("the legacy passphrase still opens");
        let alice = v.get("alice").unwrap().clone();
        assert_eq!(alice.password, "alice-password-legacy");
        assert_eq!(alice.uid, 274_000_001);
        assert!(!alice.settings.lowmem && alice.settings.auto_login);
        assert_eq!(alice.settings.world, Some(2));
        assert_eq!(alice.settings.tutorial_skipped, Some(true));
        assert_eq!(alice.settings.raster, super::RasterMode::Cpu);
        assert!(!alice.settings.random_events && !alice.settings.lamp_auto);
        assert_eq!(alice.settings.lamp_skill, "magic");
        assert_eq!(alice.settings.clue_duel_partner, "bob");
        let assignment = alice.settings.script_assignment.as_ref().unwrap();
        assert_eq!(
            (
                assignment.source_kind.as_str(),
                assignment.identity.as_str()
            ),
            ("catalog", "ChickenKiller")
        );
        assert_eq!(
            alice.settings.script_settings["catalog:ChickenKiller"]["eatAtPercent"],
            47
        );
        assert_eq!(v.get("bob").unwrap().password, "bob-password-legacy");

        // Saving keeps working and keeps the header the file was written with.
        v.upsert(profile("carol", "carol-password")).unwrap();
        assert!(v.remove("bob").unwrap());
        drop(v);
        let v = Vault::unlock(&path, "bot").unwrap();
        assert_eq!(v.get("alice"), Some(&alice), "existing data is intact");
        assert!(v.get("bob").is_none());
        assert_eq!(v.get("carol").unwrap().password, "carol-password");
        let saved = std::fs::read(&path).unwrap();
        assert_eq!(&saved[..9], &LEGACY_VAULT[..9], "magic and format version");
        assert_eq!(
            header_rounds(&saved),
            100_000,
            "the KDF is not silently changed"
        );
    }

    #[test]
    fn wrong_passphrase_fails_without_wipe() {
        let path = tmp_path("wrong.vault");

        let mut v = Vault::create(&path, "correct-horse").unwrap();
        v.upsert(profile("alice", "s3cret")).unwrap();
        drop(v);

        assert!(matches!(
            Vault::unlock(&path, "wrong"),
            Err(VaultError::WrongPassphrase)
        ));
        // File survives a failed unlock and still opens with the right passphrase.
        assert!(path.exists());
        let v = Vault::unlock(&path, "correct-horse").unwrap();
        assert_eq!(v.get("alice").unwrap().password, "s3cret");
    }

    #[test]
    fn deserialize_errors_do_not_echo_decrypted_values() {
        const SECRET: &str = "hunter2-secret-value";
        let path = tmp_path("deserialize-secret.vault");
        let salt = [0x5au8; SALT_LEN];
        let key = key_of(PASS, salt, super::PBKDF2_ROUNDS);
        let plaintext = serde_json::json!({
            "alice": {
                "username": "alice",
                "password": "safe-password",
                "uid": SECRET,
                "settings": {"lowmem": false}
            }
        })
        .to_string();
        let blob = build_blob(&key, None, plaintext.as_bytes()).unwrap();
        std::fs::write(&path, blob).unwrap();

        let error = match Vault::unlock(&path, PASS) {
            Err(error) => error,
            Ok(_) => panic!("malformed profiles unexpectedly opened"),
        };
        let display = error.to_string();
        let debug = format!("{error:?}");

        assert!(
            !display.contains(SECRET),
            "Display leaked the decrypted value: {display}"
        );
        assert!(
            !debug.contains(SECRET),
            "Debug leaked the decrypted value: {debug}"
        );
        assert!(
            display.contains("Data"),
            "missing error category: {display}"
        );
        assert!(
            display.contains("line 1 column"),
            "missing error location: {display}"
        );
        assert!(debug.contains("Data"), "missing error category: {debug}");
        assert!(
            debug.contains("line 1 column"),
            "missing error location: {debug}"
        );
    }

    #[test]
    fn truncated_ciphertext_is_corrupt_not_a_wrong_passphrase() {
        let path = tmp_path("truncated-ciphertext.vault");
        Vault::create(&path, PASS).unwrap();
        let blob = std::fs::read(&path).unwrap();

        for length in super::HEADER_LEN..super::HEADER_LEN + 16 {
            let truncated = &blob[..length];
            std::fs::write(&path, truncated).unwrap();

            assert!(
                matches!(
                    Vault::unlock(&path, PASS),
                    Err(VaultError::Corrupt(message)) if message == "ciphertext too short"
                ),
                "length {length}"
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                truncated,
                "length {length}: failed unlock must not alter the file"
            );
        }
    }

    #[test]
    fn ciphertext_has_no_plaintext_password() {
        let path = tmp_path("plaintext.vault");

        let mut v = Vault::create(&path, PASS).unwrap();
        v.upsert(profile("zezima", "hunter2isasecret")).unwrap();
        drop(v);

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.windows(5).any(|w| w == b"hunte"),
            "profile password bytes leaked into the ciphertext file"
        );
        assert!(
            !bytes.windows(PASS.len()).any(|w| w == PASS.as_bytes()),
            "vault passphrase bytes leaked into the ciphertext file"
        );
    }

    #[test]
    fn create_refuses_existing_vault() {
        let path = tmp_path("exists.vault");

        Vault::create(&path, PASS).unwrap();
        assert!(matches!(
            create_err(&path, PASS),
            VaultError::AlreadyExists(_)
        ));
    }

    /// Creators that all passed an `exists()` check would each publish a vault
    /// and each be told it succeeded; exactly one may.
    #[test]
    fn of_concurrent_creators_exactly_one_vault_is_made_and_it_opens_with_its_passphrase() {
        let path = tmp_path("create-race.vault");
        let start = std::sync::Arc::new(std::sync::Barrier::new(4));
        // Spawn every creator before joining any: the barrier needs all four
        // racers waiting at once, so a fused spawn-join chain would deadlock.
        let mut handles = Vec::with_capacity(4);
        for i in 0..4 {
            let (path, start) = (path.clone(), start.clone());
            handles.push(std::thread::spawn(move || {
                let passphrase = format!("passphrase-number-{i}");
                start.wait();
                Vault::create(&path, &passphrase).map(|_| passphrase)
            }));
        }
        let outcomes: Vec<_> = handles.into_iter().map(|c| c.join().unwrap()).collect();

        let winners: Vec<&String> = outcomes.iter().filter_map(|o| o.as_ref().ok()).collect();
        assert_eq!(winners.len(), 1, "exactly one creator may succeed");
        for lost in outcomes.iter().filter_map(|o| o.as_ref().err()) {
            assert!(matches!(lost, VaultError::AlreadyExists(_)), "{lost:?}");
        }
        Vault::unlock(&path, winners[0]).unwrap();
    }

    #[test]
    fn unlock_directory_is_io_not_not_found() {
        let dir = tmp_path("vault-as-dir");
        std::fs::create_dir_all(&dir).unwrap();
        match Vault::unlock(&dir, PASS) {
            Err(VaultError::Io(_)) => {}
            Err(e) => panic!("expected Io, got {e}"),
            Ok(_) => panic!("expected Io, unlocked a directory"),
        }
        assert!(dir.is_dir(), "must not replace the path with a vault file");
    }

    #[test]
    fn remove_checked_deletes_only_the_confirmed_file() {
        let path = tmp_path("reset.vault");
        Vault::create(&path, PASS).unwrap();
        let confirmed = super::current_digest(&path).unwrap().unwrap();
        let other = super::file_digest(b"another image");
        assert!(matches!(
            super::remove_checked(&path, &other),
            Err(VaultError::Changed(_))
        ));
        assert!(path.is_file(), "a different file is kept");
        super::remove_checked(&path, &confirmed).unwrap();
        assert!(!path.exists());
        assert!(matches!(
            super::remove_checked(&path, &confirmed),
            Err(VaultError::NotFound(_))
        ));
        Vault::create(&path, "another-passphrase").unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn upsert_failure_leaves_state_unchanged() {
        let dir = std::env::temp_dir()
            .join(format!("274bot-vault-test-{}", std::process::id()))
            .join("rollback.d");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("vault");

        let mut v = Vault::create(&file, PASS).unwrap();
        v.upsert(profile("alice", "pw1")).unwrap();

        // Make the target a directory so the next publish fails on every
        // platform (Windows cannot delete the open vault's directory, and
        // permission bits do not stop it writing).
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(v.upsert(profile("bob", "pw2")).is_err());
        assert!(
            v.get("bob").is_none(),
            "failed upsert must not change in-memory state"
        );
        assert_eq!(v.get("alice").unwrap().password, "pw1");
    }

    /// An install made under `umask 002` (or by an older release) leaves the
    /// bot root group-writable. The first save tightens it and goes through,
    /// instead of refusing until the user runs chmod by hand.
    #[cfg(unix)]
    #[test]
    fn an_existing_vault_in_a_group_writable_directory_saves_after_tightening_it() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir()
            .join(format!("274bot-vault-test-{}", std::process::id()))
            .join("umask-002");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("vault");
        let mut v = Vault::create(&file, PASS).unwrap();
        v.upsert(profile("alice", "pw1")).unwrap();
        drop(v);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o775)).unwrap();

        let mut v = Vault::unlock(&file, PASS).unwrap();
        v.upsert(profile("bob", "pw2")).unwrap();

        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o755, "the first save removes group/other write");
        let reopened = Vault::unlock(&file, PASS).unwrap();
        assert_eq!(reopened.get("alice").unwrap().password, "pw1");
        assert_eq!(reopened.get("bob").unwrap().password, "pw2");
    }

    /// A header for `rounds` without deriving a key for it: only the header's
    /// round field matters to these tests.
    fn blob_with_rounds(rounds: u32) -> Vec<u8> {
        let key = VaultKey {
            salt: [1u8; SALT_LEN],
            key: zeroize::Zeroizing::new([9u8; 32]),
            rounds,
        };
        build_blob(&key, None, b"{}").unwrap()
    }

    #[test]
    fn stored_round_counts_are_accepted_only_inside_the_cap() {
        for accepted in [1, 100_000, MAX_PBKDF2_ROUNDS] {
            assert!(
                parse(&blob_with_rounds(accepted)).is_ok(),
                "{accepted} rounds"
            );
        }
        for rejected in [0, MAX_PBKDF2_ROUNDS + 1, u32::MAX] {
            match parse(&blob_with_rounds(rejected)).map(|parsed| parsed.rounds) {
                Err(VaultError::Corrupt(message)) => {
                    assert_eq!(message, "invalid pbkdf2 rounds")
                }
                other => panic!("{rejected} rounds: expected Corrupt, got {other:?}"),
            }
        }
    }

    #[test]
    fn unlock_refuses_a_crafted_round_count_before_deriving_any_key() {
        let path = tmp_path("rounds.vault");
        Vault::create(&path, PASS).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();

        for rounds in [MAX_PBKDF2_ROUNDS + 1, u32::MAX] {
            bytes[9..13].copy_from_slice(&rounds.to_le_bytes());
            std::fs::write(&path, &bytes).unwrap();
            // Deriving first would cost minutes at these counts.
            let started = Instant::now();
            assert!(matches!(
                Vault::unlock(&path, PASS),
                Err(VaultError::Corrupt(_))
            ));
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{rounds} rounds"
            );
        }
    }

    #[test]
    fn unlock_bounds_the_vault_file_size_at_the_boundary() {
        let path = tmp_path("huge.vault");
        let file = std::fs::File::create(&path).unwrap();

        // Exactly at the bound the file is read, then refused as not a vault.
        file.set_len(MAX_VAULT_FILE_BYTES).unwrap();
        match Vault::unlock(&path, PASS) {
            Err(VaultError::Corrupt(message)) => assert!(message.contains("magic"), "{message}"),
            other => panic!("expected a bad-magic Corrupt, got {:?}", other.err()),
        }
        // One byte over is refused for its size before it is parsed.
        file.set_len(MAX_VAULT_FILE_BYTES + 1).unwrap();
        match Vault::unlock(&path, PASS) {
            Err(VaultError::Corrupt(message)) => {
                assert!(message.contains("larger than"), "{message}");
            }
            other => panic!("expected a size Corrupt, got {:?}", other.err()),
        }
    }

    #[test]
    fn upsert_stamps_unlock_rounds_not_current_constant() {
        let path = tmp_path("old-rounds.vault");
        let key = key_of("bot", [7u8; super::SALT_LEN], 50_000);
        let empty: BTreeMap<String, Profile> = Default::default();
        let data = serialize_profiles(&empty).unwrap();
        let blob = build_blob(&key, None, &data).unwrap();
        std::fs::write(&path, blob).unwrap();

        let mut v = Vault::unlock(&path, "bot").unwrap();
        v.upsert(profile("alice", "pw")).unwrap();
        drop(v);

        let v = Vault::unlock(&path, "bot").unwrap();
        assert_eq!(v.get("alice").unwrap().password, "pw");
        assert_eq!(header_rounds(&std::fs::read(&path).unwrap()), 50_000);
    }

    #[cfg(unix)]
    #[test]
    fn create_file_mode_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = tmp_path("mode.vault");
        Vault::create(&path, PASS).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "vault file must be owner-read/write only");
    }

    #[test]
    fn remove_deletes_only_that_profile_and_persists() {
        let path = tmp_path("remove.vault");
        let mut v = Vault::create(&path, PASS).unwrap();
        v.upsert(profile("alice", "pw1")).unwrap();
        v.upsert(profile("bob", "pw2")).unwrap();
        assert!(v.remove("alice").unwrap(), "chooser ✕ removes the row");
        assert!(
            !v.remove("alice").unwrap(),
            "a second remove of the same name is a no-op"
        );
        assert!(v.get("alice").is_none());
        assert_eq!(v.get("bob").unwrap().password, "pw2");
        drop(v);
        let v = Vault::unlock(&path, PASS).unwrap();
        assert!(v.get("alice").is_none(), "removal persists across unlock");
        assert!(v.get("bob").is_some());
    }

    #[test]
    fn legacy_profile_json_has_no_assignment_or_settings() {
        let missing: ProfileSettings = serde_json::from_str(r#"{"lowmem":true}"#).unwrap();
        assert!(missing.script_assignment.is_none());
        assert!(missing.script_settings.is_empty());
    }

    #[test]
    fn assignment_roundtrip_keeps_missing_source_identity() {
        let mut settings = ProfileSettings {
            script_assignment: Some(super::ScriptAssignment {
                source_kind: "file".into(),
                identity: "/tmp/gone/bot.ts".into(),
                display_name: "bot".into(),
                unavailable: Some("missing file: /tmp/gone/bot.ts".into()),
            }),
            ..Default::default()
        };
        let mut overrides = serde_json::Map::new();
        overrides.insert("buryBones".into(), serde_json::json!(false));
        settings
            .script_settings
            .insert("file:/tmp/gone/bot.ts".into(), overrides);
        let raw = serde_json::to_string(&settings).unwrap();
        let back: ProfileSettings = serde_json::from_str(&raw).unwrap();
        let asg = back.script_assignment.unwrap();
        assert_eq!(asg.source_kind, "file");
        assert_eq!(asg.identity, "/tmp/gone/bot.ts");
        assert_eq!(asg.display_name, "bot");
        assert_eq!(
            asg.unavailable.as_deref(),
            Some("missing file: /tmp/gone/bot.ts")
        );
        assert_eq!(
            back.script_settings
                .get("file:/tmp/gone/bot.ts")
                .and_then(|m| m.get("buryBones")),
            Some(&serde_json::json!(false))
        );
    }

    #[test]
    fn store_commit_persists_changes_in_order_and_staging_stays_in_memory() {
        let path = tmp_path("store-commit.vault");
        let mut vault = Vault::create(&path, PASS).unwrap();
        let mut store = vault.store();
        vault.stage_upsert(profile("alice", "a"));
        assert!(
            Vault::unlock(&path, PASS).unwrap().get("alice").is_none(),
            "staging never writes"
        );
        store
            .commit_legacy(&[
                VaultChange::Upsert(profile("alice", "a")),
                VaultChange::Upsert(profile("bob", "b")),
                VaultChange::Remove("bob".into()),
            ])
            .unwrap();
        let reopened = Vault::unlock(&path, PASS).unwrap();
        assert_eq!(reopened.get("alice").unwrap().password, "a");
        assert!(reopened.get("bob").is_none());
        assert_eq!(store.get("alice").unwrap().password, "a");
    }

    #[test]
    fn a_failed_store_commit_rolls_back_the_in_memory_copy() {
        let path = tmp_path("store-fail.vault");
        let vault = Vault::create(&path, PASS).unwrap();
        let mut store = vault.store();
        // The publish cannot replace a directory (on any platform).
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        assert!(store
            .commit_legacy(&[VaultChange::Upsert(profile("alice", "a"))])
            .is_err());
        assert!(store.get("alice").is_none());
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn serialized_profiles_match_serde_across_buffer_growth() {
        let mut profiles = BTreeMap::new();
        for i in 0..200 {
            let name = format!("user{i:03}");
            profiles.insert(name.clone(), profile(&name, &"p".repeat(64 + i)));
        }

        let ours = serialize_profiles(&profiles).unwrap();

        assert!(ours.len() > 4096 * 4, "large enough to regrow the buffer");
        assert_eq!(&ours[..], &serde_json::to_vec(&profiles).unwrap()[..]);
    }

    /// The legacy 0.1.9 fixture sealed as a credential-only image: the key's
    /// salt and rounds are the source's, and the plaintext holds exactly
    /// `username` and `password` per account.
    fn sealed_legacy_fixture() -> (Vec<u8>, super::VaultBinding, VaultKey) {
        let (key, opened) = super::authenticate(LEGACY_VAULT, "bot").unwrap();
        let super::Opened::Legacy(profiles) = opened else {
            panic!("the 0.1.9 fixture is a legacy vault")
        };
        let binding = super::VaultBinding::new("Local-274", "vault").unwrap();
        let blob = super::seal(&key, &binding, &super::credentials_of(&profiles)).unwrap();
        (blob, binding, key)
    }

    #[test]
    fn a_sealed_vault_keeps_the_source_kdf_and_holds_only_credentials() {
        let (blob, binding, key) = sealed_legacy_fixture();
        assert_eq!(binding.server(), "local-274", "the server key is lowercase");
        assert_eq!(&blob[9..13], &LEGACY_VAULT[9..13], "rounds kept");
        assert_eq!(&blob[13..29], &LEGACY_VAULT[13..29], "salt kept");
        let raw = decrypted_profiles(&blob, "bot");
        for (name, row) in raw.as_object().unwrap() {
            let mut fields: Vec<&str> = row
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            fields.sort_unstable();
            assert_eq!(fields, ["password", "username"], "{name}: credentials only");
        }
        match super::authenticate(&blob, "bot").unwrap().1 {
            super::Opened::Sealed {
                binding: found,
                credentials,
            } => {
                assert_eq!(found, binding);
                assert_eq!(credentials.len(), raw.as_object().unwrap().len());
            }
            super::Opened::Legacy(_) => panic!("a sealed image reads as sealed"),
        }
        assert!(matches!(
            super::authenticate_with_key(&blob, &key),
            Ok(super::Opened::Sealed { .. })
        ));
        assert!(matches!(
            super::authenticate(&blob, "not-bot"),
            Err(VaultError::WrongPassphrase)
        ));
    }

    #[test]
    fn a_changed_binding_header_never_authenticates() {
        let (blob, binding, key) = sealed_legacy_fixture();
        // Same length, another server: the GCM tag covers the header.
        let mut moved = blob;
        let at = super::HEADER_LEN + 1;
        assert_eq!(&moved[at..at + binding.server().len()], b"local-274");
        moved[at..at + 9].copy_from_slice(b"local-289");
        assert!(matches!(
            super::authenticate_with_key(&moved, &key),
            Err(VaultError::WrongPassphrase)
        ));
        assert!(matches!(
            super::authenticate(&moved, "bot"),
            Err(VaultError::WrongPassphrase)
        ));
    }

    /// `parse_header` exactly as 0.2.9 shipped it
    /// (`9d2407404:crates/vault/src/lib.rs:624-645`), with its constants.
    fn shipped_0_2_9_parse_header(blob: &[u8]) -> Result<([u8; 16], u32, &[u8]), VaultError> {
        const MAGIC: &[u8; 8] = b"274VAULT";
        const FORMAT_VERSION: u8 = 1;
        const SALT_LEN: usize = 16;
        const NONCE_LEN: usize = 12;
        const TAG_LEN: usize = 16;
        const MAX_PBKDF2_ROUNDS: u32 = 2_000_000;
        const HEADER_LEN: usize = MAGIC.len() + 1 + 4 + SALT_LEN + NONCE_LEN;
        if blob.len() < HEADER_LEN {
            return Err(VaultError::Corrupt("file too short".into()));
        }
        if &blob[..MAGIC.len()] != MAGIC {
            return Err(VaultError::Corrupt("bad magic".into()));
        }
        if blob[MAGIC.len()] != FORMAT_VERSION {
            return Err(VaultError::Corrupt("unsupported format version".into()));
        }
        let rounds_bytes: [u8; 4] = blob[MAGIC.len() + 1..MAGIC.len() + 5]
            .try_into()
            .map_err(|_| VaultError::Corrupt("short rounds field".into()))?;
        let rounds = u32::from_le_bytes(rounds_bytes);
        if rounds == 0 || rounds > MAX_PBKDF2_ROUNDS {
            return Err(VaultError::Corrupt("invalid pbkdf2 rounds".into()));
        }
        if blob.len() < HEADER_LEN + TAG_LEN {
            return Err(VaultError::Corrupt("ciphertext too short".into()));
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&blob[MAGIC.len() + 5..HEADER_LEN - NONCE_LEN]);
        Ok((salt, rounds, &blob[HEADER_LEN - NONCE_LEN..]))
    }

    /// R4: the shipped 0.2.9 decoder reports a converted vault as corrupt
    /// before it derives a key, on a test copy, without any reset.
    #[test]
    fn the_shipped_0_2_9_decoder_refuses_a_converted_vault() {
        assert!(shipped_0_2_9_parse_header(LEGACY_VAULT).is_ok());
        let (blob, _, _) = sealed_legacy_fixture();
        match shipped_0_2_9_parse_header(&blob) {
            Err(VaultError::Corrupt(message)) => {
                assert_eq!(message, "unsupported format version")
            }
            other => panic!("0.2.9 must refuse the converted vault: {:?}", other.err()),
        }
    }

    #[test]
    fn a_checked_replacement_publishes_only_over_the_expected_image() {
        let path = tmp_path("checked-replace.vault");
        std::fs::write(&path, LEGACY_VAULT).unwrap();
        let (blob, _, _) = sealed_legacy_fixture();
        let stale = super::file_digest(b"some other image");
        assert!(matches!(
            super::replace_checked(&path, &blob, &stale),
            Err(VaultError::Changed(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), LEGACY_VAULT, "kept");
        super::replace_checked(&path, &blob, &super::file_digest(LEGACY_VAULT)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), blob);
        assert!(
            Vault::unlock(&path, "bot").is_err(),
            "the legacy reader never adopts a credential-only image"
        );
    }
}
