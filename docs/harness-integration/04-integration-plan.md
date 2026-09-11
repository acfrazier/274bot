# Combined harness application integration plan

Architect: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: architecture only for brief 46 section D, with
brief 47 / report 06 as a required parent. Not implementation, not
LIVE, not a release tag, not source acceptance, not native proof.

Read once: `AGENTS.md`, `docs/execution.md`, `docs/harness.md`,
`docs/compat/STATE.md`, briefs 46 and 47, and reports 01, 02, 03, 05,
06. Branch checked first: `codex/rs2b0t-multirevision` (not `main`).
Work is this report only. No product edits, LIVE, fixtures, STATE,
sibling files, stash, reset, restore, checkout, merge, remotes, or
release actions. Concurrent untracked evidence under
`docs/compat/evidence/` is root's; it is not this plan's product.

Section E (report 05) is complete and is considered. It does not gate
this plan. Design approval is not live acceptance.

## Verdict

**One product, three named builds, two audiences.** Ordinary users
get a lean `panel-play` / `tui-play` zip with identity, `--check`,
honest F12, scene-2 `--smoke`, a redacted support bundle, and the
same Play/script/nav/render paths used today. Contributors keep
`--features scenario` for `--live`, catalog proof, isolate-stores,
and qualify JSON. Memory stays `--features memory-profile` and is
never the user zip.

Do not add a second capture backend, a second scenario engine, a
host-play.exe, a campaign scheduler, or in-app SSH/fixture extract.
Do not bake counting allocators into "diagnostics". Do not require
this operator's `~/experiments` layout, SSH hosts, or builder
paths. Size and startup wins remain unmeasured hypotheses.

Proceed in thirteen compatibility-release cards (P0–P12), then
stop. Quick wiring (identity, `--check`, envelope, Results, receipts)
is separate from the substantial extract (`RunController` /
CoreWitness) and from the Cargo feature split that makes the lean
zip possible.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `72c366e3e4316af43b09f0ac2bca0e5fd2a8520a` |
| Seam inspection (Cargo / F12 / vault / parse_n) | `e74fa604a451743de2b1e179a2f0b0103f433203` |
| Client submodule HEAD | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Catalog terminal-shot / bank loop | `6c6bb2d5c25d51f5b339d6bb5f73d68b3b7296cb` |
| Startup preparation | `814e5293fec426b722e450580c514b9cffca826f` |
| Brief 46 SHA-256 | `4a5a7df764e4c25fc6f1eeafeaa1ce316df2422a5f69397a9714ab60ce7192a9` |
| Brief 47 SHA-256 | `9fcf23c5d8e1008a314fdcddf669d4f714a153b1a54cd167b6f139a638036f63` |
| Report 01 | `6b5e736a` / SHA-256 `46a967053bb997a53faf932c41ff98a20607a7222958543ab8855c9e3bee00d8` |
| Report 02 | `54d80682` / SHA-256 `7a42b65afe93e59d60e1c6f98c8a86c7d6fefe071115aea725163aaa95d77262` |
| Report 03 | `f31eeab5` / SHA-256 `632801472eb94c60b81654b0fbfc5ce5b53c47ac0dd50b21c42798cfa817804a` |
| Report 05 (E, considered) | `4492b6e4` / SHA-256 `463c94172ca592752227ce6322101cc0e5a57eb6c2af40d5da5092248bfe8b3f` |
| Report 06 (required parent) | `e74fa604` / SHA-256 `4cafce8acabcfcf588444d6ed807be58c4f32d51cb59f6531670a23de6bb0cc5` |
| Audience clarification | `428e8e17` |
| Kanban card | `t_14b7a7e3` |

Current Cargo.toml / cfg sites at `e74fa604` are authoritative.
`72c366e3` (panel proof-scope / debug opt-in, 4 lines) landed during
write-up and does not change those seams. Parent reports remain
correct on the checks below.

## Seams re-verified at HEAD

