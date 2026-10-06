# Contributing

274bot is in **beta** for 0.2.0 (host workspace crate versions still
`0.1.9`, no crates.io publish; public history tags `0.1.x`). The host, API,
nav execute, random-event guardian, headless TUI, panel, script kernel
(Browse / Start / Pause / Stop / Load / Reload, bulk Start all / Stop all,
catalog refresh), revision profiles, the native Quester / Gatherer /
Sherlock cards, and the shared panel+TUI Fleet are in-tree. Catalog
compatibility remains **partial** — unsupported helpers fail closed; do not
treat a green unit job as full script qualification. See
[CHANGELOG.md](CHANGELOG.md).

Beta is not turnkey public-world automation: you run a **local** engine for
the revision you care about and select a profile from `~/.274bot/servers.json`.
The public built-in is `rs2b2t` (`--rs2b2t` or `--profile rs2b2t`):
WSS/HTTPS on the configured rs2b2t roster, with served login keys and a baked
fallback. Local is the tested path; Cargo `TARGET` is the rustc triple,
not a world switch. This is **not** Jagex and not a hosted wall — there is
**no public-world CI** and no asset server. PRs that read like unreviewed
model output will be rejected; the product bar is a host that does not suck.

Product docs: [README.md](README.md), [NOTICE.md](NOTICE.md),
[FIRST-START.md](FIRST-START.md), [docs/](docs/README.md),
[docs/architecture.md](docs/architecture.md).

## Prerequisites

- A **local** Lost City engine for the profile under test:
  - `local-274`: TCP `127.0.0.1:43594`, HTTP `/crc` on `:80`
  - `local-289`: TCP `127.0.0.1:44594`, HTTP `/crc` on `:1080`
  This repo does not ship Jagex assets.
- Local profiles resolve the engine root from `--engine`, `ENGINE_DIR`, then
  `login_key.engine_dir` in `~/.274bot/servers.json`. New built-ins leave the
  path unset; choose a root explicitly. The pack cache is
  `$ENGINE_DIR/data/pack/client` (`--cache` overrides).
- RSA: stock LC Server uses the **Java default pair** — no bake. Rotated
  `private.pem` is read at login from `$ENGINE_DIR/data/config/private.pem`
  (or `LOGIN_RSAN` / `LOGIN_RSAE`).
- Nav pack: an ordinary build bakes and stages the selected **build-time**
  revision’s pack and flags next to the binary
  (`target/<profile>/nav/<revision>/`, revision **289** by default,
  `BOT_NAV_REVISION=274` for 274), so the app boots with a bound nav world
  and no manual step. A normal bundled-nav build needs `ENGINE_DIR`, the
  engine checkout's `engine/` folder (or set it once under `[env]` in
  `~/.cargo/config.toml`); set `BOT_NAV_BUILD=skip` to build without bundling. The
  `nav-pack` CLI stays for custom bakes (`$NAV_PACK` or
  `~/.274bot/274bot.navpack`, magic `274V`, version byte **16**; v15 and older
  are `BadVersion`). Details:
  [docs/api/nav.md](docs/api/nav.md).

## Clone and run

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
git submodule update --init

export ENGINE_DIR=/absolute/path/to/engine

