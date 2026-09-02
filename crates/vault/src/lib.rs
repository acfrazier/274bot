//! Encrypted profile vault.
//!
//! Profiles are serialized to JSON and sealed with AES-256-GCM under a key
//! derived from the passphrase via PBKDF2-HMAC-SHA256. The vault file stores
//! only the KDF salt, nonce, and ciphertext; passphrases and profile
//! passwords never appear in plaintext. An empty passphrase is rejected;
//! a wrong passphrase fails the unlock without modifying the file.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use pbkdf2::pbkdf2_hmac;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"274VAULT";
const FORMAT_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
/// PBKDF2 iterations used when creating a new vault. Unlock reads the round
/// count from the file header, so this can be raised without breaking files.
/// `parse_header` rejects round counts above `MAX_PBKDF2_ROUNDS` so a crafted
/// file cannot force a long KDF.
const PBKDF2_ROUNDS: u32 = 100_000;
const MAX_PBKDF2_ROUNDS: u32 = 10_000_000;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub lowmem: bool,
    #[serde(default)]
    pub auto_login: bool,
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
            auto_login: false,
            tutorial_skipped: None,
            raster: RasterMode::Gpu,
            random_events: true,
            lamp_skill: "strength".into(),
            lamp_auto: true,
        }
    }
}

/// A stored login profile, keyed by username.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub username: String,
    pub password: String,
    pub uid: i32,
    pub settings: ProfileSettings,
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
    pub fn create(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        require_passphrase(passphrase)?;
        if path.exists() {
            return Err(VaultError::AlreadyExists(path.to_path_buf()));
        }
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let key = derive_key(passphrase, &salt, PBKDF2_ROUNDS);
        let empty: BTreeMap<String, Profile> = BTreeMap::new();
        let data = serde_json::to_vec(&empty)
            .map_err(|e| VaultError::Corrupt(format!("serialize profiles: {e}")))?;
        let blob = build_blob(&salt, &key, &data, PBKDF2_ROUNDS)?;
        atomic_write(path, &blob)?;
        Ok(Self {
            path: path.to_path_buf(),
            salt,
            key,
            rounds: PBKDF2_ROUNDS,
            profiles: BTreeMap::new(),
        })
    }

    /// Opens the vault at `path` with the given passphrase.
    pub fn unlock(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        require_passphrase(passphrase)?;
        let blob = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::NotFound(path.to_path_buf()));
            }
            Err(e) => return Err(VaultError::Io(e)),
        };
        let (salt, rounds, payload) = parse_header(&blob)?;
        let key = derive_key(passphrase, &salt, rounds);
        let plaintext = decrypt(&key, payload).map_err(|_| VaultError::WrongPassphrase)?;
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
        let data = serde_json::to_vec(profiles)
            .map_err(|e| VaultError::Corrupt(format!("serialize profiles: {e}")))?;
        let blob = build_blob(&self.salt, &self.key, &data, self.rounds)?;
        atomic_write(&self.path, &blob)
    }
}