| Seam | Still true at `e74fa604` |
|---|---|
| panel/tui depend on `scenario` unconditionally | yes |
| host-play `scenario` is optional, only via `memory-profile` | yes |
| host-play has no `[[bin]]` | yes |
| no `required-features` anywhere | yes; catalog/world tests `use scenario` |
| F12 writes `GameSnapshot::new()` pretty JSON | `panel/src/app.rs` `enqueue_manual_shot` |
| headed `--live` exits 0 on `RunnerStatus::Passed` | `live_script_tick` |
| `catalog_watch` enters `IsolatedEnv`; production `--live` does not | example vs `run_panel` |
| CoreWitness lives only in `catalog_boundary_live.rs` | yes |
| `parse_n` rejects `"2"` | yes (`1/16/32/128`) |
| `default_vault_rel(Local)` is `"vault"`, not `"vault-289"` | yes; bound `ServerProfile` is revisioned |
| default local cache is `~/experiments/{Server,lostcity-289}/engine` | `profile.rs` |
| panel `build.rs` stamps time with `date -u` | yes |
| TUI has no `--version` | yes |
| `shot.rs` still in `crates/scenario/src/shot.rs` | yes |
| GPU camera is whole-window imgui readback | unchanged; keep |

## Audience and deployment boundary

Operator premise (brief 47 + comments on this card):

- **Users** receive a leaner gameplay release. They need to play,
  Start catalog JS, validate the selected profile, copy a support
  bundle, take an honest F12, and run headed `--smoke`. They do not
  need the fixture catalog, minted proof accounts, or a counting
  allocator.
- **Us / script authors / contributors** keep scenario builds for
  `--live`, catalog cells, isolate-stores, and qualify JSON.
- **Memory** is a third build for allocation/fleet measurement.
- Everyday support diagnostics stay in the user zip. Production
  script, protocol, navigation, snapshot, lifecycle, and rendering
  paths stay the same. A cfg that removes a runtime check from
  release only is a product fork.

Configuration boundary (final operator clarification):

| Repo owns (reusable) | Machines own (not in git, not in the zip) |
|---|---|
| CLI flags, env names, Cargo features, identity stamps | Engine/content checkouts, nav packs, caches, vaults |
| Packager that *reads* `GIT_COMMIT` / `CLIENT_COMMIT` / `BUILD_TIME` | SSH hosts, Hyper-V, Concord, jump keys |
| `--cache` / `--engine` / `ENGINE_DIR` / `274BOT_SMOKE_DIR` | This operator's `~/experiments/...` layout |
| Feature matrix and disabled-command messages | Builder rustc, NASM, portable Node, firewall |
| Redaction rules for support bundles | RSA private keys, givebank, fixture tarballs |

Backups of machine setup stay on those machines. This plan does not
authorize backup jobs or a migration. Developer tools and release
binaries must run on a foreign checkout when the operator passes
absolute cache/nav/engine paths. Missing defaults fail with the flag
to set, not with "put the engine at `~/experiments/Server`".

`docs/harness.md` already documents `ENGINE_DIR` as an absolute path
and warns that some older helpers still assume
`HOME/experiments/Server/engine`. Keep that honesty. Do not make the
assumption the install layout.

## Conflict resolutions

These are the synthesis decisions. Parent reports stand except where
this table overrides them.

