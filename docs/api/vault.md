# Vault: encrypted credentials

`crates/vault` stores each account's login credential (its exact username
and its password) in one encrypted file per server profile. The `uid` and
every profile setting live in the state database (`state.sqlite3`), keyed by
(canonical server profile, canonical account); `host-play`'s
`profile_vault` joins the two stores at unlock. Credentials are JSON sealed
with **AES-256-GCM** under a key derived from the passphrase via
**PBKDF2-HMAC-SHA256** (100,000 rounds for new files; unlock reads the round
count from the file header and **rejects** 0 or anything above
**2,000,000** as `Corrupt` before any key is derived, and refuses a vault
file over 16 MiB). Every replacement keeps the salt and round count the key
was derived with, not the current constant. The file holds only the KDF salt,
nonce, binding and ciphertext — passphrases and passwords never appear in
plaintext. The AES key lives in RAM (zeroized on drop). Argon2 is not used.

## Upgrading from 0.2.9: one-way conversion

> This upgrade converts saved state one way. Version 0.2.9 reports a
> converted vault as corrupt (`unsupported format version`). Do not use
> Reset or forgotten-passphrase recovery in 0.2.9 on a converted vault.
> Before upgrading, stop 274bot and make a private paired backup of the
> vault and its saved state.

The first unlock of a 0.2.9 vault converts it (SD1 §5, S8):

1. The vault is authenticated and decoded under the passphrase. A wrong
   passphrase stops here; nothing is written.
2. Account keys are checked. Usernames that form one login account (for
   example `Alice` and `alice`) and usernames that cannot form a login
   account are shown by name and reason only; the operator chooses the
   credential to keep, confirms removing each other listed credential (by
   name) with its settings, and confirms each invalid name's removal. A
   setting whose whole
   value or whole key equals a stored game password is shown by its
   category only; the operator clears it or confirms that it is not
   secret. Usernames and `clue_duel_partner` are account identifiers and
   are never compared; the vault passphrase is never compared. A start that
   cannot ask (headless `host-play`, a TUI without a terminal) refuses and
   changes nothing.
3. The profile rows are staged in bounded transactions, then one FULL
   transaction binds the server profile to its vault component, activates
   the rows and records the pending seal with both vault digests.
4. The credential-only image replaces the old file by private
   same-directory rename (file and directory synced), is authenticated
   again, and the pending record is settled.

A stop at any point is completed by the next unlock without losing a
setting. Every 0.2.9 default is kept: missing optional fields read as 0.2.9
read them, a saved `ignore` random-event policy reads as Flee (the new store
writes `flee`), an empty `compiled:<card>` entry is stored as the schema-1
envelope with empty `values`, and Gatherer schema-4 settings are stored as
schema 5 with the same overrides. Unsupported or malformed card settings keep
their original representation and their per-card refusal.

Once a server's binding exists its first import is closed: an old-format
vault placed at that path later is refused, never imported. A vault sealed
for another server profile or component never joins, even with the same
usernames. Edits a downgraded 0.2.9 makes are not imported back, and the
0.2.9 preference export (`state preferences export --same-root`) is retired.

## File format

```
"274VAULT" | version(2) | pbkdf2_rounds(le u32) | salt(16) | nonce(12)
  | server_len(u8) | server | component_len(u8) | component
  | ciphertext ‖ gcm_tag(16)
```

The plaintext is a map of exact username to `{username, password}`. The
header, binding included, is the GCM associated data, so a changed server or
component never authenticates. Version 1 (0.1.x–0.2.9: every profile with
its uid and settings) is read only for the first conversion.

Writes are atomic and each write is its own: the bytes are staged in a
unique, exclusively created same-directory temp file (`.<name>.<random>.tmp`,
mode `0o600`, so a planted symlink, directory or leftover file can never be
its name), flushed, checked to still be the expected image, and only then
renamed over the target; the directory is synced after the rename. A crash
cannot leave a truncated vault at the target path. A new vault is published
with a no-replace link, so of any number of concurrent creators exactly one
succeeds and none replaces an existing vault.

## API

```rust
let (bytes, digest) = vault::read_file(path)?;            // NotFound, size cap
let (key, opened) = vault::authenticate(&bytes, passphrase)?; // Legacy(profiles) | Sealed { binding, credentials }
let blob = vault::seal(&key, &binding, &credentials)?;   // credential-only image
vault::replace_checked(path, &blob, &expected_digest)?;  // only over the expected image
vault::remove_checked(path, &confirmed_digest)?;         // forgotten-passphrase reset
```

