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
const PBKDF2_ROUNDS: u32 = 100_000;
const HEADER_LEN: usize = MAGIC.len() + 1 + 4 + SALT_LEN + NONCE_LEN;

/// Per-profile settings. Low-memory is the default for headless clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub lowmem: bool,
}

impl Default for ProfileSettings {
    fn default() -> Self {
        Self { lowmem: true }
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
        let blob = build_blob(&salt, &key, &data)?;
        atomic_write(path, &blob)?;
        Ok(Self {
            path: path.to_path_buf(),
            salt,
            key,
            profiles: BTreeMap::new(),
        })
    }

    /// Opens the vault at `path` with the given passphrase.
    pub fn unlock(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        require_passphrase(passphrase)?;
        let blob = std::fs::read(path)
            .map_err(|_| VaultError::NotFound(path.to_path_buf()))?;
        let (salt, rounds, payload) = parse_header(&blob)?;
        let key = derive_key(passphrase, &salt, rounds);
        let plaintext =
            decrypt(&key, payload).map_err(|_| VaultError::WrongPassphrase)?;
        let profiles: BTreeMap<String, Profile> = serde_json::from_slice(&plaintext)
            .map_err(|e| VaultError::Corrupt(format!("deserialize profiles: {e}")))?;
        Ok(Self {
            path: path.to_path_buf(),
            salt,
            key,
            profiles,
        })
    }

    /// Looks up a profile by username.
    pub fn get(&self, username: &str) -> Option<&Profile> {
        self.profiles.get(username)
    }

    /// Inserts or replaces a profile and rewrites the encrypted file.
    pub fn upsert(&mut self, profile: Profile) -> Result<(), VaultError> {
        self.profiles.insert(profile.username.clone(), profile);
        self.persist()
    }

    fn persist(&self) -> Result<(), VaultError> {
        let data = serde_json::to_vec(&self.profiles)
            .map_err(|e| VaultError::Corrupt(format!("serialize profiles: {e}")))?;
        let blob = build_blob(&self.salt, &self.key, &data)?;
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

fn derive_key(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    rounds: u32,
) -> Zeroizing<[u8; KEY_LEN]> {
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
) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| VaultError::Corrupt(format!("aes-gcm encrypt: {e}")))?;
    let mut blob = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.push(FORMAT_VERSION);
    blob.extend_from_slice(&PBKDF2_ROUNDS.to_le_bytes());
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
    if rounds == 0 {
        return Err(VaultError::Corrupt("zero pbkdf2 rounds".into()));
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&blob[MAGIC.len() + 5..HEADER_LEN - NONCE_LEN]);
    Ok((salt, rounds, &blob[HEADER_LEN - NONCE_LEN..]))
}

fn decrypt(
    key: &[u8; KEY_LEN],
    nonce_and_payload: &[u8],
) -> Result<Vec<u8>, aes_gcm::Error> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher.decrypt(
        Nonce::from_slice(&nonce_and_payload[..NONCE_LEN]),
        &nonce_and_payload[NONCE_LEN..],
    )
}

/// Writes `blob` to `path` via a same-directory temp file + rename so a
/// crash mid-write can never leave a truncated vault at `path`.
fn atomic_write(path: &Path, blob: &[u8]) -> Result<(), VaultError> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(blob)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Profile, ProfileSettings, Vault, VaultError};

    fn tmp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-vault-test-{}", std::process::id()));
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
            settings: ProfileSettings { lowmem: false },
        }
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
}