| # | Tension | Decision |
|---|---|---|
| 1 | 03 packager `-p host-play`; 06: no `[[bin]]` | **No host-play.exe.** Identity / `--check` / support-bundle writers live in the host-play *library*. Surfaces are `panel-play` and `tui-play`. |
| 2 | 01 kept `scenario::shot`; 06 moves it for lean release | **Move write/stamp/run-dir/envelope into panel.** `ScenarioRunner::shot_sink` stays a callback. Panel must not become a scenario dependency (cycle). |
| 3 | 03 `--smoke` via `render_smoke`; 06 scene-2 watch without runner | **06 wins.** Release `--smoke` is the existing LiveSmoke latch plus F12-style enqueue. Scenario `--smoke` converges on the same watch so PASS cannot mean runner Passed. |
| 4 | 02 `RunController` always in host-play; 06 cfg `scenario` | **Controller + CoreWitness compile behind `scenario`.** Pause/Stop/reconnect stay unconditional `Play` APIs (user zip). |
| 5 | 03 `--version` on three binaries | **`--version` on panel-play and tui-play only.** |
| 6 | 03 user zip "default features"; 06 `--no-default-features` | **06 wins after P5.** Until the optional-scenario split lands, HEAD still links scenario. Do not claim a lean zip first. |
| 7 | 03 operator: Linux panel vs TUI-only | **Still operator.** Plan always ships tui-play on three OS and panel-play on Mac/Windows. Linux panel-play is an optional extra artifact, not a gate. |
| 8 | Windows `.274bot` vs AppData | **Keep `USERPROFILE\\.274bot`.** Smallest change. |
| 9 | Unsigned zip vs signed installers | **Unsigned zip of binaries + sidecar + LICENSE/NOTICE for alpha.** Signing is operator/infra, following increment. |
| 10 | Support bundle PNG vs JSON-only | **Attach last PNG when present, with 01 honesty.** JSON-only if no shot. No secrets either way. |
| 11 | Client SHA on dim line vs hover | **Hover / `--version` only.** Dim line stays the public label. |
| 12 | 01 CaptureMeta in scenario | **Panel-owned envelope.** Scenario does not learn GPU or PNG types. Headless `evidence.json` is host-play / scenario-feature, not a fake PNG. |
| 13 | 01 card 2 re-arm catalog shots | **Done at `6c6bb2d5`.** Do not re-edit `crates/scenario/src/lib.rs` for labels. Headed drain/Results consume those labels. |
| 14 | 02 sequential CLI in the compatibility increment | **Yes, contributor-only** (P12). Not in the user zip. |
| 15 | 05 game-data vs harness | **Parallel lane.** Serde load is release gameplay. Generator stays build-time on pinned machine checkouts. Not a P0–P12 dependency. |
| 16 | 03 "host-play `--support-bundle DIR`" | **Library function** called from panel menu / TUI key / both binaries' CLI. No new bin. |

Do not re-open excluded experiments: OS `CopyFromScreen` as product
camera, campaign DAG, cohort controllers, heaptrack, owner-census,
`qualify_control.py` as a runtime, `catalog-watch.exe` as a public
name, FR feature forwarding, compiling Local cheats out.

## Combined capability inventory

### User zip (release build)

`--locked --release --no-default-features -p panel --bin panel-play -p tui --bin tui-play`
after P5. Same names as today.

Keep / add: vault, profiles, Start/Stop, walk/bank/combat, MultiBox,
three-pass resource identity, `814e5293` prepare workers, `--version`,
`--check`, redacted support bundle, honest F12, scene-2 `--smoke` +
receipt, `BOT_DEBUG`, `BOT_CPU`, V8 `load`, generated game-data JSON
(when E lands), Local-only debug cheats as a **runtime** Prod no-op.

Refuse: `--live`, `BOT_LIVE`, `BOT_MEMORY_N` (exit 2, named rebuild
feature). No `ScenarioRunner`, no givebank seeds, no counting
allocator.

### Contributor (scenario build)

Default features during P5 (`default = ["scenario"]`), or
`--features scenario`. `--live script_*`, registry, isolate-stores,
CoreWitness, qualify JSON, `catalog_watch` example with
`required-features = ["scenario"]`, e2e / host-play ignored LIVE
cells, `6c6bb2d5` terminal labels.

Same `Play` slot thread, `script_start_load`, Driver, nav, snapshot
publish, panel renderer as the user zip.

### Memory build

`--features memory-profile` (implies `scenario`). `memory::Run`,
`BOT_MEMORY_*`, N ∈ {1,16,32,128}, counting allocator on frontend
bins. Never the user zip. N=2 is IsolationPair, not `parse_n`.

