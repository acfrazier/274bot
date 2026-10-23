# Build 274bot from source and run the tests

> **Audience:** contributors who build 274bot with Rust. If you only want to
> use 274bot, download a package and follow [FIRST-START.md](../../FIRST-START.md).
> This text moved here from the README; only relative links changed. See also
> [CONTRIBUTING.md](../../CONTRIBUTING.md).

## What it is

A Rust bot host over the bot-host client fork. One OS thread per `Client` on a 20 ms loop, shared unpacked type tables, a login FIFO, AES-256-GCM vaulted profiles, and an agent API (snapshot → query → interact → settle). **`panel-play`** is the first-class operator window (ImGui over a panel-owned winit + wgpu loop): profile picker, status, WalkTo picker, game blit, click-through capture, MultiBox rail/grid, script chrome, `--live` harness. **`tui-play`** is the same `Play` session with raster Off (ratatui; VPS-cheap). **`host-play`** is the headless CLI over the same kernel. A host-scoped **random-event guardian** Talk-to + continues the five dialog randoms (toggle default on).

The headed client draws with a **wgpu GPU** renderer in the submodule (CPU Pix3D is `BOT_CPU=1`). Nav is a baked collision + transport pack (`crates/nav-planner`, magic `274V`, version byte **17**; v16 and older are `BadVersion` and must be rebaked), Dijkstra router (`crates/nav-planner`), and pollable `Traveller::follow` (`crates/nav-executor`) driven from WalkTo and from scripts. Compiled script cards tick on the `PLAYER_INFO` edge; loaded JS/TS runs in an isolate with the host compatibility prelude. The compiled cards in-tree are **Quester**, **Gatherer** and **Sherlock** (see the [README](../../README.md)) — WalkTo itself is host nav, not a farming script.

## What it is not

- No hosted product, no official anything: **not** Jagex, **not** official Lost City, **not** Fairy Ring, **not** a file-port of rs2b0t (rs2b0t’s chrome is mimicked in the panel; the code is our own).
- Do **not** push `Fairy-Ring/FR-client-rust`. The live client branch is **`multirev-bh-modular`** (renamed from `r274-bh-modular` on 2026-10-10); tags `274bot-<version>` mark the client each release shipped with. `r274-modular` is the same refactor without bot-host hooks. `r274-bothost` is the pre-modular fork.
- Still no bot action API inside `client`. Packet timing and `doAction` stay Java-shaped. Client MIT/NOTICE stay in the submodule.
- This tree ships **no Jagex assets**; you bring your own local engine and pack cache.
- Not a foreign JavaScript runtime or an extra JSON host wire: the only JS ↔ Rust isolate wire is FlatBuffers. Behavior, config and fail-closed helpers live in Rust; the shim is thin coercion/marshaling.

## Quick start

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
```

Set `ENGINE_DIR` to your local engine root, or pass `--engine` on the command
line. The command-line value wins over `ENGINE_DIR` and a saved profile path.
On first `maininit` the client GETs `/crc` and jag files from the engine HTTP
and retains the checked JAGs plus decoded snapshots under the revision unpack
root; later boots revalidate and reuse them. See [FIRST-START.md](../../FIRST-START.md)
for the retention layout.

Stock Lost City Server uses the **Java default login RSA key pair** for local — no key bake. If you rotated `private.pem`, login reads the public half from `$ENGINE_DIR/data/config/private.pem` (or `LOGIN_RSAN` / `LOGIN_RSAE`).

```bash
export ENGINE_DIR=/absolute/path/to/engine

# The vault passphrase is never read from the environment or the command line:
# the panel asks in its unlock window, host-play and tui-play ask on the terminal.
# Prefer an explicit profile. Example: local 289 engine.
cargo run --release -p panel --bin panel-play -- --profile local-289
# Local 274: set ENGINE_DIR to the 274 engine root for this build/run.
# BOT_NAV_REVISION=274 cargo run --release -p panel --bin panel-play -- --profile local-274

# CLI: run one or more vaulted profiles (missing users, including the default
# test user, are created with a fresh random game password).
# Asks for the vault passphrase on the terminal; a new vault asks twice.
cargo run --release -p host-play -- --profile local-289 --user test

