//! Encrypted profile vault.
//!
//! Profiles are serialized to JSON and sealed with AES-256-GCM under a key
//! derived from the passphrase via PBKDF2-HMAC-SHA256. The vault file stores
//! only the KDF salt, nonce, and ciphertext; passphrases and profile
//! passwords never appear in plaintext. A wrong passphrase fails the unlock
//! without modifying the file.
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
    create_private_dir, create_private_file, read_private_file, read_regular_file,
    write_private_file,
};
pub use secret::Secret;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use pbkdf2::pbkdf2_hmac;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::Sha256;
use zeroize::Zeroizing;

mod compiled_settings;
pub use compiled_settings::CompiledSettingsRecord;

const MAGIC: &[u8; 8] = b"274VAULT";
const FORMAT_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
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
const HEADER_LEN: usize = MAGIC.len() + 1 + 4 + SALT_LEN + NONCE_LEN;

/// Per-profile settings. Low-memory is the default for headless clients;
/// auto-login defaults off so v1 blobs (which only carried `lowmem`)
/// deserialize with the box unchecked.
/// How this slot paints the 274 scene. Off is `set_draw` only. Gpu↔Cpu
/// (or lowmem) on a live slot drops + reattaches the `Renderer`; the
/// `Client` and its socket stay up.
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
            lamp_skill: "strength".into(),
            lamp_auto: true,
            clue_duel_partner: String::new(),
            script_assignment: None,
            script_settings: BTreeMap::new(),
        }
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
    #[error("corrupt vault file: {0}")]
    Corrupt(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// An unlocked vault. The AES-256 key lives in RAM (zeroized on drop) until
/// the vault is dropped; nothing is persisted until [`Vault::upsert`].
pub struct Vault {
    path: PathBuf,
    salt: [u8; SALT_LEN],
    key: Zeroizing<[u8; KEY_LEN]>,
    /// KDF rounds used to derive `key`. Persist stamps this, not the current
    /// [`PBKDF2_ROUNDS`] constant, so raising the constant cannot brick an
    /// already-unlocked file on the next upsert.
    rounds: u32,
    profiles: BTreeMap<String, Profile>,
}

impl Vault {
    /// Creates a new empty vault at `path`. Fails if the file already exists.
    /// The passphrase must satisfy [`check_new_passphrase`]; nothing is
    /// written when it does not.
    pub fn create(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        check_new_passphrase(passphrase)?;
        if path.exists() {
            return Err(VaultError::AlreadyExists(path.to_path_buf()));
        }
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let key = derive_key(passphrase, &salt, PBKDF2_ROUNDS);
        let empty: BTreeMap<String, Profile> = BTreeMap::new();
        let data = serialize_profiles(&empty)?;
        let blob = build_blob(&salt, &key, &data, PBKDF2_ROUNDS)?;
        create_private_file(path, &blob).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => VaultError::AlreadyExists(path.to_path_buf()),
            _ => VaultError::Io(e),
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            salt,
            key,
            rounds: PBKDF2_ROUNDS,
            profiles: BTreeMap::new(),
        })
    }

    /// Opens the vault at `path` with the given passphrase. Any non-empty
    /// passphrase that decrypts the file is accepted, including one of only
    /// whitespace: releases through 0.1.9.1 created vaults under that rule,
    /// and the new-vault trim check must never lock them out.
    pub fn unlock(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::EmptyPassphrase);
        }
        let blob = read_vault_file(path)?;
        let (salt, rounds, payload) = parse_header(&blob)?;
        let key = derive_key(passphrase, &salt, rounds);
        let plaintext =
            Zeroizing::new(decrypt(&key, payload).map_err(|_| VaultError::WrongPassphrase)?);
        let profiles: BTreeMap<String, Profile> = serde_json::from_slice(&plaintext)
            .map_err(|e| VaultError::Corrupt(format!("deserialize profiles: {e}")))?;
        Ok(Self {
            path: path.to_path_buf(),
            salt,
            key,
            rounds,
            profiles,
        })
    }

    /// Looks up a profile by username.
    pub fn get(&self, username: &str) -> Option<&Profile> {
        self.profiles.get(username)
    }

    /// All stored profiles (sorted by username; the map is a `BTreeMap`).
    pub fn profiles(&self) -> impl Iterator<Item = &Profile> {
        self.profiles.values()
    }

    /// Inserts or replaces a profile and rewrites the encrypted file. The new
    /// state is written to disk first; on error the vault is unchanged both on
    /// disk and in memory.
    pub fn upsert(&mut self, profile: Profile) -> Result<(), VaultError> {
        let mut next = self.profiles.clone();
        next.insert(profile.username.clone(), profile);
        self.persist_map(&next)?;
        self.profiles = next;
        Ok(())
    }

    /// Removes a profile by username and rewrites the encrypted file (the
    /// chooser's row ✕). Returns false when no such profile exists. On error
    /// the vault is unchanged both on disk and in memory. Wall membership
    /// is untouched — a running member survives a chooser ✕.
    pub fn remove(&mut self, username: &str) -> Result<bool, VaultError> {
        let mut next = self.profiles.clone();
        if next.remove(username).is_none() {
            return Ok(false);
        }
        self.persist_map(&next)?;
        self.profiles = next;
        Ok(true)
    }

    /// Delete the vault file. Forgotten-password recovery — does **not**
    /// create a replacement. A missing file is already gone (`Ok`).
    pub fn reset_file(path: &Path) -> Result<(), VaultError> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn persist_map(&self, profiles: &BTreeMap<String, Profile>) -> Result<(), VaultError> {
        persist(&self.path, &self.salt, &self.key, self.rounds, profiles)
    }

    /// A detached durable copy of this vault (same file and key) for a
    /// writer that persists off the caller's thread. Once one exists, every
    /// write must go through it: [`Vault::stage_upsert`] /
    /// [`Vault::stage_remove`] change only this in-memory view.
    pub fn store(&self) -> VaultStore {
        VaultStore {
            path: self.path.clone(),
            salt: self.salt,
            key: Zeroizing::new(*self.key),
            rounds: self.rounds,
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

// Change batches are short-lived and small; boxing `Upsert` would add an
// allocation to every profile save only to shrink this transient enum.
#[allow(clippy::large_enum_variant)]
/// One profile change for [`VaultStore::commit`].
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

/// The durable side of a [`Vault`]: the profiles last written to disk plus
/// the key to write more. Owned by one writer; the key is zeroized on drop.
pub struct VaultStore {
    path: PathBuf,
    salt: [u8; SALT_LEN],
    key: Zeroizing<[u8; KEY_LEN]>,
    rounds: u32,
    profiles: BTreeMap<String, Profile>,
}

impl VaultStore {
    /// The durable value of one profile.
    pub fn get(&self, username: &str) -> Option<&Profile> {
        self.profiles.get(username)
    }

    /// Apply `changes` in order and rewrite the encrypted file once. On error
    /// the store is unchanged both on disk and in memory.
    pub fn commit(&mut self, changes: &[VaultChange]) -> Result<(), VaultError> {
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
        persist(&self.path, &self.salt, &self.key, self.rounds, &next)?;
        self.profiles = next;
        Ok(())
    }
}

fn persist(
    path: &Path,
    salt: &[u8; SALT_LEN],
    key: &[u8; KEY_LEN],
    rounds: u32,
    profiles: &BTreeMap<String, Profile>,
) -> Result<(), VaultError> {
    let data = serialize_profiles(profiles)?;
    let blob = build_blob(salt, key, &data, rounds)?;
    atomic_write(path, &blob)
}

/// The policy for a passphrase that will protect a **new** vault: it must not
/// be empty after surrounding whitespace is trimmed. Passphrase strength is
/// the user's choice. [`Vault::create`] applies it; a caller that prompts can
/// apply it first and ask again instead of failing late. It is also used when
/// opening a missing vault through a frontend. [`Vault::unlock`] does not
/// apply it (see there).
pub fn check_new_passphrase(passphrase: &str) -> Result<(), VaultError> {
    if passphrase.trim().is_empty() {
        Err(VaultError::EmptyPassphrase)
    } else {
        Ok(())
    }
}

/// Upper bound on the vault file read into memory. A real vault is tens of
/// KiB; this only stops a crafted or wrong file from being read whole.
const MAX_VAULT_FILE_BYTES: u64 = 16 * 1024 * 1024;

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
fn serialize_profiles(
    profiles: &BTreeMap<String, Profile>,
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let mut out = SecretBuf(Zeroizing::new(Vec::with_capacity(4096)));
    serde_json::to_writer(&mut out, profiles)
        .map_err(|e| VaultError::Corrupt(format!("serialize profiles: {e}")))?;
    Ok(out.0)
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

/// Header bytes (magic, version, rounds, salt, nonce) followed by
/// ciphertext || 16-byte GCM tag.
fn build_blob(
    salt: &[u8; SALT_LEN],
    key: &[u8; KEY_LEN],
    plaintext: &[u8],
    rounds: u32,
) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| VaultError::Corrupt(format!("aes-gcm encrypt: {e}")))?;
    let mut blob = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.push(FORMAT_VERSION);
    blob.extend_from_slice(&rounds.to_le_bytes());
    blob.extend_from_slice(salt);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Returns (salt, rounds, nonce || ciphertext || tag).
fn parse_header(blob: &[u8]) -> Result<([u8; SALT_LEN], u32, &[u8]), VaultError> {
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
        return Err(VaultError::Corrupt(format!(
            "pbkdf2 rounds {rounds} outside the accepted range 1..={MAX_PBKDF2_ROUNDS}"
        )));
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&blob[MAGIC.len() + 5..HEADER_LEN - NONCE_LEN]);
    Ok((salt, rounds, &blob[HEADER_LEN - NONCE_LEN..]))
}

fn decrypt(key: &[u8; KEY_LEN], nonce_and_payload: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher.decrypt(
        Nonce::from_slice(&nonce_and_payload[..NONCE_LEN]),
        &nonce_and_payload[NONCE_LEN..],
    )
}

/// Writes `blob` to `path` via a same-directory temp file + rename so a
/// crash mid-write can never leave a truncated vault at `path`.
fn atomic_write(path: &Path, blob: &[u8]) -> Result<(), VaultError> {
    write_private_file(path, blob).map_err(VaultError::Io)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{
        build_blob, derive_key, parse_header, serialize_profiles, Profile, ProfileSettings, Vault,
        VaultChange, VaultError, MAX_PBKDF2_ROUNDS, MAX_VAULT_FILE_BYTES, SALT_LEN,
    };

    /// A passphrase used by the vaults these tests create.
    const PASS: &str = "test-passphrase-01";

    /// A real vault written by the code that shipped before this policy (0.1.9:
    /// passphrase `bot`, 100,000 rounds, two synthetic profiles). It is that
    /// writer's own output, not something this build produced.
    const LEGACY_VAULT: &[u8] = include_bytes!("../tests/fixtures/legacy-0.1.9.vault");

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
        let salt = [7u8; SALT_LEN];
        let key = super::derive_key(passphrase, &salt, super::PBKDF2_ROUNDS);
        let data = super::serialize_profiles(&std::collections::BTreeMap::new()).unwrap();
        let blob = super::build_blob(&salt, &key, &data, super::PBKDF2_ROUNDS).unwrap();
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
        let creators: Vec<_> = (0..4)
            .map(|i| {
                let (path, start) = (path.clone(), start.clone());
                std::thread::spawn(move || {
                    let passphrase = format!("passphrase-number-{i}");
                    start.wait();
                    Vault::create(&path, &passphrase).map(|_| passphrase)
                })
            })
            .collect();
        let outcomes: Vec<_> = creators.into_iter().map(|c| c.join().unwrap()).collect();

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
    fn reset_file_removes_vault_missing_is_ok() {
        let path = tmp_path("reset.vault");
        Vault::create(&path, PASS).unwrap();
        assert!(path.is_file());
        Vault::reset_file(&path).unwrap();
        assert!(!path.exists());
        Vault::reset_file(&path).unwrap();
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
        build_blob(&[1u8; SALT_LEN], &[9u8; 32], b"{}", rounds).unwrap()
    }

    #[test]
    fn stored_round_counts_are_accepted_only_inside_the_cap() {
        for accepted in [1, 100_000, MAX_PBKDF2_ROUNDS] {
            assert!(
                parse_header(&blob_with_rounds(accepted)).is_ok(),
                "{accepted} rounds"
            );
        }
        for rejected in [0, MAX_PBKDF2_ROUNDS + 1, u32::MAX] {
            match parse_header(&blob_with_rounds(rejected)) {
                Err(VaultError::Corrupt(message)) => assert!(
                    message.contains(&rejected.to_string())
                        && message.contains(&MAX_PBKDF2_ROUNDS.to_string()),
                    "the error names the value and the cap: {message}"
                ),
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
        let salt = [7u8; super::SALT_LEN];
        let rounds = 50_000;
        let key = derive_key("bot", &salt, rounds);
        let empty: BTreeMap<String, Profile> = Default::default();
        let data = serialize_profiles(&empty).unwrap();
        let blob = build_blob(&salt, &key, &data, rounds).unwrap();
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
            .commit(&[
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
            .commit(&[VaultChange::Upsert(profile("alice", "a"))])
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
}