### Outside every binary

SSH, fixture extract/initialize, staff givebank, remote cargo, NASM,
PowerShell `CopyFromScreen`, catalog-watch rename, auto-update,
crash-phone-home, this operator's hostnames and engine paths.

## Ordered implementation plan

Cards are for a later orch. This report does not create them. No
LIVE in any of them. Source landing is not catalog/frontend
acceptance.

Kinds: **quick** = wiring on existing types; **boundary** = crate
feature or file move; **extract** = shared controller.

### Compatibility release (stop after P12)

**P0 — Shared build identity (quick, user)**
Depends on: nothing.
Owns: `crates/panel/build.rs`, `crates/panel/src/build_info.rs`,
thin stamp module reused by `tui` (and host-play lib if tests need
it). Replace `date -u` with env `BUILD_TIME` or a Windows-safe
`SystemTime` format. Bake host commit, `CLIENT_COMMIT`, dirty,
target triple. `panel-play --version` / `tui-play --version`. Dim
line unchanged; client SHA on hover/`--version`.
Outcome: a tarball without `.git` still stamps when env is set.
Acceptance: stamp helper + `"unknown"` fallback unit tests.
`cargo test -p panel --lib` / `-p tui --lib`.

**P1 — host-play `scenario` feature + required-features (boundary)**
Depends on: nothing (can parallel P0).
Owns: `crates/host-play/Cargo.toml`;
`required-features = ["scenario"]` on `catalog_boundary_live` and
`world_boundary_live`. `memory-profile = ["scenario", "script/memory-profile"]`.
Panel/tui still unconditional.
Outcome: none for users. Default `cargo test -p host-play --lib`
compiles.
Acceptance: `cargo test -p host-play --lib`;
`cargo test -p host-play --features scenario --test catalog_boundary_live -- --list`.
Do not `cargo test --all-features` as the gate.

**P2 — Move `shot.rs` into panel; release `--smoke` without runner (boundary, user)**
Depends on: P1 optional; do not start P5 before this.
Owns: `crates/scenario/src/shot.rs` → `crates/panel/src/shot.rs`
(or `window.rs` neighbor). F12 / `pump_shots` stop importing
`scenario::shot`. `--smoke` watches focused `ingame && scene_state == 2`
and enqueues; it must not construct `render_smoke` /
`ScenarioRunner` in the user build.
Outcome: user zip can F12 and `--smoke` after P5.
Acceptance: moved naming/write tests; smoke table tests still pass;
panel lib tests compile without constructing a runner for `--smoke`.
No LIVE. Hotspot: `crates/panel/src/app.rs`.

**P3 — Honest capture envelope + F12 (quick, user)**
Depends on: P2 (envelope lives next to the writer).
Owns: panel envelope type (`kind`, `label`, `ingame`, `scene_state`,
`tick`, `snapshot` or null, `snapshot_honest`, `window:
whole_window_imgui`, optional `error`). `enqueue_manual_shot` uses
the focused snapshot or `snapshot: null`. Never serialize
`GameSnapshot::new()` as observed state. `snapshot_honest` is false
when readback failed or `scene_state != 2` and the caller asked for
gameplay proof.
Outcome: F12 is not a lie. Last-FBO freeze at `scene_state==1`
stays annotated, not "fixed".
Acceptance: unit test that default snapshot is not honest game
state; focused vs null F12.

**P4 — Run manifest + hashes (quick, user)**
Depends on: P2.
Owns: `manifest.json` in the run dir (profile, revision, host/client
commits if the session knows them, stamp, pid, `274BOT_SMOKE_DIR`).
SHA-256 of each PNG/sidecar after `write_shot`. App records
`CARGO_PKG_VERSION` / baked identity; it does not shell out to git
at runtime.
Outcome: Python launchers hash the dir after exit; the writer also
records hashes.
Acceptance: temp-dir test with a 2×2 PNG.

