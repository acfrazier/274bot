# First start

274bot talks to a **local Lost City engine** first. Public `--prod` is WSS + HTTPS on `w1.rs2b2t.com` and is a ship-gate login, not a substitute for local tests. This repo does not ship Jagex assets.

## Toolchain

- Rust **1.98** (see `rust-toolchain.toml` if present, else `rustc --version`).
- Git with submodules: `git clone --recurse-submodules https://github.com/acfrazier/274bot.git`
- A local 274 engine: game TCP `:43594`, HTTP `/crc` on `:80`.
- Point **`$ENGINE_DIR`** at the engine root (default `$HOME/experiments/Server/engine`).

## Cache and nav pack

Pack cache is `$ENGINE_DIR/data/pack/client` (`--cache` overrides). First `maininit` GETs `/crc` and jags from the engine HTTP into that directory; later boots reuse disk.

If the cache directory is empty, run [`scripts/fetch-cache.sh`](scripts/fetch-cache.sh) (copies from `$ENGINE_DIR` if files exist, otherwise tells you to boot once against the local engine). Prod boots with no local engine fetch `/crc` and jags over **HTTPS :443** on first `maininit`.

Nav pack: `cargo run -p nav --bin nav-pack` over `$ENGINE_DIR/../content/maps`. Output is `$NAV_PACK` or `~/.274bot/274bot.navpack` (magic `274V`, version byte **8**). Rebake after a version bump.

Catalog scripts (optional): set **`$RS2B0T`** to an rs2b0t checkout so panel/TUI can Start catalog cards.

## Local (TCP)

```bash
export BOT_VAULT_PASS=bot
export ENGINE_DIR="${ENGINE_DIR:-$HOME/experiments/Server/engine}"
# optional: export RS2B0T=/path/to/rs2b0t

cargo run --release -p panel --bin panel-play
# headless twin:
cargo run --release -p tui --bin tui-play
```

Live harness (FAIL + exit 1, waits `ingame && scene_state==2`):

```bash
cargo run --release -p panel --bin panel-play -- --live script_bone_burier
cargo run --release -p tui --bin tui-play -- --live script_bone_burier
```

`CLIENT_CHEAT`, TutSkip, mainland hop, and the debug dest strip are **local-only**.

## Prod (WSS + HTTPS)

After local golds work. Use a real password (not username-as-password). Do not expect `give` / TutSkip / `tele` to work on the public world.

```bash
export BOT_VAULT_PASS=bot

cargo run --release -p host-play -- --prod --user YOUR_NAME
cargo run --release -p panel --bin panel-play -- --prod
cargo run --release -p tui --bin tui-play -- --prod
# or: BOT_TARGET=prod
```

Prod fetches `/crc` and jags over **HTTPS :443** and the game stream is **WSS** (`binary` subprotocol) on `w1.rs2b2t.com`. Local stays TCP `:43594` + HTTP `:80`. Live `--prod` login is the ship gate, not a unit substitute.