Frontends do not call these directly: `host_play::profile_vault::{unlock,
create, reset, ProfileStore}` own both stores. A settings-only edit commits in
SQL; a credential-only edit replaces the vault and keeps its binding; a create
writes its row before its credential; a delete hides its credential before
its row retires. A rename prepares the renamed row with both vault digests,
replaces the vault once and finalizes in one transaction; the profile writer
holds the vault from the snapshot through the finalization, and queued writes
wait for it. No SQL transaction writes or removes more than 1 MiB of
settings: a create, a rename and a settings batch larger than that stage
their rows in bounded batches, invisible to every join, and one small
transaction makes them live (a create batch and a settings batch all or
nothing). The writer collects the replaced and unfinished copies in bounded
batches right after each write, so a running session does not accumulate
them. A vault
replacement whose directory sync fails is uncertain: the pending record and
the rows stay, the session refuses further writes, and the next unlock
settles against the image the file then holds.

`Profile.settings.lowmem` defaults to `true` (headless clients).
`ProfileSettings.auto_login` defaults to `false` (serde default), so v1
blobs that only carried `lowmem` deserialize with the box unchecked; the
panel's Profiles edit form "Log in automatically" checkbox reads and saves it.
`ProfileSettings.random_events` defaults **on** (detect and handle events).
Off is **detect-only**, not stop-on-event: the Guardian still publishes the
event's kind and name, but does not act or mark it pending for scripts.
Held Strange boxes remain untouched when this setting is off. They still
obey the engine's item rules, including its refusal to bank them. Enable
random-event handling or solve the boxes manually to remove them.
`lamp_skill` / `lamp_auto` persist with the profile.
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
file is left unmodified), `InvalidBinding`, `Changed` (a checked write or
removal found another image; nothing changed), `Corrupt`, `Io`. A missing
file is `NotFound`; other read failures are `Io` (never treated as missing,
so an unreadable path is never created over). Wrong passphrase and corrupt
files never delete or replace the vault — the panel's **Reset vault**
(confirm + “I understand”) is the only wipe. First-run **Create vault** only
runs when the file is absent.

## Reset and recovery

**Reset vault** records the confirmed file's digest, deletes only that file
(syncing its directory), then retires the server's rows and cancels any
pending seal or rename in one transaction. The binding and the closed first
import stay. A later **Create vault** writes an empty credential-only vault
at the same component; a new profile there gets a fresh uid and its own
settings, and nothing of a retired row returns. If a different file appears
during a reset it is neither deleted nor adopted.