**P5 — Optional `scenario` on panel/tui, default on (boundary)**
Depends on: P1, P2 (smoke/F12 no longer need the crate).
Owns: panel/tui `Cargo.toml`
`default = ["scenario"]`,
`scenario = ["dep:scenario", "host-play/scenario"]`,
`memory-profile = ["scenario", ...]`. Gate `--live` parser,
`LiveBoot::{Script,Null,Stress}`, `live_prepare_script`, TUI twin.
Release binary: `--live` / `BOT_LIVE` → exit 2; `BOT_MEMORY_N` set →
exit 2; `--help` lists only present flags. `catalog_watch`
`required-features = ["scenario"]`. Packager for the user zip
becomes `--no-default-features`. `docs/harness.md` still shows
`cargo run -- --live` (works because default is on).
Outcome: contributor muscle memory preserved; user zip can be lean.
Acceptance: `--no-default-features -- --live script_thiever` exit 2;
`--features scenario` still parses `script_thiever`;
`cargo test -p panel --lib` and `-p tui --lib` without scenario
cover Start/Stop/prepare/identity. No cfg on Play protocol,
snapshot, walk, bank, login, or `ingame && scene_state == 2`.

**P6 — `--check` + path/vault hygiene (quick, user)**
Depends on: P0 (identity fields in the printout). Can start after P0
in parallel with P1–P5 if it does not fight `app.rs` shot work.
Owns: host-play `profile.rs` + both bin parsers. `--check` is
resolve+bind+validate, print absolute cache/unpack/nav/vault paths
and hashes, exit 1 on missing pack / cache change / public-274 /
host-port drift. No GPU, vault open, or passphrase. Missing default
engine/cache/nav → name the path and the flag (`--cache`,
`--nav-pack`, `ENGINE_DIR`). Do not recommend
`~/experiments/...` as an install layout. Make `default_vault_path_for`
revision-aware **or** stop using the target-only helper on user
paths (bound `ServerProfile::vault_path` already has `vault-289`).
Outcome: a foreign machine can fail closed without this operator's
tree.
Acceptance: existing small profile fixtures — missing pack,
public-274 refusal, 274 vs 289 vault helper. Do not hash 1 GB packs
in CI.

**P7 — Support bundle + smoke receipt (quick, user)**
Depends on: P0, P3, P4, P6. Smoke receipt after P2's runner-free
`--smoke`.
Owns: `host_play::support_bundle::write(dir, facts)` denylist
(passphrase, vault bytes, `BOT_VAULT_PASS`, `private.pem`, IsolatedEnv
scratch). Panel "Copy diagnostics" / TUI key / `--support-bundle DIR`
on both bins. Optional last PNG from the shot dir, only with honest
envelope. `--smoke` writes `smoke.json` beside the PNG (identity,
profile hashes, elapsed as a clock not a benchmark, shot SHA, exit).
TUI live PASS/FAIL (scenario build) writes the same identity shape.
Process exit 0 without the receipt is not proof.
Outcome: users can send a redacted bundle; contributors keep
Python as a launcher that hashes the dir.
Acceptance: denylist unit test; receipt-shape unit test. No network.

**P8 — Results surface (quick, user + contributor)**
Depends on: P3 (envelope), P7 optional.
Owns: panel Results dock — last outcome, predicate, tile, scene, PNG
path, thumbnail from already-copied RGBA (no extra readback), last
error. TUI: last `Evidence` text on scenario builds; last session
error / `--check` on user builds. No TUI images. Results refresh
must not enqueue shots (smoke false-pass).
Outcome: headed FAIL is visible in-app, not only on stdout.
Acceptance: display-struct unit tests, not a live window.
Hotspot: `crates/panel/src/app.rs` — do not overlap with P2/P3/P5.

