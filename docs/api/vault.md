# Vault: encrypted profiles

`crates/vault` stores login profiles in a single encrypted file. Profiles
are JSON sealed with **AES-256-GCM** under a key derived from the passphrase
via **PBKDF2-HMAC-SHA256** (100,000 rounds for new files; unlock reads the
round count from the file header and **rejects** 0 or values above 10M as
`Corrupt`). Persist stamps the rounds the key was derived with, not the
current constant. The file holds only
the KDF salt, nonce, and ciphertext — passphrases and profile passwords
never appear in plaintext. The AES key lives in RAM (zeroized on drop).

## File format

```
"274VAULT" | version(1) | pbkdf2_rounds(le u32) | salt(16) | nonce(12) | ciphertext ‖ gcm_tag(16)
```

Writes are atomic (same-directory temp file + rename), so a crash cannot
leave a truncated vault at the target path.

## API

```rust
let mut v = Vault::create(path, passphrase)?;   // fails if the file exists
let v = Vault::unlock(path, passphrase)?;       // WrongPassphrase on a bad key
v.get(username) -> Option<&Profile>             // { username, password, uid, settings }
v.upsert(profile)?                              // rewrites the file; error leaves state unchanged
```

`Profile.settings.lowmem` defaults to `true` (headless clients).

Errors: `EmptyPassphrase`, `AlreadyExists`, `NotFound`, `WrongPassphrase`
(the file is left unmodified), `Corrupt`, `Io`.

## Passphrase sourcing

The passphrase is supplied to the CLI via the **`BOT_VAULT_PASS`** env var
or the **`--vault-pass`** flag (flag wins). One of them is required to open
a vault; an empty passphrase is rejected.