cargo run --release -p panel --bin panel-play -- --profile local-289
```

`--release` matters: a debug `cargo run` looks frozen. Headed default is
the wgpu GPU renderer in the client submodule; `BOT_CPU=1` is CpuPix3D.

**host-play** upserts named users (missing users, including the default
`test` user, are created with a fresh random game password).
**panel-play** does not: an empty first-run vault stays empty until you
Save credentials.

The vault passphrase is **never** taken from the environment or the command
line (other users can read both, and children inherit the environment). The
panel asks in its unlock window; `host-play` and `tui-play` ask on the
terminal (hidden; a new vault asks twice); all three read one line from a
pipe with `--vault-pass-stdin`. A new vault needs a non-empty passphrase after
trimming surrounding whitespace; strength is the user's choice. An existing
one opens with whatever passphrase it was created with, however short.
`BOT_VAULT_PASS` and `--vault-pass` are gone (`docs/api/vault.md`).

**tui-play** (`cargo run --release -p tui --bin tui-play -- --profile local-289`)
is the headless operator panel: ratatui + crossterm, same profile flags and
vault layout as `panel-play`, slots spawn raster Off (no GPU).
`--live script_<name>` runs the same scenario harness as
`panel-play --live`.
Paired capability gold uses disposable profiles minted by the local engine;
never substitute public accounts. Run it headlessly with
`tui-play --profile local-289 --live script_clue_duel_3554` or
`script_jive_kq_four`. KQ uses disposable max-stat qualification fixtures at
the normal 600 ms tick and intentionally continues through two kills, two bank
returns and the second descent; do not accelerate it or stop the fleet in the
lair. The headed twin is the `panel`
`pair_watch` example.

Shared profile flags (all three binaries):
`--profile NAME`, `--rs2b2t`, `--revision 274|289`, `--engine`, `--cache`,
`--vault`, `--catalog`, and related overrides. `--prod` and `BOT_TARGET` are
removed. See [README.md](README.md) and [FIRST-START.md](FIRST-START.md).

## Toolchain

Rust **1.98.0** (`rust-toolchain.toml` in this repo and in
`vendor/fr-client-rust`). GitHub Actions uses the same number, not
`@stable`. Bump both files and both workflows together — do not
`rustup update` into a new clippy `-D` set mid-tag.

## Crate layout

Workspace members (`Cargo.toml` `[workspace] members`):

| Crate | Owns |
| --- | --- |
| `host` | Slot loops, login queue, session pump |
| `vault` | Encrypted (AES-256-GCM) account profiles |
| `api` | Agent surface (snapshot → query → interact → settle) plus selected game data |
| `host-play` | Headless CLI and the shared `Play` session core the other front ends reuse |
| `frontend-core` | Shared panel/TUI core: fleet marks, walk grants, operator session |
| `panel` | `panel-play`: the ImGui operator window |
| `tui` | `tui-play`: the headless ratatui operator panel |
| `nav` | Baked collision + transport pack, router, `Traveller::follow` |
| `script` | Script kernel: compiled cards (Quester, Gatherer, Sherlock) and the JS isolate compat surface |
| `scenario` | Live scenario steps, proofs and fleet-start helpers |
| `e2e` | Native suite runner and headed/headless live twins |

`vendor/fr-client-rust` is the client submodule: a path dependency, not a
workspace member (see Client submodule below).

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

# Crate-graph ownership (Python 3.11+). Does not replace fmt/clippy/test.
python3 tools/architecture/check.py
python3 tools/architecture/check.py --self-test
```

Crate edges are an explicit allowlist
([`tools/architecture/policy.toml`](tools/architecture/policy.toml),
[docs/architecture.md](docs/architecture.md)). New reverse or convenience
dependencies, workspace aliases, target tables, and unknown workspace
members fail closed. Optional-only allows do not accept a required target
dependency. Membership is the explicit `workspace.members` paths plus a
root `[package]` if present; globs are unsupported and fail closed.
Lifecycle, FlatBuffer isolate/host wire, last-FBO, and JS-shim policy are
review requirements, not this checker.

GitHub Actions runs the same two manifests after installing ALSA + X11
headers (`libasound2-dev` — panel pulls client `audio` / cpal), plus the
feature-gated test lanes (script `load` and V8-free `--no-default-features`,
host `performance-profile`, host-play/panel/tui `memory-profile`, host-play
`memory-profile-no-alloc`, panel `render-diagnostics`) and the architecture checker
(no Rust toolchain). `path-schema`, `live-harness` and `test-support` are
exercised through the workspace test lane's feature unification; no CI lane
runs `LIVE=1` tests. Independent test commands run as parallel matrix lanes;
the required `test` status aggregates every lane. It is still a **subset**:
`SKIP_GPU=1` (no adapter on those VMs) and never `LIVE=1`. A green GH job is
not a headed or engine pass.

## Live-test rules