fn require_passphrase(passphrase: &str) -> Result<(), VaultError> {
    if passphrase.is_empty() {
        Err(VaultError::EmptyPassphrase)
    } else {
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
        return Err(VaultError::Corrupt("pbkdf2 rounds out of range".into()));
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

/// Writes `data` to `path` via a same-directory `.tmp` file + rename. On Unix
/// the parent directory is created `0o700` and the final file is `0o600`
/// (explicit on the temp file before rename so umask cannot widen it).
pub fn write_private_file(path: &Path, data: &[u8]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        #[cfg(unix)]
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        #[cfg(not(unix))]
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        #[cfg(unix)]
        set_mode(&tmp, 0o600)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::DirBuilderExt;

    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_dir(dir: &Path) -> Result<(), std::io::Error> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Writes `blob` to `path` via a same-directory temp file + rename so a
/// crash mid-write can never leave a truncated vault at `path`.
fn atomic_write(path: &Path, blob: &[u8]) -> Result<(), VaultError> {
    write_private_file(path, blob).map_err(VaultError::Io)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Profile, ProfileSettings, Vault, VaultError};

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
                auto_login: false,
                tutorial_skipped: None,
                raster: super::RasterMode::Gpu,
                random_events: true,
                lamp_skill: "strength".into(),
                lamp_auto: true,
            },
        }
    }

    #[test]
    fn auto_login_defaults_false_and_old_json_unlocks_off() {
        assert!(!ProfileSettings::default().auto_login);
        let path = tmp_path("old-settings.vault");
        let mut v = Vault::create(&path, "bot").unwrap();
        v.upsert(Profile {
            username: "a".into(),
            password: "a".into(),
            uid: 1,
            settings: ProfileSettings {
                lowmem: true,
                auto_login: false,
                tutorial_skipped: None,
                raster: super::RasterMode::Gpu,
                random_events: true,
                lamp_skill: "strength".into(),
                lamp_auto: true,
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

        let mut v = Vault::create(&path, "bot").unwrap();
        v.upsert(profile("zezima", "hunter2")).unwrap();
        drop(v);

        let v = Vault::unlock(&path, "bot").unwrap();
        let p = v.get("zezima").expect("profile present after unlock");
        assert_eq!(p.password, "hunter2");
        assert_eq!(p.uid, 42);
        assert!(!p.settings.lowmem);
        assert!(v.get("nobody").is_none());
    }

    #[test]
    fn empty_passphrase_rejected() {
        let path = tmp_path("empty.vault");

        assert!(matches!(
            Vault::create(&path, ""),
            Err(VaultError::EmptyPassphrase)
        ));
        // Nothing written on failure.
        assert!(!path.exists());

        // Unlock must reject an empty passphrase too.
        Vault::create(&path, "bot").unwrap();
        assert!(matches!(
            Vault::unlock(&path, ""),
            Err(VaultError::EmptyPassphrase)
        ));
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

        let mut v = Vault::create(&path, "bot").unwrap();
        v.upsert(profile("zezima", "hunter2isasecret")).unwrap();
        drop(v);

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.windows(5).any(|w| w == b"hunte"),
            "profile password bytes leaked into the ciphertext file"
        );
        assert!(
            !bytes.windows(3).any(|w| w == b"bot"),
            "vault passphrase bytes leaked into the ciphertext file"
        );
    }

    #[test]
    fn create_refuses_existing_vault() {
        let path = tmp_path("exists.vault");

        Vault::create(&path, "bot").unwrap();
        assert!(matches!(
            Vault::create(&path, "bot"),
            Err(VaultError::AlreadyExists(_))
        ));
    }

    #[test]
    fn unlock_directory_is_io_not_not_found() {
        let dir = tmp_path("vault-as-dir");
        std::fs::create_dir_all(&dir).unwrap();
        match Vault::unlock(&dir, "bot") {
            Err(VaultError::Io(_)) => {}
            Err(e) => panic!("expected Io, got {e}"),
            Ok(_) => panic!("expected Io, unlocked a directory"),
        }
        assert!(dir.is_dir(), "must not replace the path with a vault file");
    }

    #[test]
    fn reset_file_removes_vault_missing_is_ok() {
        let path = tmp_path("reset.vault");
        Vault::create(&path, "bot").unwrap();
        assert!(path.is_file());
        Vault::reset_file(&path).unwrap();
        assert!(!path.exists());
        Vault::reset_file(&path).unwrap();
        Vault::create(&path, "newpass").unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn upsert_failure_leaves_state_unchanged() {
        let dir = std::env::temp_dir()
            .join(format!("274bot-vault-test-{}", std::process::id()))
            .join("rollback.d");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("vault");

        let mut v = Vault::create(&file, "bot").unwrap();
        v.upsert(profile("alice", "pw1")).unwrap();

        // Drop write permission so the next atomic write cannot succeed.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        }
        #[cfg(not(unix))]
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(v.upsert(profile("bob", "pw2")).is_err());
        assert!(
            v.get("bob").is_none(),
            "failed upsert must not change in-memory state"
        );
        assert_eq!(v.get("alice").unwrap().password, "pw1");
    }

    #[test]
    fn unlock_rejects_absurd_pbkdf2_rounds() {
        let path = tmp_path("rounds.vault");
        Vault::create(&path, "bot").unwrap();

        // Patch the header's rounds field to u32::MAX.
        let mut bytes = std::fs::read(&path).unwrap();
        let rounds_off = b"274VAULT".len() + 1;
        bytes[rounds_off..rounds_off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();

        assert!(matches!(
            Vault::unlock(&path, "bot"),
            Err(VaultError::Corrupt(_))
        ));
    }

    #[test]
    fn upsert_stamps_unlock_rounds_not_current_constant() {
        let path = tmp_path("old-rounds.vault");
        let salt = [7u8; super::SALT_LEN];
        let rounds = 50_000;
        let key = super::derive_key("bot", &salt, rounds);
        let empty: std::collections::BTreeMap<String, Profile> = Default::default();
        let data = serde_json::to_vec(&empty).unwrap();
        let blob = super::build_blob(&salt, &key, &data, rounds).unwrap();
        std::fs::write(&path, blob).unwrap();

        let mut v = Vault::unlock(&path, "bot").unwrap();
        v.upsert(profile("alice", "pw")).unwrap();
        drop(v);

        let v = Vault::unlock(&path, "bot").unwrap();
        assert_eq!(v.get("alice").unwrap().password, "pw");
        let bytes = std::fs::read(&path).unwrap();
        let rounds_off = b"274VAULT".len() + 1;
        let stored = u32::from_le_bytes(bytes[rounds_off..rounds_off + 4].try_into().unwrap());
        assert_eq!(stored, 50_000);
    }

    #[cfg(unix)]
    #[test]
    fn create_file_mode_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = tmp_path("mode.vault");
        Vault::create(&path, "bot").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "vault file must be owner-read/write only");
    }

    #[test]
    fn remove_deletes_only_that_profile_and_persists() {
        let path = tmp_path("remove.vault");
        let mut v = Vault::create(&path, "bot").unwrap();
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
        let v = Vault::unlock(&path, "bot").unwrap();
        assert!(v.get("alice").is_none(), "removal persists across unlock");
        assert!(v.get("bob").is_some());
    }
}