If a converted vault has credentials without saved settings, restore the
paired backup of the vault and its saved state. If no backup is usable,
stop every 274bot process for the root and run
`panel-play state vault recover --defaults --profile <server>` (or
`tui-play`). It asks for the vault passphrase, shows the missing-row count
and the loss of their original settings and uids, and after confirmation
creates only the missing rows with product defaults and fresh uids, above
every uid the server ever stored (a lost row's uid is never reused). It
refuses a pending conversion, rename or reset and a vault bound to another
server.

A file that ends before the end of its GCM tag is `Corrupt`, not
`WrongPassphrase`. This includes a file that ends right after its header. A
file damaged further in still reports `WrongPassphrase`, because the
encryption cannot tell the two apart.

## Passphrase policy and upgrading an existing vault

A **new** vault needs a passphrase that is not empty after surrounding
whitespace is trimmed. Passphrase strength is the user's choice. `VaultKey::create`
and first-run **Create vault** in every frontend refuse an empty or
whitespace-only passphrase and write nothing. `check_new_passphrase` applies
the same rule for a caller that prompts and wants to ask again.

An **existing** vault is opened with any non-empty passphrase that decrypts the
file, including one of only whitespace (allowed before 0.2.0). A vault created
under an earlier policy keeps opening, saving, and retaining its data. The file
format, the round count, and every stored profile are unchanged by an upgrade;
nothing is rewritten until the next ordinary save, and that save keeps the
header it found.

The passphrase is used exactly as typed, spaces included, in every frontend.
Earlier panel unlock windows stored and matched the trimmed text, so when
a typed passphrase with surrounding spaces is wrong the panel tries the trimmed
text once before reporting a wrong passphrase.

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

## Which vault file

The configured vault is the `vault` field of the selected profile in
LocalState's durable server roster. The default state root is `~/.274bot`.
Fresh-root defaults use `vault` for `local-274`, `vault-289` for `local-289`,
`vault-377` for `local-377`, `vault-prod` for `rs2b2t`, and
`vault-fr-r377-beta` for `fr-r377-beta`. Inspect the saved mapping at any time
with `state servers show` (read-only).

`--vault` is not available in this build. Before the first 0.3.0 unlock for this
server, move or copy the legacy vault to
`<state-root>/<vault-component>` (default root: `~/.274bot`). Do not overwrite
another vault. Find the component in the fresh-root defaults, `state servers show`,
or the `vault` field in the original `servers.json` or its private `.migrated`
copy. Stop 274bot before moving the file. If the destination exists, back up
both vaults privately and choose which one to import. This remedy applies only
before the first import. Do not replace or reopen a completed vault binding.

Server-to-vault bindings are permanent once created in S8. The roster editor
(`state servers`, panel **Servers**, TUI **Servers**) refuses changing a bound
server's vault component or case-insensitive namespace, and refuses reusing a
vault component bound to another server even after that server is removed.
Removing a bound server preserves its binding and profile rows; re-adding it
requires the same vault component (otherwise add a new server name and vault
component). Normal product launchers do not accept a vault-path override.
Test-support builds expose it only for disposable e2e fixtures. Profiles must
name distinct vault files, so profiles never share storage implicitly.
## Importing profiles from rs2b0t

rs2b0t's Settings can export its unlocked profile vault, with each account's
saved settings, as `rs2b0t-profiles.json`. 274bot imports that file: the
panel's **Profiles** window has **Import from rs2b0t…**, and the TUI command
palette has **Import rs2b0t profiles…**. Both need the vault unlocked, ask
for the file's path, and show a preview before anything is written. Nothing
is saved until you confirm. The shared flow is
`frontend_core::rs2b0t_import`.

The importer reads only rs2b0t's own export: kind
`rs2b0t-multibox-profiles`, version 1. It refuses any other kind or version,
or a malformed file, with a message that says why. Like rs2b0t, it accepts a
file without the `storage` part. Errors never quote the file: a bad profile
entry is named by its number, and a bad `kind` or `v` by the type of value
found (for example "an object"), so a password in a damaged file never
appears in one. The path must name a regular file (a FIFO, device or
directory is refused before anything is read), and at most 16 MiB is read
from it, whatever its size claimed.

| rs2b0t | 274bot |
| --- | --- |
| username, password | a vault profile; the password goes only into the encrypted vault |
| world (1–3) | the profile's world (`wN`), used when the server's world list offers it |
| the file's profile order | new profiles take the end of the saved **Load order** in the file's sequence (into an empty vault, that sequence is the order); a profile already in the vault keeps its place, overwritten or skipped |
| tab, tab list, active tab | not imported: 274bot has no profile tabs or groups |
| selected script | the profile's script, when your registered rs2b0t catalog has a script of that name that can run; a dimmed or unknown script is reported |
| script parameters (`set:<Script>:<key>`) | that script's per-profile parameters, read the way rs2b0t reads them: numbers and tile coordinates with JavaScript's `Number()` (so `0x10` is 16 and an empty value is 0), then limited to the parameter's range; options matched without case. A parameter the catalog script doesn't have, or a value rs2b0t would have replaced with the default, is reported |
| Global `lampSkill` | genie lamp skill |
| Global `clueDuelPartner` | clue duel partner |
| `rendererEnabled` | `0` sets the profile's display to Off; `1` turns an Off display back on |
| Global `bankCommonJunk` | not imported: 274bot has no account-wide default for this parameter |
| Global `useZanarisBank`, `useMageBank` | not imported: Zanaris requires Lost City completion, and eligibility for Mage Arena follows the account's wilderness permission. Compatibility `Banking.open` can select and walk to an eligible bank through wilderness even when the account setting is off. |
| Global `runAuto`, `runEnergyMin` | not imported: 274bot turns run back on by itself at 20% energy (scripts may change that); there is no per-account setting |
| Global `navTeleports` | not imported: one app-wide switch in Nav config |
| Global `navPathStallTicks`, `navPathDeviation` | not imported: no such setting |
| Global `navCameraFollow` | not imported: app-wide panel Nav setting |
| Global `showNavPath` and the `navPath*` drawing options | not imported: route drawing is one app-wide panel Nav setting in 274bot; `navPathSceneExpand` has no equivalent |
| `MapPicker.*` | not imported: rs2b0t tile-map picker display |
| `Loadouts.sets` | not imported: 274bot keeps one loadout list for all accounts |
| `PriceBooks.books` | not imported: order books are not supported |
| `creds` | not imported: rs2b0t's per-tab login copy (its value is never read or shown) |
| stored settings of an account the file doesn't list | not imported |

An account the 289 client cannot log in with is listed as unusable, with the
reason, and is not imported unless you choose to import those accounts too:
a username longer than 12 characters, an empty password or one longer than
20 characters (the 289 server refuses these at login), or a line break or a
character beyond Latin-1 in either (the login carries one byte per
character). The import never shortens a credential, and never shows one.

An account already in the vault is kept as it is unless you choose
**Overwrite**. Overwrite keeps the profile's uid and every setting the export
doesn't carry (memory mode, auto-login, random events, a world when the
export has none, and the like). It writes the password, the world and each
setting the export carries. A script's parameters are merged one by one: a
parameter in the export replaces the profile's, every other parameter of
that script stays, and a value the import refuses keeps the profile's own.
The preview and the report say, for each parameter, its new value against
the current one (`[new]`, `[unchanged]` or `[was …]`), list the parameters
that stay, and give the current value a refused one keeps.