Live harnesses need a local engine. `-p e2e`, `null_raster` and `rss_ladder`
connect to a **local-274** engine on `127.0.0.1:43594`; the panel/TUI `--live`
runs and the 289-gated host-play tests use the selected profile. Failures
print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`. Quiet unless
`BOT_DEBUG=1`.

- `LIVE=1` gates every ignored live test; without it, `-- --ignored` still
  skips them. Default `cargo test` stays green with no engine running.
- Run live tests single-threaded (`--test-threads=1`) against a throwaway
  `HOME` (for example `HOME="$(mktemp -d)"`), so live runs never touch your
  operator vault, profiles or cache.
- Live twins mint per-run usernames on disposable local-engine profiles (the
  engine auto-registers unknown accounts). Never substitute public accounts.
- Live runs target local profiles only: `-p e2e`, `null_raster` and
  `rss_ladder` need `local-274`; the panel/TUI `--live` runs and the
  289-gated host-play tests use the selected profile (`local-289` for the
  production revision). Unit tests do not verify public login.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
LIVE=1 cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_nav_door
```

Ordinary `cargo test` reads no content outside the repo. The nav real-map
qualification tests are `#[ignore]`d and need the content tree and its
client config jag named explicitly (colon-separated, one jag per root or one
for all); a missing input fails, never skips:

```bash
: "${ENGINE_DIR:?set ENGINE_DIR to the 289 engine root}"
NAV_CONTENT_ROOT="$ENGINE_DIR/../content" \
NAV_CACHE="$ENGINE_DIR/data/pack/client/config" \
cargo test -p nav --lib -- --ignored
```

Nav / panel / scenario twins live in `crates/e2e`. Login / RSS / null-raster
twins live in `crates/host-play`. Ordered multi-case native runs:
[docs/e2e-suite.md](docs/e2e-suite.md). Memory fleet harness:
[docs/harness.md](docs/harness.md).

`cargo test` here does not run Fairy-Ring client integration tests.

## Selected-data generation

The one entry point is `tools/game-data/generate.ts`; its verifier can run a
single revision without requiring the other source checkout to be clean.
Every external source root is a required environment variable — an unset
variable fails closed and names itself, with no machine default:

```bash
export GAME_DATA_274_ENGINE=/absolute/path/to/274-engine
export GAME_DATA_274_CONTENT=/absolute/path/to/274-content
export GAME_DATA_289_ENGINE=/absolute/path/to/289-engine
export GAME_DATA_289_CONTENT=/absolute/path/to/289-content
export RS2B0T=/absolute/path/to/rs2b0t
export GAME_DATA_274_SNAPSHOTS=/absolute/path/to/274-snapshots
export GAME_DATA_289_SNAPSHOTS=/absolute/path/to/289-snapshots
```

```bash
cargo build -p nav --bin cache-content-id
bun tools/game-data/generate.test.ts
bun tools/game-data/verify.ts --revision 289
bun tools/game-data/generate.ts --revision 289
```

Both commands accept `--revision 274` too. Without a flag they attempt both
revisions: 289 is required, while unavailable/dirty 274 is reported as
`REFUSED` without hiding 289's result. An explicitly requested revision fails
on refusal. Cross-pin corroboration is reported as unchecked unless both
revisions passed. Regenerating one revision preserves the other manifest row;
it does not claim to repair or verify that row.

Input identity covers tracked scripts/packs/maps and engine decoder/limit
sources, plus the ignored generated files pinned by size and SHA-256 in
`tools/game-data/generated-inputs.json`. Those generated-input pins are tied to
the engine/content commits; changing them is a deliberate input-pin change,
not an automatic blessing of local generated bytes. Unknown, missing or edited
ignored inputs refuse. Platform metadata such as `.DS_Store` is not an input.

At the typed-family cutover, `generateSelected` stages families, core and
manifest beside the live revision directory. Nav must bind the emitted quest
digest **and** schema; sources are rechecked before publication. A refused build
removes staging and leaves the previous assets intact. The final manifest's
digest is computed externally, never embedded in its own bytes.

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
Do not claim full catalog parity in product docs or PR descriptions.

## License

MIT ([LICENSE](LICENSE)). Attribution in [NOTICE.md](NOTICE.md).