**P9 — Extract catalog proof library (extract, contributor)**
Depends on: P1 (feature exists). Do this **before** headed `--live`
grows a catalog path, so it cannot keep `RunnerStatus::Passed`.
Owns: new `host-play` module (cfg `scenario`) extracted from
committed `catalog_boundary_live.rs` at `6c6bb2d5` plus later
witness edits: `CoreCase`, identity ledger, `Observation` /
`CoreWitness` (including bank-fletcher cycle), `prepare_catalog_card`,
settings merge order, Start-baseline gates. Ignored test becomes env
parse + controller + assert. Move existing negative unit tests with
the extract.
Outcome: one witness module. No user-visible change.
Acceptance: first-burial, depleted-only, stale-bank, missing
transfer, seeded-only Alcher still fail. Unknown `CATALOG_SETTINGS_JSON`
keys fail. Core-contradicting explicit setting fails.
Hotspot: `crates/host-play/tests/catalog_boundary_live.rs` — extract
out of it; do not pile more body into the test file.

**P10 — `RunController` + headed/TUI catalog path (extract, contributor)**
Depends on: P5, P9.
Owns: `crates/host-play/src/run_control.rs` (name flexible).
`RunSpec` / `RunController` / `RunOutcome { Passed { qualify }, Failed, Cancelled }`.
`live_script_tick` / TUI `live_status` call `poll`. Proof kinds:
`Scenario` (runner Passed, host-driven nav/e2e), `CatalogCore`
(exactly one Start **and** Start baseline **and**
`CoreWitness::qualify()` **and** runner not Failed), later
`MemoryFleet` / `IsolationPair` not in this card. Frontends do not
decide PASS. Host-driven `Scenario` cells unchanged.
Outcome: headed BoneBurier cannot exit 0 on first burial.
Acceptance: controller unit — `poll` on runner Passed without
witness is Failed for `CatalogCore` and Passed for `Scenario`.
Cancel yields `Cancelled`, never Passed (even if a later card wires
Ctrl-C). Terminal-shot drain holds only an already-decided
Passed/Failed; it must not convert Cancel into PASS.
Hotspot: `crates/panel/src/app.rs` `live_script_tick` — after P8.

**P11 — `--isolate-stores` on production live-proof (quick, contributor)**
Depends on: P5, P10.
Owns: `--isolate-stores` default **on** for `--live` proof and
`catalog_watch`; **off** for interactive operator sessions. Enter
`IsolatedEnv` on the UI thread before slot spawn (`ensure_thread`).
Do not rewrite process `HOME`. Proof bags: empty overrides + inject,
never `ScriptSettingsStore::save`.
Outcome: production `--live` cannot write the operator's JS/settings.
Acceptance: prepare-path unit — js/script-settings resolve under the
pin and restore on drop. IsolatedEnv tests already prove HOME is
not process-mutated; keep them.
Promote `catalog_watch` to that default **or** keep it as a one-line
wrapper. Do not maintain two headed catalog entries. Example stays
`required-features = ["scenario"]`. Public name is never
`catalog-watch.exe`.

**P12 — Qualification JSON + sequential selected-run CLI (contributor)**
Depends on: P10, P11, P4 (run dir / identity fields).
Owns: per-cell qualify file (identity, account, settings, baseline,
witness, per-slot script state, outcome). Exit 0 only after
`Passed { qualify }`. A small launcher (host-play example or
`tools/`, scenario build) execs **one process per named cell**,
refuses if the receipt path exists, fails closed, does not treat
skip as PASS unless that exact identity already has `qualified:
true`. Replaces local use of `run_cell.py` for *proof*; Python may
remain a frozen-binary launcher on the operator machine.
Outcome: repeatable selected runs without a campaign DAG.
Acceptance: missing qualify file ⇒ fail; existing qualified receipt
is not overwritten. No in-process 20-cell matrix.

### Following increment (not this compatibility release)

