# First start

274bot talks to a **local Lost City engine** first. Choose an immutable
**server profile** for the process (`local-274`, `local-289`, or
`public-289`). Public `public-289` / `--prod` uses WSS + HTTPS on
the worlds in `~/.274bot/worlds.json` (default w1/w2 on port 443)
and requires separate live login verification.
This repo does not ship Jagex assets and does not promise
automatic asset distribution beyond the client’s ordinary `/crc` + jag
fetch into the configured cache/unpack directory.

## Downloaded Alpha 3 packages

The macOS app opens on **public-289**. Standalone binaries use
`./panel-play --profile public-289` or `./tui-play --profile public-289`
(`.exe` on Windows). Keep the adjacent `nav/` directory; the macOS app
contains its own resources. Initial distributed navigation targets the public
289 cache. A custom local engine with different cache bytes needs a matching
source build or explicit external navigation pack.

No Rust toolchain or local engine is needed for the public package. Game assets
are fetched from the configured public server, and catalog scripts still come
from your chosen rs2b0t checkout. Public login requires your own account.

## Toolchain

- Rust **1.98.0** (`rust-toolchain.toml` in this repo and in
  `vendor/fr-client-rust`).
- Git with submodules:
  `git clone --recurse-submodules https://github.com/acfrazier/274bot.git`
- A local engine for the revision you will run:
  - **274:** game TCP `:43594`, HTTP `/crc` on `:80` (default engine root
    `$HOME/experiments/Server/engine`)
  - **289:** game TCP `:44594`, HTTP `/crc` on `:1080` (default engine root
    `$HOME/experiments/lostcity-289/engine`)
- Point **`$ENGINE_DIR`** or `--engine` at that engine root when it is not
  the default.

## Cache and nav pack

Pack cache for **local** is `$ENGINE_DIR/data/pack/client` (`--cache`
overrides). First `maininit` GETs `/crc` and jags from the engine HTTP into
that directory; later boots reuse disk.

If the cache directory is empty, run
[`scripts/fetch-cache.sh`](scripts/fetch-cache.sh) (copies from
`$ENGINE_DIR` if files exist, otherwise tells you to boot once against the
local engine). That script does not download assets from the public
internet for you.

**Prod / public-289** downloads `/crc` and jags into
**`~/.274bot/unpack-289`** by default. The local-274 unpack default is
`~/.274bot/unpack`; there is no public-274 profile. Versioned model/anim
snapshots from `unpack-cache` live in a child folder named the first 8 hex
bytes of SHA-256(`versionlist`) — not the `/crc` table.

Nav pack: an ordinary application build bakes and stages the selected
**build-time** revision’s pack next to the binary
(`target/<profile>/nav/289/` by default via `BOT_NAV_REVISION`, default
**289**), so WalkTo has a bound nav world with no manual step. Missing
canonical inputs fail the build (`BOT_NAV_BUILD=skip` opts out;
`BOT_NAV_REVISION=274` builds 274). Runtime profile revision and
build-time nav revision should match the world you play. `nav-pack` stays
for custom bakes: `cargo run -p nav --bin nav-pack` over the content maps
tree (output `$NAV_PACK` or `~/.274bot/274bot.navpack`, magic `274V`,
version byte **10**). Details: [docs/api/nav.md](docs/api/nav.md).

Catalog scripts (optional): set **`$RS2B0T`** or pass `--catalog` to an
rs2b0t checkout so panel/TUI can Browse/Start catalog cards.

## Local (TCP)

```bash
export BOT_VAULT_PASS=bot
# 289 example — adjust ENGINE_DIR if your tree is not the default
export ENGINE_DIR="${ENGINE_DIR:-$HOME/experiments/lostcity-289/engine}"
# optional: export RS2B0T=/path/to/rs2b0t

cargo run --release -p panel --bin panel-play -- --profile local-289
# headless twin:
cargo run --release -p tui --bin tui-play -- --profile local-289
# 274:
# BOT_NAV_REVISION=274 ENGINE_DIR="$HOME/experiments/Server/engine" cargo run --release -p panel --bin panel-play -- --profile local-274
```

Live harness (FAIL + exit 1, waits `ingame && scene_state==2`):

```bash
cargo run --release -p panel --bin panel-play -- --profile local-289 --live script_bone_burier
cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_bone_burier
```

`CLIENT_CHEAT`, TutSkip, mainland hop, and the debug dest strip are
**local-only**.

Without `--profile`, plain local still resolves to **274** (legacy
default). Prefer naming the profile.

## Prod (WSS + HTTPS)

After local golds work. Use a real password (not username-as-password). Do
not expect `give` / TutSkip / `tele` to work on the public world.

```bash
export BOT_VAULT_PASS=bot

cargo run --release -p host-play -- --profile public-289 --user YOUR_NAME
cargo run --release -p panel --bin panel-play -- --profile public-289
cargo run --release -p tui --bin tui-play -- --profile public-289
# or: --prod / BOT_TARGET=prod (selects public-289 when revision is unset)
```

`public-289` fetches `/crc` and jags over **HTTPS :443** into the shared
`~/.274bot/unpack-289` directory (tries the next listed asset world when
the first is unavailable). The game stream uses **WSS** (`binary`
subprotocol) on the account's selected world. First launch writes
`~/.274bot/worlds.json` with ordered w1/w2 endpoints and node ids; edit it
and restart to change the list. A malformed file refuses startup. Panel
Profiles edits each account's world (`auto`, w1 or w2). Auto retries the
next world promptly on \"world full\"; pinned accounts do not move.
`tui-play --world 2` chooses w2 only for accounts stored as auto.
Local stays TCP on the profile's game/asset ports.
Vault defaults: `~/.274bot/vault-prod` for public-289; local-274
`~/.274bot/vault`; local-289 `~/.274bot/vault-289`. `--vault PATH` still
wins. Unit tests do not verify public login.
