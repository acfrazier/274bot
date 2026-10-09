# 274bot

**Beta (0.2.0 Beta 1).** A Rust **bot host** for RuneScape revision **289** (~2004-era Lost City stack), with revision **274** on a best-effort path: N clients in one process, shared type tables, a login FIFO, an encrypted vault, an agent API, a native panel, a headless TUI, whole-world nav, and a host-scoped random-event guardian.

The host, API, navigation, guardian, panel and TUI are in-tree. Browse / Start / Pause / Stop, bulk Start all / Stop all, catalog refresh, manual Reload, and JS/TS loading share the Rust host kernel. Catalog compatibility is **partial**: implemented banking, food, loadout and traversal helpers run through Rust; unsupported operations fail closed. This is not a claim that every catalog script or option matrix passes. See [CHANGELOG.md](CHANGELOG.md), [docs/api/script.md](docs/api/script.md), and the [reusable live harness](docs/harness.md) / [native suite runner](docs/e2e-suite.md).

## What 0.2.0 does

Native scripts compiled into the host, driven through the panel or TUI script chrome like any other card:

- **Quester** — runs built-in quest Paths on revision 289 (20 guides, plus 5 untested drafts marked `[draft]` that run only when picked by name or when "Include draft guides when no quests are picked" is on; members quests run only on members worlds), reading progress from the quest journal and picking up again after Stop/Start. Pick quests, skips and their order from lists; each run by default recovers from two deaths (setting) across the queue and stops on the next. Custom guides load from a folder set under Quest Paths in the Quester settings (panel Script prefs, TUI settings). Path format: [docs/quester-paths.md](docs/quester-paths.md).
- **Gatherer** — woodcutting, mining and fishing at a named site, a start tile, a custom area, or Auto search for the nearest usable spots. Power disposition drops products; Bank disposition banks them at the nearest bank you can walk to (or a bank you pick) and walks back.
- **Combat** — the shared native fight behind the native scripts: melee, ranged and magic with protection prayers, potion sips and eating. Prayers you turned on yourself stay on; only the prayers the bot raised are switched off afterwards.
- **Clues (Sherlock)** — the Rust-native clue-trail solver: waits for a held clue and works it through the shared clue machine.
- **Fleet** — mark bots once and drive them from the panel Fleet window or the TUI fleet table: Start, Stop, Assign, Assign & restart, Log in/out, Walk N to one destination, and Apply the focused bot's settings to the marks. Marks and per-bot outcomes are shared between the two front ends.
- **Load scripts** — JS/TS scripts can run the bot's own fighter (`api.combat.fight(...)` / `api.combat.stop()`, shown in `api.snapshot.combat`), start, watch and stop a Gatherer session, read quest progress, and use the rs2b0t `Area` class. Bundled examples live in `crates/script/examples/`.

Getting around and paying your way:

- **Server profiles** — one entry from `~/.274bot/servers.json` per process (`local-274`, `local-289`, `rs2b2t` built in; see below).
- **Danger routing** — walks route around dangerous monsters and the Temple of Ikov lava bridge by default and name the monsters in the way when no safe route exists. Nav config's **Danger routing** has three levels: Never, When survivable (the default; it acts as Never until the bot can track poison) and Always. WalkTo offers a per-walk "Route through danger zones" opt-in, and scripts can allow danger zones for their own walks in Script prefs.
- **Fares and tolls** — boat and cart rides pay their real fares and the Lumbridge–Al Kharid gate takes its 10 coins; a walk that cannot pay refuses before boarding and says how many coins are missing.
- **Bank choice** — Gatherer banks at Nearest by default or a named bank you pick; map-walk bank fetch withdraws only what the walk is missing.