# TUI: same vault, raster Off (no GPU). Same passphrase prompt; from a script use
# `printf '%s\n' "$PASS" | tui-play --vault-pass-stdin ...` (see docs/api/vault.md).
cargo run --release -p tui --bin tui-play -- --profile local-289 --user test

# Headed live (no vault passphrase: it makes a throwaway vault): FAIL+exit 1
cargo run --release -p panel --bin panel-play -- --profile local-289 --live null_raster
# Nav execute corpus (selected profile's engine; unique live account):
# cargo run --release -p panel --bin panel-play -- --profile local-289 --live script_nav_routes
# Headless twin (needs a local-274 engine on 127.0.0.1:43594): LIVE=1 cargo test -p host-play --test null_raster -- --ignored --test-threads=1

# Headless RSS ladder (needs a local-274 engine on 127.0.0.1:43594; all slots Null / set_draw=false). One N per process.
# Prints rss=…; does not fail on size. Not a Started-JS isolate ladder.
LIVE=1 RSS_N=1 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture

# 50-bot RAM watch (cap-only; Game 50 fps; rail skip-paint; 10 min; local engine)
cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50
# 50-bot full-rate Game + sidecar (run after stress50 holds)
cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50_full

# Unit tests (no engine). Local CI: fmt + clippy -D + test on host *and* vendor/fr-client-rust (no LIVE=1).
cargo test -p nav-planner --offline
cargo test -p api --offline
```

`--release` matters: a debug `cargo run` looks frozen and spikes RAM. Headed default is the GPU renderer; `BOT_CPU=1` forces CpuPix3D. 274bot defaults to **lowmem**. Renderer-off / `set_draw(false)` still runs `mainloop` and collision, but does not decode loc meshes. **Music / SFX** sets `Profile.settings.lowmem = false` for that username.

The panel only starts the **focused** vault profile; switching the combo starts a parked name once. Last focus persists in `~/.274bot/panel-ui.json`. Credentials are **2×2**: Save/Clear then Log in/Logout. Unlocking the vault starts the **first** profile as a live slot; MultiBox raises the running set as a sidecar rail or a grid. The fleet toolbar (at the top of the rail, in the side panel's fleet section in Grid or without MultiBox, and in the Fleet window) acts on the marked bots, or on every loaded bot shown after asking first: Start, Stop, Assign, Assign & restart, Log in, Log out, Walk, Switch world and Copy settings to. Auto-login defaults **off** per profile.

**Coming from rs2b0t:** export your profiles from rs2b0t's Settings, then use **Import from rs2b0t…** in the panel's Profiles window (or **Import rs2b0t profiles…** in the TUI palette) to bring over your accounts, their worlds, selected scripts, script parameters, genie lamp skill and clue duel partner, after a preview. Details and the full mapping: [docs/api/vault.md](../api/vault.md#importing-profiles-from-rs2b0t).

**panel-play does not auto-create an account**: an empty first-run vault stays empty until you type a username/password and Save. `host-play` and `tui-play` create requested missing users with fresh 20-character game passwords. **The vault passphrase is never taken from the environment or command line**: the panel asks in its unlock window, `host-play` and `tui-play` ask on the terminal (hidden; a new vault asks twice), and all three read one line from a pipe with `--vault-pass-stdin`. A **new** vault needs a non-empty passphrase after trimming surrounding whitespace; strength is the user's choice. An **existing** vault opens with the passphrase it was created with, however short. `BOT_VAULT_PASS` and `--vault-pass` were removed (see [docs/api…

**Scripts:** panel **Browse / Load / Reload / Start / Pause / Stop** are live; the fleet toolbar adds bulk **Start / Stop**. Catalog **Refresh catalog** (with confirm when running/paused bots are affected). Successful Start persists a per-profile script assignment and settings bag in the vault. Transpile is content-addressed under `~/.274bot/js-cache` (raw-source SHA-256). WalkTo on the main chrome is host nav, not a script card. Catalog scripts come from your configured `$RS2B0T` / `--catalog` checkout; this repository does not copy their source. Details: [docs/api/script.md](../api/script.md).

**Window:** `panel-play` is an OS window. It does **not** open the client’s `Present` applet. The Game pane blits the client frame (GPU texture or CpuPix3D), **never below 765×503**. Unfocused slots paint at the **1 fps** watch cadence only in a MultiBox wall with *Only render focused* off; otherwise they stay draw-off. The focused slot runs at **50 fps**. The real 765×503 applet is `vendor/fr-client-rust` `client-play --window` (bothost), for fidelity.

## Nav

Application builds bake, stage and bundle the navigation world themselves: a
default build targets revision **289**, bakes (or reuses) the pack with the
shared baker, and stages pack + flags next to the built binary
(`target/<profile>/nav/289/`) — exactly where the running app looks. WalkTo /
`Traveller::follow` (`nav-executor` over the `nav` world) work with no manual step:

```bash
cargo build --release -p panel --bin panel-play   # stages nav/289, reusing it when unchanged
```

Missing canonical inputs fail the build with a clear message instead of
producing an app without navigation (`BOT_NAV_BUILD=skip` opts out explicitly;
`BOT_NAV_REVISION=274` selects revision 274). A macOS bundle ships
`Contents/Resources/nav/<revision>/`; `BOT_NAV_RESOURCE_DIR` stages into a
packager's own layout. Warm builds reuse unchanged artifacts; a change to the
content, the cache identity, the pack format or the baker rebakes. Full knob
table and the build-time identity rules: [docs/api/nav.md](../api/nav.md).

`nav-pack` stays for deliberate developer/custom-input bakes over a Server tree:

```bash
cargo run -p nav-planner --bin nav-pack
```

Output: `$NAV_PACK` or `~/.274bot/274bot.navpack` (magic `274V`, version byte **17**).
Rebake existing v16 and older packs after you update 274bot.

Version 17 adds count-prefixed `forbidden_held` and `forbidden_worn` vectors after `worn_all_req`.
These content-derived absence gates block an edge when listed items are held or worn.
Version 16 added the conjunctive `worn_all_req` list and an optional exact takeoff per edge.
Version 15 added count-prefixed per-edge `consumed_req` and `item_returns` vectors after reusable `item_req`.

Spell runes and script-deleted fares and passes are consumed.
Charged jewellery returns its content-declared `next_obj_stage` after a successful rub.
Resource counts must be positive, and returned items require consumption.
The v14 kind-byte flag marks content-derived player-relative ladder and stair landings without added wire bytes.
Version 17 retains the zone table, selected quest-family binding, typed quest-stage gates, and per-edge approach geometry.
`decode` rejects v16 and older as `BadVersion`.
See [the navigation API guide](../api/nav.md) for the full wire format and bake options.

**External v17 packs keep their baked routing data.** The app rebuilds the bundled pack automatically.
It does not derive a pack that you baked yourself from the current content tree.
Rebake it with `cargo run -p nav-planner --bin nav-pack` after content or format changes to refresh its routing data.

## Live tests and suite runner

`-p e2e`, `null_raster` and `rss_ladder` connect to a **local-274** engine on `127.0.0.1:43594`; the panel/TUI `--live` runs and the 289-gated host-play tests use the selected profile. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped. Ordered multi-case runs use the native suite runner ([docs/dev/e2e-suite.md](e2e-suite.md)): runner completion is not script qualification; captures that need human readback stay `pending_visual_review`.

### Navigation qualification and paired live runs

The nav real-map qualification tests are `#[ignore]`d. They need the content tree and its client config jag named explicitly (colon-separated, one jag per root or one for all). A missing input fails; it never skips:

```bash
: "${ENGINE_DIR:?set ENGINE_DIR to the 289 engine root}"
NAV_CONTENT_ROOT="$ENGINE_DIR/../content" \
NAV_CACHE="$ENGINE_DIR/data/pack/client/config" \
cargo test -p nav-planner --lib -- --ignored
```

Nav, panel and scenario twins live in `crates/e2e`. Login, RSS and null-raster twins live in `crates/host-play`. The memory fleet harness is in [docs/dev/harness.md](harness.md). `cargo test` here does not run Fairy-Ring client integration tests.

`tui-play --live script_<name>` runs the same scenario harness as `panel-play --live`. Paired capability gold uses disposable profiles minted by the local engine; never substitute public accounts. Run it headlessly with `tui-play --profile local-289 --live script_clue_duel_3554` or `script_jive_kq_four`. KQ uses disposable max-stat qualification fixtures at the normal 600 ms tick and intentionally continues through two kills, two bank returns and the second descent; do not accelerate it or stop the fleet in the lair. The headed twin is the `panel` `pair_watch` example.
