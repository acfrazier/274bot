# Vault: encrypted profiles

`crates/vault` stores login profiles in a single encrypted file. Profiles
are JSON sealed with **AES-256-GCM** under a key derived from the passphrase
via **PBKDF2-HMAC-SHA256** (100,000 rounds for new files; unlock reads the
round count from the file header and **rejects** 0 or anything above
**2,000,000** as `Corrupt`, naming the value and the accepted range, before
any key is derived, and refuses a vault file over 16 MiB). Persist stamps the
rounds the key was derived with, not the current constant. The file holds only
the KDF salt, nonce, and ciphertext — passphrases and profile passwords
never appear in plaintext. The AES key lives in RAM (zeroized on drop).
Argon2 is not used: a change of KDF would be a format-version bump and a
re-seal, which is out of scope for 0.2.0.

## File format

```
"274VAULT" | version(1) | pbkdf2_rounds(le u32) | salt(16) | nonce(12) | ciphertext ‖ gcm_tag(16)
```

Writes are atomic and each write is its own: the bytes are staged in a
unique, exclusively created same-directory temp file (`.<name>.<random>.tmp`,
mode `0o600`, so a planted symlink, directory or leftover file can never be
its name), flushed, checked to still be that same file, and only then renamed
over the target. This call never removes a temp it did not create. A crash
cannot leave a truncated vault at the target path; a failed write removes its
own temp and leaves the target unchanged, except that two writers to one
target in the same process take turns and, across processes, the last
published write wins (each writer's `Ok` means its own bytes were published;
a read-modify-write across processes needs its own lock). A writer that
crashes leaves its temp file behind. `Vault::create` publishes with a
no-replace link, so of any number of concurrent creators exactly one succeeds
and none replaces an existing vault.

## API

```rust
let mut v = Vault::create(path, passphrase)?;   // fails if the file exists; passphrase must be non-empty
let v = Vault::unlock(path, passphrase)?;       // WrongPassphrase on a bad key
Vault::reset_file(path)?;                       // delete the file (forgotten passphrase)
v.get(username) -> Option<&Profile>             // { username, password: Secret, uid, settings }
v.upsert(profile)?                              // rewrites the file; error leaves state unchanged
```

`Profile.settings.lowmem` defaults to `true` (headless clients).
`ProfileSettings.auto_login` defaults to `false` (serde default), so v1
blobs that only carried `lowmem` deserialize with the box unchecked; the
panel's General config → slot "auto-login on title" checkbox reads and upserts it.
`ProfileSettings.random_events` defaults **on** (detect + dialog act);
off still detects and publishes `RandomStatus`. `lamp_skill` /
`lamp_auto` persist with the profile.
`ProfileSettings.tutorial_skipped` is `Option<bool>` (serde default
`None` = unknown). The panel `getvar tutorial`s unknown profiles, then
caches `Some(true)` once skipped (`>= 1000` or TutSkip); live
tests can set the flag to skip the cheat.

`ProfileSettings.script_assignment` optionally stores the last successful
script Start identity (source + path/name key, optional `unavailable`
reason when the source later disappears). `ProfileSettings.script_settings`
holds per-card override bags keyed by card identity. See
[script.md](script.md).

Errors: `EmptyPassphrase`, `AlreadyExists`, `NotFound`, `WrongPassphrase` (the
file is left unmodified), `Corrupt`, `Io`. A missing file is
`NotFound`; other read failures are `Io` (never treated as missing, so
`open_vault` will not create over an unreadable path). Wrong passphrase
and corrupt files never delete or replace the vault — the panel's
**Reset vault** (confirm + “I understand”) is the only wipe. First-run
**Create vault** only runs when the file is absent.

## Passphrase policy and upgrading an existing vault

A **new** vault needs a passphrase that is not empty after surrounding
whitespace is trimmed. Passphrase strength is the user's choice. `Vault::create`
and first-run **Create vault** in every frontend refuse an empty or
whitespace-only passphrase and write nothing. `check_new_passphrase` applies
the same rule for a caller that prompts and wants to ask again.

An **existing** vault is opened with any non-empty passphrase that decrypts the
file, including one of only whitespace (allowed before 0.2.0). A vault created
under an earlier policy keeps opening, saving, and retaining its data. The file
format, the round count, and every stored profile are unchanged by an upgrade;
nothing is rewritten until the next ordinary save, and that save keeps the
header it found.

## Passphrase sourcing