| | |
|--|--|
| **Revisions** | **289** is the production revision; **274** is best-effort (immutable per-process server profiles; see below) |
| **Engine** | A **local** Lost City engine for the selected revision (defaults differ by profile) |
| **Client** | submodule [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) `r274-bh-modular` (bot-host fork of the modularized Fairy-Ring client) |
| **This repo** | [acfrazier/274bot](https://github.com/acfrazier/274bot) — **MIT** ([LICENSE](LICENSE), [NOTICE.md](NOTICE.md)) |
| **Packages** | macOS app plus standalone `panel-play` / `tui-play` binaries for macOS, Windows and Linux (see [FIRST-START.md](FIRST-START.md)) |

## Server profiles (revision + world)

A session binds one entry from `~/.274bot/servers.json` for the whole process.
The file is created on first use with these built-ins:

| Profile | Revision | Transport | Default game | Default assets | Default vault |
| --- | --- | --- | --- | --- | --- |
| `local-274` | 274 | TCP | `127.0.0.1:43594` | HTTP `:80` | `~/.274bot/vault` |
| `local-289` | 289 | TCP | `127.0.0.1:44594` | HTTP `:1080` | `~/.274bot/vault-289` |
| `rs2b2t` | 289 | WSS | first roster world (default `w1.rs2b2t.com:443`) | HTTPS `:443` | `~/.274bot/vault-prod` |

Select any stored name with `--profile NAME` on `host-play`, `panel-play`, and
`tui-play`, or with `BOT_SERVER_PROFILE`. `--rs2b2t` selects the public built-in;
`public-289` remains a name alias. `--prod` and `BOT_TARGET` were removed.
Connection overrides apply only to the selected profile and fail closed when
they conflict with its revision, roster, or plaintext policy.

The resolved profile is `Local` only when its transport is TCP and every game
host is loopback; only that class admits client cheat packets. Every WSS
profile is `Remote`. A TCP profile may opt in to a non-loopback endpoint with
`allow_plaintext_offhost: true`, but remains `Remote`; the opt-in is rejected
on WSS profiles.

Public play uses the `rs2b2t` roster, with w1, w2 and w3 bundled by default.
The shared cache is prepared from the first world, trying the
next if its asset server is unreachable; `~/.274bot/unpack-289` remains shared.
The exact 0.2.0 `servers.json` roster with w1 and w2 is extended once;
`servers.json.pre-0.2.0.1` is the byte-exact backup and completion marker, so
later edits (including removing w3) stay respected. If the migration cannot be
written or `servers.json` is a symlink, the app warns and keeps the existing
roster. Other world lists are left alone.
Each account can select `auto` or a listed world in the panel Profiles editor.
An auto account moves to the next listed world when login says it is full,
waiting after all worlds report full; pinned accounts stay put. Panel and TUI
show the active world beside each slot. `tui-play --world N` sets a session
default only for accounts stored as auto. Public login fetches the current
RSA modulus from that world's `/client/client.js`, caches successful fetches
per host and refreshes on a wrong-key login response; a failed fetch uses
the baked key without caching it.

`--world-members true|false` is an operator-declared WORLD property, held immutable with the profile; explicit overrides always win. It is not a client/server packet observation, `NODE_MEMBERS`, or account membership. Without an override, a Local profile uses `node.members` from its engine's `data/config/world.json` only when that file declares the selected revision. Otherwise, `rs2b2t` declares members only when every configured world is an rs2b2t world (`w<N>.rs2b2t.com`, ASCII numeric label, case-insensitive, port 443). Other unresolved hosts, ports, and mixed rosters remain unknown and route as F2P. Profile-bound clients also receive `members=false` for unknown or known-free worlds; that prevents members map squares from being prefetched and makes members-object interface counts unavailable, while login remains unaffected.`

On the first launch after upgrading, a valid `~/.274bot/worlds.json` is copied
into the `rs2b2t` roster. The original is untouched; `servers.json` is
authoritative thereafter. Each profile stores its vault filename explicitly,
so existing `vault`, `vault-289`, and `vault-prod` files are not renamed or
rewritten.

Each panel/TUI map Walk confirmation emits one Info-level `WalkTo` outcome per requested slot through the shared host log sink, including refusals. It records origin, the routed destination (including snapping), teleport/wilderness/bank-fetch options, effective WORLD membership and compact provenance (`unknown`, `explicit`, `rs2b2t` or `local-world-json`), then route legs/walk steps/transports or the refusal reason. Local declaration paths and digests are not included. Unfocused refusals appear in the Process log; an empty group keeps its selection and reports “no bots selected.” Availability checks and frame-by-frame following do not emit request lines.

Without an explicit profile, the saved panel revision chooses `local-274` or
`local-289`; otherwise local 274 is the default.

Local engine roots are never inferred from `HOME`. A local profile resolves
the root in this order: `--engine`, `ENGINE_DIR`, then `login_key.engine_dir`
in `~/.274bot/servers.json`. New `servers.json` files leave that path unset;
without an explicit value, a local profile fails fast with setup guidance.
The public WSS profile does not need an engine root.

All profiles cache fetched client archives under `~/.274bot/unpack` (274) or `~/.274bot/unpack-289` (289) unless `--cache`/`--unpack` overrides them. This repo ships no Jagex assets; the client fetches `/crc` and jags from the selected profile's asset endpoint.

Beta's **tested** path is the **local** engine on revision **289**. Revision 274 is best-effort: it may run, but it is neither tested nor claimed. The public world is built in for login/asset fetch; it is **not** a hosted wall, **not** Jagex, and there is **no** public-world CI or SLA.

## What it is

A Rust bot host over the bot-host client fork. One OS thread per `Client` on a 20 ms loop, shared unpacked type tables, a login FIFO, AES-256-GCM vaulted profiles, and an agent API (snapshot → query → interact → settle). **`panel-play`** is the first-class operator window (ImGui over a panel-owned winit + wgpu loop): profile picker, status, WalkTo picker, game blit, click-through capture, MultiBox rail/grid, script chrome, `--live` harness. **`tui-play`** is the same `Play` session with raster Off (ratatui; VPS-cheap). **`host-play`** is the headless CLI over the same kernel. A host-scoped **random-event guardian** Talk-to + continues the five dialog randoms (toggle default on).

The headed client draws with a **wgpu GPU** renderer in the submodule (CPU Pix3D is `BOT_CPU=1`). Nav is a baked collision + transport pack (magic `274V`, version byte **16**; v15 and older are `BadVersion` and must be rebaked), Dijkstra router, and pollable `Traveller::follow` driven from WalkTo and from scripts. Compiled script cards tick on the `PLAYER_INFO` edge; loaded JS/TS runs in an isolate with the host compatibility prelude. The compiled cards in-tree are **Quester**, **Gatherer** and **Sherlock** (see above) — WalkTo itself is host nav, not a farming script.

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

Set `ENGINE_DIR` to your local engine root, or pass `--engine` on the command
line. The command-line value wins over `ENGINE_DIR` and a saved profile path.
On first `maininit` the client GETs `/crc` and jag files from the engine HTTP
and retains the checked JAGs plus decoded snapshots under the revision unpack
root; later boots revalidate and reuse them. See [FIRST-START.md](FIRST-START.md)
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
cargo test -p nav --offline
cargo test -p api --offline
```

`--release` matters: a debug `cargo run` looks frozen and spikes RAM. Headed default is the GPU renderer; `BOT_CPU=1` forces CpuPix3D. 274bot defaults to **lowmem**. Renderer-off / `set_draw(false)` still runs `mainloop` and collision, but does not decode loc meshes. **Music / SFX** sets `Profile.settings.lowmem = false` for that username.

The panel only starts the **focused** vault profile; switching the combo starts a parked name once. Last focus persists in `~/.274bot/panel-ui.json`. Credentials are **2×2**: Save/Clear then Log in/Logout. Unlocking the vault starts the **first** profile as a live slot; MultiBox raises the running set as a sidecar rail or a grid, with bulk **Login all / Logout all** and bulk **Start all / Stop all** (script bulk is separate from login bulk). Auto-login defaults **off** per profile.

**panel-play does not auto-create an account**: an empty first-run vault stays empty until you type a username/password and Save. `host-play` and `tui-play` create requested missing users with fresh 20-character game passwords. **The vault passphrase is never taken from the environment or command line**: the panel asks in its unlock window, `host-play` and `tui-play` ask on the terminal (hidden; a new vault asks twice), and all three read one line from a pipe with `--vault-pass-stdin`. A **new** vault needs a non-empty passphrase after trimming surrounding whitespace; strength is the user's choice. An **existing** vault opens with the passphrase it was created with, however short. `BOT_VAULT_PASS` and `--vault-pass` were removed (see [docs/api…

**Scripts:** panel **Browse / Load / Reload / Start / Pause / Stop** are live; MultiBox rail adds **Start all / Stop all**. Catalog **Refresh catalog** (with confirm when running/paused bots are affected). Successful Start persists a per-profile script assignment and settings bag in the vault. Transpile is content-addressed under `~/.274bot/js-cache` (raw-source SHA-256). WalkTo on the main chrome is host nav, not a script card. Catalog scripts come from your configured `$RS2B0T` / `--catalog` checkout; this repository does not copy their source. Details: [docs/api/script.md](docs/api/script.md).

**Window:** `panel-play` is an OS window. It does **not** open the client’s `Present` applet. The Game pane blits the client frame (GPU texture or CpuPix3D), **never below 765×503**. Unfocused slots paint at the **1 fps** watch cadence only in a MultiBox wall with *Only render selected* off; otherwise they stay draw-off. The focused slot runs at **50 fps**. The real 765×503 applet is `vendor/fr-client-rust` `client-play --window` (bothost), for fidelity.

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

Output: `$NAV_PACK` or `~/.274bot/274bot.navpack` (magic `274V`, version byte **16**). **Rebake existing v15 and older packs after updating:** v16 adds the conjunctive `worn_all_req` list and an optional exact takeoff per edge. v15 added count-prefixed per-edge `consumed_req` and `item_returns` vectors after reusable `item_req`. Spell runes and script-deleted fares/passes are consumed; charged jewellery returns its content-declared `next_obj_stage` after a successful rub. Resource counts must be positive, and returned items require consumption. The v14 kind-byte flag still marks content-derived player-relative ladder/stair landings without added wire bytes. v16 retains the zone table, selected quest-family binding, typed quest-stage gates and per-edge approach geometry; `decode` rejects v15 and older as `BadVersion`. Pass `[MAPS_DIR] [DOORS_D…

**External v16 packs keep their baked routing data.** The bundled pack is rebuilt automatically, but a pack you baked yourself (`--nav-pack`, `NAV_PACK`, or a file under `~/.274bot`) is not re-derived from the current content tree. Rebake it with `nav-pack` after content or format changes to refresh its zone table, resource accounting and routing data.

## Live tests and suite runner

`-p e2e`, `null_raster` and `rss_ladder` connect to a **local-274** engine on `127.0.0.1:43594`; the panel/TUI `--live` runs and the 289-gated host-play tests use the selected profile. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped. Ordered multi-case runs use the native suite runner ([docs/e2e-suite.md](docs/e2e-suite.md)): runner completion is not script qualification; captures that need human readback stay `pending_visual_review`.

Agent API notes: [`docs/api/`](docs/api/README.md). First-time setup: [FIRST-START.md](FIRST-START.md). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md).
