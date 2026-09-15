# Contributing

274bot is **alpha** (`0.1.0` crate versions, no crates.io publish; public
history tags `0.1.x`). The host, API, nav execute, random-event guardian,
headless TUI, panel, script kernel (Browse / Start / Pause / Stop / Load /
Reload, bulk Start all / Stop all, catalog refresh), and revision profiles
are in-tree. Catalog compatibility remains **partial** — unsupported
helpers fail closed; do not treat a green unit job as full script
qualification. See [CHANGELOG.md](CHANGELOG.md).

Alpha is not turnkey public-world automation: you run a **local** engine for
the revision you care about and point `--cache` / `ENGINE_DIR` at a pack.
The public world is built in as profile `public-289` (`--prod` /
`BOT_TARGET=prod|live`): WSS/HTTPS on **`w1.rs2b2t.com:443`** with the baked
public RSA. Local is the tested path; Cargo `TARGET` is the rustc triple,
not a world switch. This is **not** Jagex and not a hosted wall — there is
**no public-world CI** and no asset server. PRs that read like unreviewed
model output will be rejected; the product bar is a host that does not suck.

Product docs: [README.md](README.md), [NOTICE.md](NOTICE.md),
[FIRST-START.md](FIRST-START.md), [docs/](docs/README.md).

## Prerequisites

- A **local** Lost City engine for the profile under test:
  - `local-274`: TCP `127.0.0.1:43594`, HTTP `/crc` on `:80`
  - `local-289`: TCP `127.0.0.1:44594`, HTTP `/crc` on `:1080`
  This repo does not ship Jagex assets.
- **`$ENGINE_DIR`** / `--engine`: engine root (defaults
  `$HOME/experiments/Server/engine` for 274,
  `$HOME/experiments/lostcity-289/engine` for 289). Pack cache is
  `$ENGINE_DIR/data/pack/client` (`--cache` overrides).
- RSA: stock LC Server uses the **Java default pair** — no bake. Rotated
  `private.pem` is read at login from `$ENGINE_DIR/data/config/private.pem`
  (or `LOGIN_RSAN` / `LOGIN_RSAE`).
- Nav pack: an ordinary build bakes and stages the selected **build-time**
  revision’s pack and flags next to the binary
  (`target/<profile>/nav/<revision>/`, revision **289** by default,
  `BOT_NAV_REVISION=274` for 274), so the app boots with a bound nav world
  and no manual step. Missing canonical inputs fail the build:
  `BOT_NAV_BUILD=skip` opts out. `nav-pack` stays for custom bakes
  (`$NAV_PACK` or `~/.274bot/274bot.navpack`, `274V` v8; v7 is
  `BadVersion`). Details: [docs/api/nav.md](docs/api/nav.md).

## Clone and run

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
git submodule update --init

export BOT_VAULT_PASS=bot
cargo run --release -p panel --bin panel-play -- --profile local-289
```

`--release` matters: a debug `cargo run` looks frozen. Headed default is
the wgpu GPU renderer in the client submodule; `BOT_CPU=1` is CpuPix3D.

**host-play** upserts named users (`--user test` defaults to `test`/`test`).
**panel-play** does not: an empty first-run vault stays empty until you
Save credentials. `--vault-pass` is a **host-play** flag (same as
`BOT_VAULT_PASS`); the panel reads `BOT_VAULT_PASS` or the in-window
prompt.

**tui-play** (`cargo run --release -p tui --bin tui-play -- --profile local-289`)
is the headless operator panel: ratatui + crossterm, same profile flags and
vault layout as `panel-play`, slots spawn raster Off (no GPU).
`--live script_<name>` runs the same scenario harness as
`panel-play --live`.

Shared profile flags (all three binaries):
`--profile local-274|local-289|public-289`, `--revision 274|289`, `--prod`,
`--engine`, `--cache`, `--vault`, `--catalog`, and related overrides. See
[README.md](README.md) and [FIRST-START.md](FIRST-START.md).

## Toolchain

Rust **1.98.0** (`rust-toolchain.toml` in this repo and in
`vendor/fr-client-rust`). GitHub Actions uses the same number, not
`@stable`. Bump both files and both workflows together — do not
`rustup update` into a new clippy `-D` set mid-tag.

## Tests

**Local CI** is this checkout: host workspace and the vendored client
workspace, same bar. Client is a path dep (not a 274bot workspace member),
so that is two cargo manifests — not a second product. Never sets
`LIVE=1`.

```bash
cargo fmt --all -- --check
cargo fmt --all --manifest-path vendor/fr-client-rust/Cargo.toml -- --check

cargo clippy --workspace --all-targets --no-deps -- -D warnings
cargo clippy --manifest-path vendor/fr-client-rust/Cargo.toml --workspace --all-targets --no-deps -- -D warnings

cargo test --workspace
cargo test --manifest-path vendor/fr-client-rust/Cargo.toml --workspace
```

GitHub Actions runs the same two manifests after installing ALSA + X11
headers (`libasound2-dev` — panel pulls client `audio` / cpal). It is still
a **subset**: `SKIP_GPU=1` (no adapter on those VMs) and never `LIVE=1`.
A green GH job is not a headed or engine pass.

Live harnesses need the engine for the profile under test. Failures print
`FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`. Quiet unless
`BOT_DEBUG=1`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
LIVE=1 cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_nav_door
```

Nav / panel / scenario twins live in `crates/e2e`. Login / RSS / null-raster
twins live in `crates/host-play`. Ordered multi-case native runs:
[docs/e2e-suite.md](docs/e2e-suite.md). Memory fleet harness:
[docs/harness.md](docs/harness.md).

`cargo test` here does not run Fairy-Ring client integration tests.

## Client submodule

`vendor/fr-client-rust` is [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost)
branch **`r274-bh-modular`**. Public surface for that tree is its own
`README.md` / `NOTICE.md` (this is a **bot product** client, not Fairy
Ring). Do **not** push `Fairy-Ring/FR-client-rust`. Do not add a bot
action API inside `client`. Do not put 274bot crates in the client repo.
Packet timing and `doAction` stay Java-shaped.

Wiring `client` compiles the **lib**. `r274-modular` is the same refactor
without bot-host hooks; `r274-bothost` is the pre-modular fork.

## Scope (do not invent)

No dummy tick-end opcode. No deep-copy of the world every read. No extra
JSON JS↔Rust host wire (FlatBuffers only). Nav, the MultiBox wall, the
random-event guardian, revision profiles, and `tui-play` are in-tree.
Do not claim unfinished canvas/mouse/boost surfaces or full catalog
parity in product docs or PR descriptions.

## License

MIT ([LICENSE](LICENSE)). Attribution in [NOTICE.md](NOTICE.md).