Importing the same file again writes nothing. Skip leaves every account
untouched, and with Overwrite each account already matches. The preview and
the final report list every item that isn't imported, with the reason. Each
profile the import writes logs one line (profile, create or overwrite,
operation id, old → new). The log line says only whether the password was set
or changed.

No password from the export is shown in what the import writes out. When a
file is previewed, every non-empty password in it is registered for log
redaction (`api::hostlog::register_secret`), including the passwords of
accounts the import skips (already in the vault, listed twice, unusable)
and of a preview you never confirm. Every line of the preview and the
report, every skipped item and its reason, and every refusal is redacted
(`api::hostlog::redact`) when it is built, so a password that turns up in
another field (a tab, the tab list, the active tab, a stored setting's name
or value, a clue duel partner, another account's name) shows as `***`
there. The panel and the TUI show copies of those lines, and every log
message is redacted the same way. A log line's account column is not: it
always names the account the line is about (the slot), so an account whose
name equals some password shows that name there, in the panel's and the
TUI's log views, in copied log lines and in the session log file. The data
written to the vault is the file's, unchanged: a clue duel partner equal to
some password is saved as it is. Like the vault's own passwords, a
registered password stays registered until the process exits.

An account counts as imported only when the import's own write of it is
saved. The report says "not confirmed" for a write that a later save of the
same profile replaced before it was saved, or whose result the session no
longer knows, and lists every failure. The export file holds the passwords in
plain text: only when every account you chose to import is confirmed saved
does the report offer to delete the file, and it asks you to confirm. Loaded
bots use an imported password and world at their next login, and imported
script parameters at their next Start.

The import holds the passwords in `vault::Secret`s. It reads the file into
one buffer sized from the file's length and zeroes it after parsing, checks
the JSON syntax before it builds any value tree, and zeroes every string
value of that tree when it is dropped. It does not reach object keys, a
value that a repeated key in the same object replaced, serde_json's own
scratch buffer for strings with escapes, a buffer the read had to grow
because the file grew meanwhile, or the log-redaction list described below.

The importer keeps owning data behind private fields. Catalog adapters use
`CatalogCard::new` and its accessors. Consumers read skipped items through
`Skipped::item` and `Skipped::reason`. `ImportRun::results` returns borrowed
`AccountResult` views, so reading a settled failure does not copy its text.
`ImportError` is opaque. Its display and `Debug` diagnostics are unchanged, and
`std::error::Error::source` exposes the original I/O cause.

The operator command census records import preview choices and export-deletion
confirmations as application decisions. Confirmed account writes use the
existing `CreateProfile` or `SaveProfile` row, according to the planned account
status. The census describes the existing owner entry points. It does not add
another import execution path.

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
form (used for a new vault). `read_private_file` is the reader for state
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
