# 274bot

Headless **RuneScape revision 274** (~2004) **bot host**: N clients in one process, shared type tables, a login FIFO, an encrypted vault, an agent API, a native panel, and whole-world nav.

| | |
|--|--|
| **Revision** | RuneScape 274 (~2004) |
| **Engine** | A **local** Lost City engine (game `43594`, HTTP `/crc` on `:80`) |
| **Client** | submodule [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) `r274-bh-modular` (bot-host fork of the modularized Fairy-Ring 274 client) |
| **This repo** | [acfrazier/274bot](https://github.com/acfrazier/274bot) — **MIT** ([LICENSE](LICENSE), [NOTICE.md](NOTICE.md)) |

## What it is

A Rust bot host over the 274 client. One OS thread per `Client` on a 20 ms loop, shared unpacked type tables, a login FIFO, AES-256-GCM vaulted profiles, and an agent API (snapshot → query → interact → settle). **`panel-play`** is the operator window (dear-app/ImGui): credentials, status, WalkTo picker, game blit, click-through capture, MultiBox rail/grid, `--live` harness.

The headed client draws with a **wgpu GPU** renderer in the submodule (CPU Pix3D is `BOT_CPU=1`). Nav is a baked collision + transport pack, Dijkstra router, and pollable `Traveller::follow` driven from WalkTo and from scripts. Compiled script cards tick on the `PLAYER_INFO` edge; Load’d JS is isolate + stub prelude.

## What it is not

- No hosted product, no official anything: **not** Jagex, **not** official Lost City, **not** Fairy Ring, **not** a file-port of rs2b0t (rs2b0t’s chrome is mimicked in the panel; the code is our own).
- Do **not** push `Fairy-Ring/FR-client-rust`. The live client branch is **`r274-bh-modular`**. `r274-modular` is the same refactor without bot-host hooks. `r274-bothost` is the pre-modular fork.
- Still no bot action API inside `client`. Packet timing and `doAction` stay Java-shaped. Client MIT/NOTICE stay in the submodule.
- This tree ships **no Jagex assets**; you bring your own local engine and pack cache.

## Quick start

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
```

You need a **local 274 engine** (game `43594`, HTTP `/crc` on `:80`) and the pack cache. This repo does **not** ship or download Jagex assets. Default cache path: `$HOME/experiments/Server/engine/data/pack/client` (override with `--cache`). On first `maininit` the client GETs `/crc` and jag files from the engine HTTP into that directory; later boots reuse the files on disk. Contributors: run the engine, point `--cache` at its `data/pack/client`, `git submodule update --init`, bake RSA from the engine `private.pem` (`LOGIN_RSAN` / `vendor/fr-client-rust/tools/redeploy.sh`), then `cargo run --release -p panel --bin panel-play`. Nav pack is a separate bake (`cargo run -p nav --bin nav-pack`) over the engine `maps/`.

**RSA bake:** cargo’s `TARGET` is the rustc triple, so the live/prod switch is **`BOT_TARGET`**. Local (default) still uses `LOGIN_RSAN` / `LOGIN_RSAE` or `vendor/fr-client-rust/tools/redeploy.sh` from the engine `private.pem`. Live rs2b2t is **not** that pem — scrape the public modulus from the served client:

```bash
MOD=$(curl -s --max-time 15 https://w1.rs2b2t.com/client/client.js \
  | grep -oE '[0-9]+' | awk 'length($0) >= 250 { print; exit }')
BOT_TARGET=live LIVE_RSAN="$MOD" cargo build -p client
# host-play live host: TARGET=live (runtime env, not the bake triple)
```

Login response **6** retries once after `/loginkey` then that `client.js` scrape. `prod` bake requires `PROD_RSAN`. TCP `w1.rs2b2t.com:43594` — no WSS.

```bash
export BOT_VAULT_PASS=bot
# CLI: run one or more vaulted profiles
cargo run --release -p host-play -- --user test

# Panel: same vault, native UI (MultiBox, --live)
cargo run --release -p panel --bin panel-play

# Headed live (BOT_VAULT_PASS unused): FAIL+exit 1; PASS keeps the window up
cargo run --release -p panel --bin panel-play -- --live null_raster
# or BOT_LIVE=null_raster; headless twin: LIVE=1 cargo test -p e2e --test null_raster -- --ignored --test-threads=1

# Headless RSS ladder (all slots Null / set_draw=false). One N per process.
LIVE=1 RSS_N=1 cargo test -p e2e --test rss_ladder -- --ignored --test-threads=1 --nocapture

# 50-bot MultiBox wall watch (temp vault s00…s49; 10 min timeout; local engine)
cargo run --release -p panel --bin panel-play -- --live stress50

# Unit tests (no engine). CI: fmt + clippy --no-deps + cargo test (no LIVE=1)
cargo test -p nav --offline
cargo test -p api --offline
```

`--release` matters: a debug `cargo run` looks frozen and spikes RAM. Headed default is the GPU renderer; `BOT_CPU=1` forces CpuPix3D. 274bot defaults to **lowmem**. Renderer-off / `set_draw(false)` still runs `mainloop` and collision, but does not decode loc meshes. **Music / SFX** sets `Profile.settings.lowmem = false` for that username.

The panel only starts the **focused** vault profile; switching the combo starts a parked name once. Last focus persists in `~/.274bot/panel-ui.json`. Credentials are **2×2**: Save/Clear then Log in/Logout. Unlocking the vault starts the **first** profile as a live slot; MultiBox raises the running set as a sidecar rail or a grid, with bulk **Login all / Logout all**. Auto-login defaults **off** per profile.

**panel-play does not auto-create `test`/`test`**: an empty first-run vault stays empty until you type a username/password and Save. `--vault-pass` is equivalent to `BOT_VAULT_PASS`. Empty passphrase is rejected. **host-play** upserts named users (`--user test` defaults to `test`/`test`). `--debug` or `BOT_DEBUG=1` prints slot logs. `--mainland` (or `BOT_MAINLAND=1`) after scene 2 sends the rs2b0t tutorial skip.

**Scripts:** panel **Browse / Start / Pause / Stop** are live. Compiled cards tick on the **PLAYER_INFO** edge. Idle slots have no V8. **Load** a `.ts`/`.js` to add a picker card tagged JS. WalkTo on the main chrome is host nav, not a script card (compiled names like WalkTo are reserved). Persist: `~/.274bot/js-scripts.json`.

**Windows:** `panel-play` is an OS window. It does **not** open the client’s `Present` applet. The Game pane blits the client frame (GPU texture or CpuPix3D), **never below 765×503**. Watch **1 fps**, capture **50 fps**. The real 765×503 applet is `vendor/fr-client-rust` `client-play --window` (bothost), for fidelity.

## Nav

Bake the collision + transport pack, then WalkTo / `Traveller::follow` over it:

```bash
cargo run -p nav --bin nav-pack
```

Output: `$NAV_PACK` or `~/.274bot/274bot.navpack`. Live twins: `nav_full`, `nav_walk`, `nav_door` (`LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1`).

## Live tests

Require the local engine. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped. Agent API notes: [`docs/api/`](docs/api/README.md).
