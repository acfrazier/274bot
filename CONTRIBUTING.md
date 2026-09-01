# Contributing

274bot is **alpha** (`0.1.0` crate versions, no crates.io publish; git
tag `0.1.1`). This tag is the host + API + nav **execute** surface. The
script *kernel* (Browse / Start / Pause / Stop, JS Load) and WalkTo are
in-tree; honest bot scripts are not this tag (see [CHANGELOG.md](CHANGELOG.md)
for v0.1.2 / v0.1.5 / v0.2.x).

Alpha is not turnkey rs2b2t: you run a local 274 engine and point
`--cache` at a pack. Live cache fetch is a beta goal. PRs that read like
unreviewed model output will be rejected; the product bar is a host that
does not suck.

Product docs: [README.md](README.md), [NOTICE.md](NOTICE.md),
[docs/](docs/README.md). Coding-agent rules: [AGENTS.md](AGENTS.md).

## Prerequisites

- A **local** Lost City 274 engine: game TCP `127.0.0.1:43594`, HTTP
  `/crc` on `:80`. This repo does not ship Jagex assets.
- **`$ENGINE_DIR`**: engine root (default `$HOME/experiments/Server/engine`).
  Pack cache is `$ENGINE_DIR/data/pack/client` (`--cache` overrides).
- RSA: stock LC Server uses the **Java default pair** — no bake. Rotated
  `private.pem` is read at login from `$ENGINE_DIR/data/config/private.pem`
  (or `LOGIN_RSAN` / `LOGIN_RSAE`).
- Nav pack: `$NAV_PACK` or `~/.274bot/274bot.navpack` (`274V` v7), baked
  with `cargo run -p nav --bin nav-pack` over `$ENGINE_DIR/../content/maps`.
  `gates.loc` follows the maps dir's parent. Alpha assumes you already
  have a Server tree. Rebake after this tag (`274V` v7; v6 is `BadVersion`).

## Clone and run

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
git submodule update --init

export BOT_VAULT_PASS=bot
cargo run --release -p panel --bin panel-play
```

`--release` matters: a debug `cargo run` looks frozen. Headed default is
the wgpu GPU renderer in the client submodule; `BOT_CPU=1` is CpuPix3D.

**host-play** upserts named users (`--user test` defaults to `test`/`test`).
**panel-play** does not: an empty first-run vault stays empty until you
Save credentials. `--vault-pass` is a **host-play** flag (same as
`BOT_VAULT_PASS`); the panel reads `BOT_VAULT_PASS` or the in-window
prompt.

**tui-play** (`cargo run --release -p tui --bin tui-play`) is the
headless operator panel: ratatui + crossterm, same flags and vault as
`panel-play`, slots spawn raster Off (no GPU). `--live script_<name>`
runs the same scenario harness as `panel-play --live`.

## Toolchain

Rust **1.98.0** for this tag (`rust-toolchain.toml` in this repo and in
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

Live harnesses need the engine. Failures print `FAIL:` and `exit(1)`.
Wait `ingame && scene_state == 2`. Quiet unless `BOT_DEBUG=1`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
LIVE=1 cargo run --release -p tui --bin tui-play -- --live script_nav_door
```

Nav / panel / scenario twins live in `crates/e2e`. Login / RSS / null-raster
twins live in `crates/host-play`.

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

No dummy tick-end opcode. No deep-copy of the world every read. Nav and
the MultiBox wall are in-tree. Script *ports* of farming bots are not
this tag.

## License

MIT ([LICENSE](LICENSE)). Attribution in [NOTICE.md](NOTICE.md).
