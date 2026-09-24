# 274bot

**Alpha.** A Rust **bot host** for RuneScape revisions **274** and **289** (~2004-era Lost City stacks): N clients in one process, shared type tables, a login FIFO, an encrypted vault, an agent API, a native panel, a headless TUI, whole-world nav, and a host-scoped random-event guardian.

The host, API, navigation, guardian, panel and TUI are in-tree. Browse / Start / Pause / Stop, bulk Start all / Stop all, catalog refresh, manual Reload, and JS/TS loading share the Rust host kernel. Catalog compatibility is **partial**: implemented banking, food, loadout and traversal helpers run through Rust; unsupported operations fail closed. This is not a claim that every catalog script or option matrix passes. See [CHANGELOG.md](CHANGELOG.md), [docs/api/script.md](docs/api/script.md), and the [reusable live harness](docs/harness.md) / [native suite runner](docs/e2e-suite.md).

| | |
|--|--|
| **Revisions** | **274** and **289** (immutable per-process server profiles; see below) |
| **Engine** | A **local** Lost City engine for the selected revision (defaults differ by profile) |
| **Client** | submodule [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) `r274-bh-modular` (bot-host fork of the modularized Fairy-Ring client) |
| **This repo** | [acfrazier/274bot](https://github.com/acfrazier/274bot) — **MIT** ([LICENSE](LICENSE), [NOTICE.md](NOTICE.md)) |

## Server profiles (revision + world)

A session binds **one** server profile for the whole process. Named profiles are:

| Profile | Revision | World | Default game | Default assets | Default vault |
| --- | --- | --- | --- | --- | --- |
| `local-274` | 274 | local engine | `127.0.0.1:43594` | HTTP `:80` | `~/.274bot/vault` |
| `local-289` | 289 | local engine | `127.0.0.1:44594` | HTTP `:1080` | `~/.274bot/vault-289` |
| `public-289` | 289 | public WSS/HTTPS | first configured world (default `w1.rs2b2t.com:443`) | HTTPS `:443` | `~/.274bot/vault-prod` |

Select with `--profile local-274|local-289|public-289` on `host-play`, `panel-play`, and `tui-play` (or `BOT_SERVER_PROFILE`). Related knobs: `--revision 274|289`, `BOT_REVISION`, `--prod` / `BOT_TARGET=prod|live`, `--engine`, `--cache`, `--vault`, `--catalog`, `--world-members true|false`. Conflicting combinations fail closed (for example `--prod` with a local profile, or `public-274`).

Public world endpoints live in `~/.274bot/worlds.json` (written with w1 and w2
defaults on the first public launch). Its `schema_version: 1` and ordered
`worlds` list hold `{number, host, port, node_id}` entries; edit the file and
restart to apply. Invalid files fail startup, and a public host/port not in the
list is refused. The shared cache is prepared from the first world, trying the
next if its asset server is unreachable; `~/.274bot/unpack-289` remains shared.
Each account can select `auto` or a listed world in the panel Profiles editor.
An auto account moves to the next listed world when login says it is full,
waiting after all worlds report full; pinned accounts stay put. Panel and TUI
show the active world beside each slot. `tui-play --world N` sets a session
default only for accounts stored as auto. Public login fetches the current
RSA modulus from that world's `/client/client.js`, caches it per host and
refreshes on a wrong-key login response; if unavailable, it uses the baked key.

`--world-members true|false` is an operator-declared property of the selected endpoint, held immutable with the profile. It is not a client/server packet observation and not `NODE_MEMBERS`. Explicit `false` beats a local `world.json` `members: true`. Omission uses a guarded bind: only `local-274` / `local-289` loopback profiles whose `engine_dir/data/config/world.json` has matching typed `engine.revision`, `node.port`, and a JSON bool `node.members` inherit that bool. Missing, malformed, mismatched, or public-289 records stay **unknown** and route as not-members. Public-289 never inherits the local engine file; it may still be declared explicitly through `--world-members`. Cache/account membership flags are a different fact.

Without an explicit profile, legacy resolution still applies: plain local defaults to **274**; `--prod` / `BOT_TARGET=prod` without a revision selects **public-289**. Prefer naming the profile.

Default engine roots (override with `--engine` / `ENGINE_DIR`):

- 274: `$HOME/experiments/Server/engine`
- 289: `$HOME/experiments/lostcity-289/engine`

Pack cache for local is `$ENGINE_DIR/data/pack/client` (`--cache` overrides). Public-289 uses `~/.274bot/unpack-289` by default. This repo ships no Jagex assets; the client can fetch `/crc` and jags from the selected engine or public asset endpoint.

Alpha’s **tested** path is the **local** engine for the profile you run. The public world is built in for login/asset fetch; it is **not** a hosted wall, **not** Jagex, and there is **no** public-world CI or SLA.

## What it is

A Rust bot host over the bot-host client fork. One OS thread per `Client` on a 20 ms loop, shared unpacked type tables, a login FIFO, AES-256-GCM vaulted profiles, and an agent API (snapshot → query → interact → settle). **`panel-play`** is the first-class operator window (ImGui over a panel-owned winit + wgpu loop): profile picker, status, WalkTo picker, game blit, click-through capture, MultiBox rail/grid, script chrome, `--live` harness. **`tui-play`** is the same `Play` session with raster Off (ratatui; VPS-cheap). **`host-play`** is the headless CLI over the same kernel. A host-scoped **random-event guardian** Talk-to + continues the five dialog randoms (toggle default on).

The headed client draws with a **wgpu GPU** renderer in the submodule (CPU Pix3D is `BOT_CPU=1`). Nav is a baked collision + transport pack (magic `274V`, version byte **9**), Dijkstra router, and pollable `Traveller::follow` driven from WalkTo and from scripts. Compiled script cards tick on the `PLAYER_INFO` edge; loaded JS/TS runs in an isolate with the host compatibility prelude. The only compiled card in-tree is the WalkTo *name* reservation — WalkTo itself is host nav, not a farming script.

## What it is not

- No hosted product, no official anything: **not** Jagex, **not** official Lost City, **not** Fairy Ring, **not** a file-port of rs2b0t (rs2b0t’s chrome is mimicked in the panel; the code is our own).
- Do **not** push `Fairy-Ring/FR-client-rust`. The live client branch is **`r274-bh-modular`**. `r274-modular` is the same refactor without bot-host hooks. `r274-bothost` is the pre-modular fork.
- Still no bot action API inside `client`. Packet timing and `doAction` stay Java-shaped. Client MIT/NOTICE stay in the submodule.
- This tree ships **no Jagex assets**; you bring your own local engine and pack cache.
- Not a foreign JavaScript runtime or an extra JSON host wire: the only JS ↔ Rust isolate wire is FlatBuffers. Behavior, config and fail-closed helpers live in Rust; the shim is thin coercion/marshaling.

## Quick start

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
```

Point **`$ENGINE_DIR`** (or `--engine`) at the engine root for the revision you will run. On first `maininit` the client GETs `/crc` and jag files from the engine HTTP into the pack cache; later boots reuse disk. Stock Lost City Server uses the **Java default login RSA** for local — no key bake. If you rotated `private.pem`, login reads the public half from `$ENGINE_DIR/data/config/private.pem` (or `LOGIN_RSAN` / `LOGIN_RSAE`).

```bash
export BOT_VAULT_PASS=bot
# Prefer an explicit profile. Example: local 289 engine.
cargo run --release -p panel --bin panel-play -- --profile local-289
# Local 274:
# BOT_NAV_REVISION=274 ENGINE_DIR="$HOME/experiments/Server/engine" cargo run --release -p panel --bin panel-play -- --profile local-274

# CLI: run one or more vaulted profiles (upserts --user; default test/test)
cargo run --release -p host-play -- --profile local-289 --user test

# TUI: same vault, raster Off (no GPU)
cargo run --release -p tui --bin tui-play -- --profile local-289

# Headed live (BOT_VAULT_PASS unused): FAIL+exit 1
cargo run --release -p panel --bin panel-play -- --profile local-289 --live null_raster
# Nav execute corpus (local engine; unique live account):
# cargo run --release -p panel --bin panel-play -- --profile local-289 --live script_nav_routes
# Headless twin: LIVE=1 cargo test -p host-play --test null_raster -- --ignored --test-threads=1

# Headless RSS ladder (all slots Null / set_draw=false). One N per process.
# Prints rss=…; does not fail on size. Not a Started-JS isolate ladder.
LIVE=1 RSS_N=1 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture

# 50-bot RAM watch (cap-only; Game 50 fps; rail skip-paint; 10 min; local engine)
cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50
# 50-bot full-rate Game + sidecar (run after stress50 holds)
cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50_full

# Unit tests (no engine). Local CI: fmt + clippy -D + test on host *and* vendor/fr-client-rust (no LIVE=1).
cargo test -p nav --offline
cargo test -p api --offline
```

`--release` matters: a debug `cargo run` looks frozen and spikes RAM. Headed default is the GPU renderer; `BOT_CPU=1` forces CpuPix3D. 274bot defaults to **lowmem**. Renderer-off / `set_draw(false)` still runs `mainloop` and collision, but does not decode loc meshes. **Music / SFX** sets `Profile.settings.lowmem = false` for that username.

The panel only starts the **focused** vault profile; switching the combo starts a parked name once. Last focus persists in `~/.274bot/panel-ui.json`. Credentials are **2×2**: Save/Clear then Log in/Logout. Unlocking the vault starts the **first** profile as a live slot; MultiBox raises the running set as a sidecar rail or a grid, with bulk **Login all / Logout all** and bulk **Start all / Stop all** (script bulk is separate from login bulk). Auto-login defaults **off** per profile.

**panel-play does not auto-create `test`/`test`**: an empty first-run vault stays empty until you type a username/password and Save. **host-play** accepts `--vault-pass` (same as `BOT_VAULT_PASS`) and upserts named users (`--user test` defaults to `test`/`test`). The panel has no `--vault-pass` flag — passphrase is `BOT_VAULT_PASS` or the in-window prompt. Empty passphrase is rejected. `--debug` or `BOT_DEBUG=1` prints slot logs. `--mainland` / `BOT_MAINLAND=1` (host-play) after scene 2 sends the courtyard tele + `setvar tutorial 1000`. On a local engine the panel **TutSkip** button is omitted until `getvar tutorial` says the tutorial is still open; press is `setvar tutorial 1000` and caches `tutorial_skipped`.

**Scripts:** panel **Browse / Load / Reload / Start / Pause / Stop** are live; MultiBox rail adds **Start all / Stop all**. Catalog **Refresh catalog** (with confirm when running/paused bots are affected). Successful Start persists a per-profile script assignment and settings bag in the vault. Transpile is content-addressed under `~/.274bot/js-cache` (raw-source SHA-256). WalkTo on the main chrome is host nav, not a script card. Catalog scripts come from your configured `$RS2B0T` / `--catalog` checkout; this repository does not copy their source. Details: [docs/api/script.md](docs/api/script.md).

**Windows:** `panel-play` is an OS window. It does **not** open the client’s `Present` applet. The Game pane blits the client frame (GPU texture or CpuPix3D), **never below 765×503**. Watch **1 fps**, capture **50 fps**. The real 765×503 applet is `vendor/fr-client-rust` `client-play --window` (bothost), for fidelity.

## Nav

Application builds bake, stage and bundle the navigation world themselves: a
default build targets revision **289**, bakes (or reuses) the pack with the
shared baker, and stages pack + flags next to the built binary
(`target/<profile>/nav/289/`) — exactly where the running app looks. WalkTo /
`Traveller::follow` work with no manual step:

```bash
cargo build --release -p panel --bin panel-play   # stages nav/289, reusing it when unchanged
```

Missing canonical inputs fail the build with a clear message instead of
producing an app without navigation (`BOT_NAV_BUILD=skip` opts out explicitly;
`BOT_NAV_REVISION=274` selects revision 274). A macOS bundle ships
`Contents/Resources/nav/<revision>/`; `BOT_NAV_RESOURCE_DIR` stages into a
packager's own layout. Warm builds reuse unchanged artifacts; a change to the
content, the cache identity, the pack format or the baker rebakes. Full knob
table and the build-time identity rules: [docs/api/nav.md](docs/api/nav.md).

`nav-pack` stays for deliberate developer/custom-input bakes over a Server tree:

```bash
cargo run -p nav --bin nav-pack
```

Output: `$NAV_PACK` or `~/.274bot/274bot.navpack` (magic `274V`, version byte **9**). **Rebake existing v8 packs after updating:** v9 adds per-edge `members_req` and `decode` rejects v8 as `BadVersion`. Pass `[MAPS_DIR] [DOORS_DIR] [CONFIG_JAG]` if the Server tree is not at the bake defaults. `find` is fail-closed on live `WorldState` and keeps wilderness and any-tile teleports **off** unless `FindOptions` opts in. Live twins include `script_nav_routes` (headed corpus) and `nav_door` (Catherby door-troll gold fixture), plus gate / cart / spirit / wildy / toll / essence / Elkoy / Zanaris tests under `crates/e2e/tests`. Example: `LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1`.

## Live tests and suite runner

Require the local engine for the profile under test. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped. Ordered multi-case runs use the native suite runner ([docs/e2e-suite.md](docs/e2e-suite.md)): runner completion is not script qualification; captures that need human readback stay `pending_visual_review`.

Agent API notes: [`docs/api/`](docs/api/README.md). First-time setup: [FIRST-START.md](FIRST-START.md). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md).
