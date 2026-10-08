# First start

274bot talks to a **local Lost City engine** first. Choose one immutable
entry from `~/.274bot/servers.json` for the process. First use creates
`local-274`, `local-289`, and `rs2b2t`; `public-289` remains an alias for
`rs2b2t`. The public profile uses WSS + HTTPS on its rs2b2t roster and
requires separate live login verification.
This repo does not ship Jagex assets and does not promise
automatic asset distribution beyond the client’s ordinary `/crc` + jag
fetch into the configured cache/unpack directory.

## Downloaded packages

The macOS app opens on **rs2b2t**. Standalone binaries (macOS, Windows and
Linux) use `./panel-play --profile rs2b2t` or `./tui-play --profile
rs2b2t` (`.exe` on Windows). Keep the adjacent `nav/` and `map/`
directories; the macOS app contains its own resources. Initial distributed
navigation and WalkTo map terrain target the public 289 cache: the first
WalkTo open installs the shipped terrain when your client cache matches it,
and otherwise asks before baking terrain locally. A custom local engine with
different cache bytes needs a matching source build or explicit external
navigation pack.

On Linux (glibc 2.39 or newer), `panel-play` needs an X11 or Wayland session
and Vulkan: the Vulkan loader plus a GPU driver, or Mesa lavapipe for software
rendering. `BOT_CPU=1` draws the game view on the CPU; the window still uses
Vulkan. On Ubuntu 24.04 the libraries come from `libasound2t64 libssl3t64
libx11-6 libx11-xcb1 libxcursor1 libxi6 libxkbcommon0 libxkbcommon-x11-0
libwayland-client0 libwayland-cursor0 libvulkan1 mesa-vulkan-drivers`.
`tui-play` needs only glibc and OpenSSL 3 (`libssl3t64`).

On macOS, `panel-play` and `tui-play` need Apple Silicon running macOS 11.0 or later.

On Windows, `panel-play` uses Vulkan and falls back to Direct3D 12 when
Vulkan has no working GPU. It prefers the power-saving GPU, which on a laptop
with two GPUs is the integrated one driving the screen. To choose otherwise,
set `WGPU_BACKEND=dx12` (or `vulkan`) and `WGPU_POWER_PREF=high` before
starting it. On some laptops `WGPU_POWER_PREF=high` makes the window stall
for minutes at startup while the dedicated GPU's Direct3D driver loads. Windows also needs the Microsoft Visual C++ x64 runtime (`VCRUNTIME140.dll` and `VCRUNTIME140_1.dll`).

No Rust toolchain or local engine is needed for the public package. Game assets
are fetched from the configured public server, and catalog scripts still come
from your chosen rs2b0t checkout. Public login requires your own account.

Upgrading from 0.1.9.1: read "Upgrading from 0.1.9.1" in the release notes
(the same text is in `CHANGELOG.md` and `docs/upgrade-0.2.0.md` in the
repository). Server profiles, the vault passphrase prompt, local engine paths,
self-baked navigation packs and the graphics defaults changed.

## Toolchain

- Rust **1.98.0** (`rust-toolchain.toml` in this repo and in
  `vendor/fr-client-rust`).
- Git with submodules:
  `git clone --recurse-submodules https://github.com/acfrazier/274bot.git`
- A local engine for the revision you will run:
  - **274:** game TCP `:43594`, HTTP `/crc` on `:80`
  - **289:** game TCP `:44594`, HTTP `/crc` on `:1080`
- Set `ENGINE_DIR`, pass `--engine`, or configure `login_key.engine_dir` in
  `~/.274bot/servers.json`. Resolution order is `--engine`, `ENGINE_DIR`, then
  the saved profile path. New `servers.json` files leave the local engine path
  unset; selecting a local profile without one fails fast with setup guidance.

## Cache and nav pack

The **local** packed-cache source is `$ENGINE_DIR/data/pack/client`
(`--cache` overrides). Ordinary preparation negotiates `/crc`, checks packed
JAGs, and downloads missing assets from the selected engine.

If the cache directory is empty, run
[`scripts/fetch-cache.sh`](scripts/fetch-cache.sh) (copies from
`$ENGINE_DIR` if files exist, otherwise tells you to boot once against the
local engine). That script does not download assets from the public
internet for you.

The **rs2b2t** snapshot root is **`~/.274bot/unpack-289`** by default.
The local-274 default is `~/.274bot/unpack`; there is no public 274 profile.
A successful ordinary first preparation retains all eight checked packed
JAGs and the complete decoded model, animation, map and MIDI archives in
that root. Retained selections are namespaced by revision, the negotiated
`/crc` table, and the full raw `versionlist` SHA-256.