| ID | Why later |
|---|---|
| F1 Headless `evidence.json` when sink is no-op | Useful; 01 already cut it from the stop line. |
| F2 Bounded in-process timeline (cap 256, not per-tick) | Python remains the clock until then. |
| F3 N=2 IsolationPair | Plan step 8; after P11. Not `parse_n("2")`. |
| F4 Ctrl-C / window close through `cancel` | After P10; outcome `Cancelled`. |
| F5 N=32 qualified Thiever | Existing `memory::Run` + sustain fixture. Memory build. |
| F6 Drop `default = ["scenario"]` | Habit/docs change; update `docs/harness.md` and launchers in the same card. |
| F7 Game-data spells/recipes/acquisition | Report 05 cards 4–6. |
| F8 Signed installers, auto-update, Linux panel-in-zip | Operator/infra. |
| F9 In-process catalog matrix, remote fleets | Forbidden, not deferred. |

Root-owned, never these cards: native Mac/Windows paint-during-prepare
after `814e5293`; changed-resource refusal on full-size packs; LIVE
headed/headless catalog cells; zip packaging on a builder; machine
backups.

### Parallel: generated game-data (section E)

Not a P0–P12 dependency. Same compatibility release, different
ownership.

- **G1** runtime serde load + Arc on `ServerProfile` + one-shot JS
  publication; qualified food/pickpocket (05 cards 1–2). **Release**
  — do not hide behind `scenario`. Alcher cost and food heals are
  gameplay.
- Generator / verify / pins stay `tools/game-data` on the operator
  machine. Binaries consume committed JSON; they do not shell out to
  Node or git against engine checkouts.
- Hotspot: `crates/api/src/content.rs`, `crates/script/src/shim/mod.rs`,
  `crates/script/src/load.rs`. Do not pile harness work onto those
  files.

## Build and packager matrix

Optimization profile `--release` is not a product class.

| Artifact | Command | Ships |
|---|---|---|
| User zip | `cargo build --locked --release --no-default-features -p panel --bin panel-play -p tui --bin tui-play` | yes, after P5 |
| Contributor / LIVE | `cargo build --locked --release --features scenario -p panel --bin panel-play -p tui --bin tui-play` (or omit `--features` while default stays on) | tree / CI, not the user zip |
| Memory | `cargo build --locked --release --features memory-profile -p panel --bin panel-play -p tui --bin tui-play` | never the user zip |

Packager (repo `tools/` or operator machine, **not** linked into
panel-play) records SHA-256, rustc, **feature list**, host/client
commits, target triple, build time. Zip binaries + LICENSE +
NOTICE. No engine, JAGs, nav packs, vaults, private keys, catalog
scripts, or fixture databases.

Names: `panel-play-{triple}` / `tui-play-{triple}`. Proof binaries
if retained use the same names plus sidecar `features: ["scenario"]`.
Env stamps: `GIT_COMMIT`, `GIT_DIRTY=0`, `CLIENT_COMMIT`, `BUILD_TIME`.
Do not require git inside an extracted tarball.

Until P5 lands, HEAD `cargo build --release -p panel -p tui` still
links scenario. That is today's configuration; this plan does not
change it.

## Crate ownership and hotspots

| Crate | This increment | Must not |
|---|---|---|
| `panel` | GPU camera, envelope, F12, `--smoke` watch, Results, `--version`/`--check` CLI, optional `scenario` feature | SSH, CoreWitness, a second wgpu path |
| `tui` | `--version`/`--check`, support bundle, evidence text, optional `scenario` | PNG camera, `date(1)` build stamp copy-paste without sharing the module |
| `host-play` (lib) | profile `--check`, support-bundle writer, `RunController` (cfg scenario), CoreWitness extract, `memory::Run` (cfg memory-profile) | UI, GPU, a new `[[bin]]` |
| `scenario` | seed/step/proof machine, registry, terminal_shot *labels* already landed | PNG writer after P2, catalog identity ledger, process exit |
| `script` | `IsolatedEnv`, `merge_bag`, pause/stop | proof predicates |
| `api` | game-data types when G1 runs | capture |
| `client` | last-FBO freeze | 274bot features forwarded in |
| `tools/game-data` | generator | runtime |
| Python / PowerShell | launch, frozen-binary receipts, platform SSH | PASS/FAIL, product camera |

