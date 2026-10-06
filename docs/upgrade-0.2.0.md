# Upgrading to 0.2.0 Beta 1

From 0.1.9.1. One section per breaking change; everything else keeps working.

## Server profiles

`--profile NAME` now picks an entry from `~/.274bot/servers.json`, created on
first use with `local-274`, `local-289` and `rs2b2t`. `--rs2b2t` selects the
public profile (`public-289` still works as a name); `BOT_SERVER_PROFILE` is
the environment form. `--prod` and `BOT_TARGET` were removed and now error —
use `--profile` instead.

A valid `~/.274bot/worlds.json` is imported once into the `rs2b2t` roster;
after that, edit `servers.json` (later changes to `worlds.json` are ignored).
`--user` creates missing accounts with a fresh random game password.

## Vault passphrase

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

## Navigation packs

The pack format is now v16. Packs you baked yourself with 0.1.9.1 or earlier
are refused with a message to rebake them with
`nav-pack`. The bundled pack is rebuilt for you.

## Local engines

The bot no longer guesses where a local game engine is. Give it with
`--engine`, the `ENGINE_DIR` environment variable, or the profile's
`engine_dir` in `servers.json`; without one, a local profile stops with a
message saying how to set it. Public (rs2b2t) profiles need nothing.

## Graphics

On Windows the panel tries Vulkan first and falls back to Direct3D 12. On
every platform the window now prefers the power-saving GPU. `WGPU_BACKEND` and
`WGPU_POWER_PREF` override this (see `FIRST-START.md`); `WGPU_POWER_PREF=high`
can make startup stall for minutes on some laptops.
