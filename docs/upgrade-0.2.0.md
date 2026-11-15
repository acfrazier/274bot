# Upgrading 274bot

## Saved data in 0.2.9

Version 0.2.9 reads formats used by 0.2.8 and 0.2.9.
A file with a supported format loads even if an older release wrote it.
There is no writer-release marker.
A shape that 0.2.8 loaded without asking you is a supported format, even when
an older release wrote it and 0.2.8 never rewrote it. Examples: a bank memory
file without a game-content record, a vault profile without a setting that a
later release added, and loadouts with worn gear as a plain list.
Formats that 0.2.8 did not load, or converted at once, no longer load. In 0.2.9
these are a bank memory file without `schema_version`, and `worlds.json` when
there is no `servers.json`.

Before normal startup, the application checks preferences, script settings,
loadouts, all account hints and server settings, including normally lazy stores.
The panel and TUI show every refused path and ask once whether to move the files
aside. Yes moves each file to a sibling named `<name>.pre-0.2.8`, with a numbered
suffix if needed. Existing backups are never overwritten. Startup then treats
the original paths as absent. No quits without changing any file.

The vault is checked after decryption at unlock. 0.2.9 reads every vault that
0.2.8 read. A later release that stops reading an old vault shape shows the
same choice at unlock. Yes moves the vault aside and returns to empty-vault
setup. No leaves it unchanged and quits.

Noninteractive live, smoke and agent runs do not ask or move files.
The `host-play` command is always noninteractive for this decision.
These runs quit with a typed error that names each refused file.
Ordinary malformed-file errors are not classified as old formats.

## Changes from 0.1.9.1 to 0.2.0 Beta 1

The following sections describe that older release, not the 0.2.9 format policy.

### Server profiles

`--profile NAME` now picks an entry from `~/.274bot/servers.json`, created on
first use with `local-274`, `local-289` and `rs2b2t`. `--rs2b2t` selects the
public profile (`public-289` still works as a name); `BOT_SERVER_PROFILE` is
the environment form. `--prod` and `BOT_TARGET` were removed and now error —
use `--profile` instead.

A valid `~/.274bot/worlds.json` is imported once into the `rs2b2t` roster;
after that, edit `servers.json` (later changes to `worlds.json` are ignored).
`--user` creates missing accounts with a fresh random game password.

### Vault passphrase

`BOT_VAULT_PASS` and `--vault-pass` were removed: anything on the command line
or in the environment is visible to other users. `host-play` and `tui-play`
ask at a hidden terminal prompt (twice for a new vault); pipe it with
`--vault-pass-stdin`; the panel asks in its unlock window.

Unlocking uses the passphrase exactly as typed. Vaults made by the 0.1.9.1
panel still open in the panel; at the `host-play` / `tui-play` prompt, type
the passphrase without surrounding spaces. A new vault needs a passphrase that
is not blank after trimming; strength is your choice.

A first run of `tui-play` with no `--user` now stops with "vault has no
profiles" instead of seeding a `test` profile; pass `--user NAME` to create
the first one.

### Navigation packs

The pack format is now v16. Packs you baked yourself with 0.1.9.1 or earlier
are refused with a message to rebake them with
`nav-pack`. The bundled pack is rebuilt for you.

### Local engines

The bot no longer guesses where a local game engine is. Give it with
`--engine`, the `ENGINE_DIR` environment variable, or the profile's
`login_key.engine_dir` in `servers.json`; without one, a local profile stops
with a message saying how to set it. Public (rs2b2t) profiles need nothing.

### Graphics

On Windows the panel tries Vulkan first and falls back to Direct3D 12.
Windows and macOS prefer the power-saving GPU. From 0.2.5, the Linux
panel follows the system's choice of graphics chip instead.
`WGPU_BACKEND` and `WGPU_POWER_PREF` override these defaults (see
`FIRST-START.md`). `WGPU_POWER_PREF=high` can make startup stall for
minutes on some laptops.