The passphrase is **never** read from the environment or the command line:
both are readable by other local users (`ps`, `/proc/<pid>/environ`) and the
environment is inherited by every child process. `BOT_VAULT_PASS` and
`--vault-pass PASS` were removed with no alias. A process whose environment
still has `BOT_VAULT_PASS` says on stderr that it is ignored (the value is
never printed); `--vault-pass` and `--vault-pass=VALUE` fail with a message
that does not repeat the value.

| Front end | Channels |
| --- | --- |
| `panel-play` | the unlock window; `--vault-pass-stdin` unlocks before the window opens |
| `tui-play` | hidden terminal prompt; `--vault-pass-stdin` |
| `host-play` | hidden terminal prompt; `--vault-pass-stdin` |

- **Prompt.** When stdin is a terminal the passphrase is read without echo
  (raw mode is on before the prompt is shown, so input sent the moment it
  appears is never echoed). Enter, or a newline byte, ends the line;
  Backspace edits, Ctrl-U clears, Esc / Ctrl-C / Ctrl-D cancel. A new vault
  asks twice, rejects an empty or whitespace-only entry, and offers three attempts.
- **Standard input.** With `--vault-pass-stdin` and a pipe or file on stdin,
  the first line is the passphrase: only the trailing `\n` or `\r\n` is
  removed, at most 4096 bytes, UTF-8. Without the flag a pipe is never read
  as a passphrase. Example:
  `printf '%s\n' "$PASS" | tui-play --profile local-289 --vault-pass-stdin`
  (`printf` and `echo` are shell builtins, so nothing reaches another
  process's argv).
- **Launchers and harnesses.** Give a child a pty and type the passphrase at
  its prompt, or give it a pipe and `--vault-pass-stdin`. The `e2e-suite`
  launcher removes `BOT_VAULT_PASS` from every child's environment and
  refuses `--child-arg --vault-pass` (a child's argv is recorded verbatim in
  the run ledger). `--live` runs need no passphrase: they make a throwaway
  vault whose passphrase is fixed for a local engine (it protects nothing) and
  minted per run for the public world.

## Secrets in memory

`Profile.password` is a `vault::Secret` (also the panel's passphrase and
credential edit buffers): its buffer is overwritten with zeros when it is
dropped or cleared, `Debug` prints `<redacted>`, and it serializes as the
plain string, so the vault JSON is unchanged. The derived key, the AES key
schedule (`aes` built with its `zeroize` feature) and the serialized profiles
built to write the file are zeroed after use. The passphrase buffers of
`host-play` and `tui-play` are reserved at the 4096-byte limit up front (a
longer entry is refused at a pipe and ignored at the prompt) so none grows and
leaves a freed copy, and both front ends drop the passphrase as soon as the
vault is open, before the long run.

This is best effort, not a promise that every copy is wiped. It does not
reach the standard library's buffered `stdin` reader (which keeps its own copy
of what it read from a pipe), the terminal driver's queue, copies already
moved elsewhere (a login block handed to the network layer, the `Arc<str>` a
slot keeps for reconnects, the log-redaction list of registered secrets that
lives for the process), compiler moves and registers, or the PBKDF2/HMAC
working state inside those crates.

## State files beside the vault

`write_private_file` (also used for the script library store, settings and
logs) is the atomic writer above and `create_private_file` its no-replace
form (used by `Vault::create`). `read_private_file` is the reader for state
that decides what the host later loads or runs (`js-scripts.json`,
`rs2b0t-path`): it refuses anything that is not a regular file, is larger than
its bound, is owned by another user (root is accepted) or is writable by its
group or by others; state that others can only read (written before the
`0o600` writer) is tightened to `0o600` and read. See
[script.md](script.md#persistence) for what restore does with a refusal.

**Directory trust.** On Unix the immediate parent directory must be owned by
this user or root and must not be writable by group or others. A directory
this user owns that group or others can write to (an install made under a
`umask` of `002`, or by an older release) is tightened once, automatically,
on the first save (`mode & !0o022`, for example `0775` becomes `0755`) and the
result is re-checked, so an existing vault keeps saving without a manual
`chmod`. A directory owned by another user, or by root when this user is not
root, that others can write to is never modified and the write is refused with
`chmod go-w` guidance. A sticky directory such as `/tmp` is accepted as it is.
Only the immediate parent is checked, not its ancestors. The instance lock and
first-run vault creation create a missing `~/.274bot` owner-only (`0700`).

**Windows.** There are no mode bits to check and this crate reads and sets no
ACLs: the file and its directory get whatever ACL the parent directory
inherits (normally the per-user profile), and a shared or permissive location
is not detected. The staging file is created exclusively and opened with
sharing denied while it is written.