Every later preparation still negotiates `/crc`, validates the selected
packed JAGs and snapshot integrity, and recomputes the canonical decoded
identity from the actual bytes. Matching retained assets avoid another
full download or decode. Missing, incomplete or corrupt assets are repaired;
changed negotiation/version inputs select a new snapshot. Each prepared
profile uses its own immutable `.runtime-*` copy, removed when its last
owner is dropped; retained snapshots survive ordinary process exit.
Dead runtime copies and interrupted-publication staging folders left by crashes
or explicit exits are swept on the next preparation without deleting a live
process's copy.

The three most recently used verified snapshot namespaces per revision are
kept, with recency refreshed on every successful preparation, including reuse.
This lets you switch between servers without downloading their assets again,
while bounding abandoned copies. A copy a live process may still read or write
is never removed, even outside that bound; liveness uses a conservative PID
probe, so an unknown result counts as alive. Snapshots in the old layout
directly under `unpack-289/<version>/` are neither read nor removed.
Reusing individual files across negotiation keys is out of scope: a new or
evicted negotiation key refills from validated entries.

If saving retained assets fails (for example, the destination is read-only),
startup warns once and continues using the verified private runtime copy.
Without a saved snapshot, the next launch will need to download the assets again.

The standalone `unpack-cache` layout remains separate: its child folder is
the first 8 bytes of SHA-256(`versionlist`), encoded as 16 hex characters,
not a hash of the `/crc` table.

Nav pack: an ordinary application build bakes and stages the selected
**build-time** revision’s pack next to the binary
(`target/<profile>/nav/289/` by default via `BOT_NAV_REVISION`, default
**289**), so WalkTo has a bound nav world with no manual step. Missing
canonical inputs fail the build (`BOT_NAV_BUILD=skip` opts out;
`BOT_NAV_REVISION=274` builds 274). Runtime profile revision and
build-time nav revision should match the world you play. `nav-pack` stays
for custom bakes: `cargo run -p nav --bin nav-pack` over the content maps
tree (output `$NAV_PACK` or `~/.274bot/274bot.navpack`, magic `274V`,
version byte **16**; v15 and older are `BadVersion` and must be rebaked).
Details: [docs/api/nav.md](docs/api/nav.md).

Catalog scripts (optional): set **`$RS2B0T`** or pass `--catalog` to an
rs2b0t checkout so panel/TUI can Browse/Start catalog cards.

## Local (TCP)

```bash
# The vault passphrase is typed at the prompt (panel window / terminal); it is
# never an environment variable or a flag. See docs/api/vault.md.
# Set ENGINE_DIR to the engine root for the selected local revision.
export ENGINE_DIR=/absolute/path/to/engine
# optional: export RS2B0T=/path/to/rs2b0t

cargo run --release -p panel --bin panel-play -- --profile local-289
# headless twin:
cargo run --release -p tui --bin tui-play -- --profile local-289
# 274 (set ENGINE_DIR to the 274 engine root):
# BOT_NAV_REVISION=274 cargo run --release -p panel --bin panel-play -- --profile local-274
```

A first `tui-play` run with no `--user` stops with "vault has no profiles"
instead of seeding an account: pass `--user NAME` to create the first one
(`host-play --user` also creates missing accounts with a fresh random game
password). The panel never auto-creates an account: an empty first-run vault
stays empty until you type a username/password and Save.

Live harness (FAIL + exit 1, waits `ingame && scene_state==2`):

```bash
cargo run --release -p panel --bin panel-play -- --profile local-289 --live script_bone_burier
cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_bone_burier
```

`CLIENT_CHEAT`, TutSkip, mainland hop, and the debug dest strip are
**local-only**.

Without `--profile`, plain local still resolves to **274** (legacy
default). Prefer naming the profile.

## rs2b2t (WSS + HTTPS)

After local golds work. Use a real password (not username-as-password). Do
not expect `give` / TutSkip / `tele` to work on the public world.

```bash
# host-play and tui-play ask for the vault passphrase on the terminal; the panel
# asks in its unlock window. A new vault needs a non-empty passphrase after
# trimming surrounding whitespace; strength is the user's choice.

cargo run --release -p host-play -- --rs2b2t --user YOUR_NAME
cargo run --release -p panel --bin panel-play -- --rs2b2t
cargo run --release -p tui --bin tui-play -- --rs2b2t
# Equivalent: --profile rs2b2t (public-289 is a compatibility name alias)
```

`rs2b2t` fetches `/crc` and jags over **HTTPS :443** into the shared
`~/.274bot/unpack-289` directory (tries the next listed asset world when
the first is unavailable). The game stream uses **WSS** (`binary`
subprotocol) on the account's selected rs2b2t world (w1/w2 by default).
Panel Profiles selects each account's world (`auto` or a listed world). Auto retries the
next world promptly on \"world full\"; pinned accounts do not move.
`tui-play --world 2` chooses w2 only for accounts stored as auto.
Local stays TCP on the profile's game/asset ports.
Vault defaults: `~/.274bot/vault-prod` for `rs2b2t`; local-274 uses
`~/.274bot/vault`; local-289 uses `~/.274bot/vault-289`. `--vault PATH` still
wins. Unit tests do not verify public login.