Hotspots — later orch should not fan two implementers onto these:

- `crates/panel/src/app.rs` — P2, P3, P5, P8, P10
- `crates/scenario/src/lib.rs` — leave catalog bodies alone
- `crates/host-play/src/profile.rs` — P6
- `crates/host-play/tests/catalog_boundary_live.rs` — P9 extract
- `crates/api/src/content.rs` — G1 only

## Risks

1. Landing headed catalog `--live` (P10) before the extract (P9)
   preserves the first-burial PASS. Order is mandatory.
2. Optional-scenario (P5) before shot move (P2) drops F12/`--smoke`
   from the user zip. Order is mandatory.
3. cfg'ing a Play invariant behind `scenario` creates a second
   product. Blocking review finding.
4. Shipping `memory-profile` as the support zip.
5. Support bundle copying `~/.274bot/vault`.
6. Treating 584d05bc 198 s or 06b 37 s as a release budget, or as
   814e evidence.
7. Silent `~/experiments` defaults on Windows/Linux.
8. `--all-features` CI testing the no-alloc allocator.
9. Terminal-shot drain converting Cancel into PASS.
10. Byte/ms "lean" claims without a measurement. Forbidden here.
11. Two headed catalog entries (`catalog_watch` vs `--live`).
12. Joining hash workers on shutdown (reintroduces the stall).
13. Putting fixture extract or SSH behind a panel button.

## Focused validation (no LIVE from this card)

Offline, proportional, matching parent reports:

- P0/P6/P7: stamp fallback, `--check` missing nav/cache, bundle
  denylist, vault 274 vs 289.
- P3/P4: envelope honesty, 2×2 PNG hash.
- P5: `--live` exit 2 on `--no-default-features`; panel/tui lib
  tests without scenario; host-play lib default; catalog test
  `-- --list` with `--features scenario`.
- P9/P10: CoreWitness negatives; controller Passed-without-witness;
  `parse_n("2")` remains `Err`.
- P11: pin paths restore on drop.
- Existing host-play mismatch tests stay (`cache changed`, flags
  changed, bound session refuses revision change).

Later LIVE (root, scenario binary): one frozen headed cell and its
headless twin share `qualify` keys; exit 0 iff `qualified`; one
`--smoke` PNG whose envelope has `scene_state==2`; a human or
vision-capable tool reads the PNG (filename is not proof). That is
not this card and not P0–P12 acceptance.

Three-OS compile of the user command after P5 is an implementation
check, not a native beachball proof.

## Remaining operator decisions

Settled by this plan: no host-play.exe; keep `.274bot`; unsigned zip
for alpha; bundle attaches last honest PNG; client SHA is
hover/`--version`; user zip is `--no-default-features` after P5;
memory-profile never ships as support.

Still operator, not invented here:

1. Linux zip includes `panel-play` (wgpu) or is TUI-only.
2. Public label bump (`alpha 1` / crate `0.1.0` vs stated 0.1.7 /
   alpha 2) at tag time.
3. Whether to run F6 (default-off scenario) after the compatibility
   release.

## Suggested stopping point

Stop when P0–P12 have landed on top of `6c6bb2d5` labels and
`814e5293` prepare: user zip is identity + `--check` + honest F12 +
scene-2 `--smoke` + support bundle, without the runner; contributor
`--live` uses one proof kernel, isolated stores, and qualify JSON;
sequential selected runs are one process per cell; packager uses
`--no-default-features` for the user artifact and never
`memory-profile`.

Do not wait for N=2, N=32, an in-process timeline, installers, TUI
images, Windows HWND capture, dropping default features, or
game-data spells. Do not restart the memory campaign. Source
approval of this note does not imply live acceptance and does not
change the current release configuration.
)
