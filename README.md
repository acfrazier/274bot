# 274bot

**Alpha `0.1.0`.** A Rust **bot host** for RuneScape revision 274 (~2004): N clients in one process, shared type tables, a login FIFO, an encrypted vault, an agent API, a native panel, and whole-world nav.

This tag is the public surface for the **host + API + nav**. The script *kernel* (Browse / Start / Pause / Stop, JS Load) and WalkTo are in-tree; **honest bot scripts are not** — that campaign comes after this tag. See [CONTRIBUTING.md](CONTRIBUTING.md) and [CHANGELOG.md](CHANGELOG.md).

| | |
|--|--|
| **Revision** | RuneScape 274 (~2004) |
| **Engine** | A **local** Lost City engine (game `43594`, HTTP `/crc` on `:80`) |
| **Client** | submodule [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) `r274-bh-modular` (bot-host fork of the modularized Fairy-Ring 274 client) |
| **This repo** | [acfrazier/274bot](https://github.com/acfrazier/274bot) — **MIT** ([LICENSE](LICENSE), [NOTICE.md](NOTICE.md)) |

## What it is

A Rust bot host over the 274 client. One OS thread per `Client` on a 20 ms loop, shared unpacked type tables, a login FIFO, AES-256-GCM vaulted profiles, and an agent API (snapshot → query → interact → settle). **`panel-play`** is the first-class operator window (dear-app/ImGui): profile picker, status, WalkTo picker, game blit, click-through capture, MultiBox rail/grid, `--live` harness. **`host-play`** is the headless CLI over the same kernel.

The headed client draws with a **wgpu GPU** renderer in the submodule (CPU Pix3D is `BOT_CPU=1`). Nav is a baked collision + transport pack (magic `274V`, version byte **6**, compact walk words), Dijkstra router, and pollable `Traveller::follow` driven from WalkTo and from scripts. Compiled script cards tick on the `PLAYER_INFO` edge; Load’d JS is isolate + stub prelude. The only compiled card in-tree is the WalkTo *name* reservation — WalkTo itself is host nav, not a farming script.

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

You need a **local 274 engine** (game `43594`, HTTP `/crc` on `:80`) and the pack cache. This repo does **not** ship or download Jagex assets. Point **`$ENGINE_DIR`** at the engine root (default `$HOME/experiments/Server/engine`). Cache is `$ENGINE_DIR/data/pack/client` (override with `--cache`). On first `maininit` the client GETs `/crc` and jag files from the engine HTTP into that directory; later boots reuse the files on disk.

Stock Lost City Server uses the **Java default login RSA**. That is the usual local-dev case — no key bake. If you rotated the engine `private.pem`, 274bot reads the public half from `$ENGINE_DIR/data/config/private.pem` at login (or `LOGIN_RSAN` / `LOGIN_RSAE`). Then `cargo run --release -p panel --bin panel-play`. Nav pack: `cargo run -p nav --bin nav-pack` over `$ENGINE_DIR/../content/maps`.

Alpha’s supported world is the **local engine**. Cargo `TARGET` is the rustc triple, not a world switch.

```bash
export BOT_VAULT_PASS=bot
# CLI: run one or more vaulted profiles
cargo run --release -p host-play -- --user test

# Panel: same vault, native UI (MultiBox, --live)
cargo run --release -p panel --bin panel-play

# Headed live (BOT_VAULT_PASS unused): FAIL+exit 1; PASS keeps the window up
cargo run --release -p panel --bin panel-play -- --live null_raster
# or BOT_LIVE=null_raster; headless twin: LIVE=1 cargo test -p host-play --test null_raster -- --ignored --test-threads=1

# Headless RSS ladder (all slots Null / set_draw=false). One N per process.
LIVE=1 RSS_N=1 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture

# 50-bot RAM watch (cap-only; Game 50 fps; rail skip-paint; 10 min; local engine)
cargo run --release -p panel --bin panel-play -- --live stress50
# 50-bot full-rate Game + sidecar (run after stress50 holds)
cargo run --release -p panel --bin panel-play -- --live stress50_full

# Unit tests (no engine). CI: fmt + clippy --no-deps + cargo test (no LIVE=1)
cargo test -p nav --offline
cargo test -p api --offline
```

`--release` matters: a debug `cargo run` looks frozen and spikes RAM. Headed default is the GPU renderer; `BOT_CPU=1` forces CpuPix3D. 274bot defaults to **lowmem**. Renderer-off / `set_draw(false)` still runs `mainloop` and collision, but does not decode loc meshes. **Music / SFX** sets `Profile.settings.lowmem = false` for that username.

The panel only starts the **focused** vault profile; switching the combo starts a parked name once. Last focus persists in `~/.274bot/panel-ui.json`. Credentials are **2×2**: Save/Clear then Log in/Logout. Unlocking the vault starts the **first** profile as a live slot; MultiBox raises the running set as a sidecar rail or a grid, with bulk **Login all / Logout all**. Auto-login defaults **off** per profile.

**panel-play does not auto-create `test`/`test`**: an empty first-run vault stays empty until you type a username/password and Save. **host-play** accepts `--vault-pass` (same as `BOT_VAULT_PASS`) and upserts named users (`--user test` defaults to `test`/`test`). The panel has no `--vault-pass` flag — passphrase is `BOT_VAULT_PASS` or the in-window prompt. Empty passphrase is rejected. `--debug` or `BOT_DEBUG=1` prints slot logs. `--mainland` / `BOT_MAINLAND=1` (host-play) after scene 2 sends the courtyard tele + `setvar tutorial 1000`. On a local engine the panel **TutSkip** button is omitted until `getvar tutorial` says the tutorial is still open; press is `setvar tutorial 1000` and caches `tutorial_skipped`.

**Scripts:** panel **Browse / Start / Pause / Stop** are live. Compiled cards tick on the **PLAYER_INFO** edge. Idle slots have no V8. **Load** a `.ts`/`.js` to add a picker card tagged JS. WalkTo on the main chrome is host nav, not a script card (compiled names like WalkTo are reserved). Persist: `~/.274bot/js-scripts.json`. There are no honest skilling/farming ports in this tag.

**Windows:** `panel-play` is an OS window. It does **not** open the client’s `Present` applet. The Game pane blits the client frame (GPU texture or CpuPix3D), **never below 765×503**. Watch **1 fps**, capture **50 fps**. The real 765×503 applet is `vendor/fr-client-rust` `client-play --window` (bothost), for fidelity.

## Nav

Bake the collision + transport pack, then WalkTo / `Traveller::follow` over it:

```bash
cargo run -p nav --bin nav-pack
```

Output: `$NAV_PACK` or `~/.274bot/274bot.navpack` (magic `274V`, version byte **6**). Rebake after this tag — v5 files are `BadVersion`. Pass `[MAPS_DIR] [DOORS_DIR] [CONFIG_JAG]` if the Server tree is not at the bake defaults. `find` keeps wilderness and any-tile teleports **off** unless `FindOptions` opts in. Live twins include `nav_full` and `nav_door` (Catherby door-troll gold fixture), plus gate / cart / spirit / wildy / toll / essence / Elkoy / Zanaris tests under `crates/e2e/tests`. Example: `LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1`.

## Live tests

Require the local engine. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped. Agent API notes: [`docs/api/`](docs/api/README.md).
