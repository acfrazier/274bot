# Release versus scenario build capabilities

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded architecture for brief 47, input to
synthesis `t_14b7a7e3`. Not implementation, not LIVE, not a release
tag, not a change to the current packager command, not a second
executable, not a restart of the memory campaign.

Read once: `AGENTS.md`, `docs/execution.md`, `docs/harness.md`,
`docs/compat/STATE.md`, `docs/compat/briefs/46-harness-architecture.md`,
`docs/compat/briefs/47-release-scenario-builds.md`, and reports
`01-capture-evidence.md`, `02-run-controls.md`,
`03-platform-delivery.md`, `05-generated-game-data.md`. Branch
checked first: `codex/rs2b0t-multirevision` (not `main`). Work was read-only except this report. No product edits,
LIVE, fixtures, STATE, sibling A–E files, subagents, stash, reset,
restore, checkout, merge, remotes, or release actions. Concurrent
uncommitted catalog/game-data WIP is not this report's product.

Operator premise (brief 47, mid-run clarification): development
facilities primarily serve us, script authors, and potential external
contributors. Ordinary users should receive a leaner gameplay
release. Keep the support diagnostics those users actually need, and
keep the same production script / protocol / navigation / state /
lifecycle / rendering paths. Footprint and startup wins are
hypotheses until measured.

## Verdict

**Split the overloaded `memory-profile` seam. Keep one binary name
per frontend. Ship the default feature set as the user release.
Build scenario and memory variants from the same crates.**

Do not add `panel-play-dev`, `catalog-watch.exe`, or a second
scenario engine. Do not bake counting allocators into a
"diagnostics" zip. Do not forward 274bot features into FR.

Three named *builds*, not three products:

| Build | Cargo | Audience |
|---|---|---|
| release | `--locked --release` default features | ordinary gameplay |
| scenario | `--locked --release --features scenario` | us, script authors, contributors, LIVE proof |
| memory | `--locked --release --features memory-profile` | allocation / fleet measurement |

`memory-profile` implies `scenario`. `memory-profile-no-alloc` is a
memory variant (system allocator, counters null). `--release` is an
optimization profile, not the product class.

Design approval is not source acceptance and not authorization to
change the current release configuration.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `41cfc85ef8fdb740ef881efe321f21ec2ec23b99` |
| Catalog terminal-shot / full bank loop | `6c6bb2d5c25d51f5b339d6bb5f73d68b3b7296cb` |
| Startup preparation | `814e5293fec426b722e450580c514b9cffca826f` |
| Client submodule HEAD | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 47 SHA-256 | `9fcf23c5d8e1008a314fdcddf669d4f714a153b1a54cd167b6f139a638036f63` |
| Brief 46 SHA-256 | `4a5a7df764e4c25fc6f1eeafeaa1ce316df2422a5f69397a9714ab60ce7192a9` |
| Brief 47 audience commit | `428e8e176e1aa836ad2280b2ce0bee541ee4de37` |
| Report 01 | `6b5e736a09e8aa432c144283295cccef975e9e67` |
| Report 02 | `54d80682a32e45f86a55017a271291a825709a1c` |
| Report 03 | `f31eeab5e90940fa7f1d76cdbbdc23fefd5dca32` |
| Report 05 | `4492b6e4631ca39685737aa9eb1aedd18776cbc5` |
| Kanban card | `t_7ec96964` |

`codex/memory-diagnostics` was not used as a requirement. Current
Cargo.toml / cfg sites are authoritative. Frozen source exports and
raw LIVE logs were not read.

## What current source actually does

### Cargo graph (verified)

Workspace `resolver = "2"`. Members: host, vault, api, host-play,
e2e, panel, nav, script, scenario, tui. No workspace `[features]`.
Dev profile optimizes `client` at `opt-level = 3`; bot crates stay
unoptimized in dev. That is a compile profile, not a capability.

| Crate | `scenario` dep | Features |
|---|---|---|
| panel | **unconditional** | `memory-profile` → `host-play/memory-profile`; `memory-profile-no-alloc` → both |
| tui | **unconditional** | same forwarding |
| host-play | **optional** `dep:scenario` | `memory-profile = ["script/memory-profile", "dep:scenario"]`; `memory-profile-no-alloc = ["memory-profile"]` |
| e2e | unconditional | none |
| scenario | (self) | none; always `image`/`png` |
| script | none | `default = ["load"]` (V8); empty `memory-profile` cfg flag |
| host, api, nav, vault | none | none |
| client (FR) | n/a | `default = []`; `window`; `audio`. Panel alone enables `audio`. |

host-play has **no `[[bin]]`**. Shipped binaries today are
`panel-play` and `tui-play`. `nav-pack` is a bake tool, not a
gameplay release. `panel/examples/catalog_watch.rs` is an example,
not a public name (report 03).

No crate sets `required-features` on any target. host-play
integration tests `catalog_boundary_live.rs` and
`world_boundary_live.rs` `use scenario::` with no `cfg`. Default
`cargo test -p host-play` therefore cannot compile those tests
without `--features memory-profile`. That is the existing accidental
coupling, not a documented gate.

Feature unification: enabling panel's `memory-profile` turns on
host-play's optional scenario. Enabling panel without that feature
still **builds and links** `scenario` because panel lists it as a
normal dependency. Workspace presence of e2e/tui does **not**
activate host-play's `dep:scenario`. `--all-features` on panel/tui
enables `memory-profile-no-alloc`, so the counting allocator is
`System` and JSON allocator fields are null. Do not use
`--all-features` as a coverage proxy.

Nothing in 274bot forwards `memory-profile` or `scenario` into FR.
Do not start.

### Runtime versus features

| Knob | Kind | Effect today |
|---|---|---|
| `cargo build --release` | optimization profile | LTO-off rustc opt; not a product class |
| `--features memory-profile` | Cargo | counting `#[global_allocator]` on panel/tui; `host_play::memory`; `script::memory_profile`; host-play links scenario |
| `--features memory-profile-no-alloc` | Cargo | same minus counting (System allocator) |
| `--live` / `BOT_LIVE` | CLI / env | panel/tui always parsed; scenario registry + mint + runner |
| `--smoke` | CLI (panel) | `render_smoke` scenario + scene-2 shot + exit |
| `BOT_MEMORY_N` | env | no-op unless memory-profile binary; then fleet harness |
| `BOT_DEBUG=1` | env | stderr script/host logs; always compiled |
| `BOT_CPU=1` | env | CpuPix3D; always compiled |
| `BOT_TARGET` / `--prod` | env / CLI | world host; cheat_allowed is Local-only |
| `LIVE=1` | env | ignored e2e / host-play live cells |
| `274BOT_SMOKE_DIR` | env | shot root |

`BOT_MEMORY_N` unset is normal play even on a memory-profile binary
(`host_play::memory` docs). The allocator is still installed for the
whole process once the feature is on.

### Use sites (production versus harness)

Ordinary interactive play does **not** go through
`ScenarioRunner`. Panel/TUI unlock a vault, spawn `Play` slots,
Start catalog JS through `script_start` / `load`, walk/bank/combat
through host-play `Driver` + nav. `814e5293` prepare workers are
panel startup, not scenario.

`scenario` is used for:

- `--live script_<name>`: `scenario::get`, minted ephemeral vault,
  `live_prepare_script`, `ScenarioRunner` on the session, PASS/FAIL
  from `RunnerStatus` (headed also drains terminal shots).
- panel `--live` extras not in the registry: `null_raster`,
  `stress50`, `stress50_full`, `nav_full`.
- panel `--smoke`: `render_smoke` + scene-2 GPU shot.
- F12 / `pump_shots`: `scenario::shot::{stamp_utc, create_run_dir,
  write_shot, safe_label}` only. Comments already say the F12 path
  has "no scenario involved" except those helpers.
- host-play `memory.rs` (cfg memory-profile): sustained Thiever
  scenario, `BOT_MEMORY_*` fleet.
- e2e ignored LIVE nav/walk cells.
- host-play ignored catalog/world cells.
- Scenario definitions embed fixture cheats (`givebank bones 28`,
  lobster/food/runes/logs, `~completequests` dialog drain).
  `api::interact::cheat` itself is production protocol, no-op unless
  `BotTarget::Local`.

Panel debug UI (`session.debug_ui` / `cheat_focused`) is runtime
Local-host only: TutSkip, `~home`, tele, setstat list. It is not
behind `memory-profile`. Prod sessions do not show it.

`script::IsolatedEnv` is always compiled. Production
`panel-play --live` does **not** enter it; `catalog_watch` does.
Report 02 already names that split.

Tracing crate: not a dependency. Bounded traces are `BOT_DEBUG`
and cfg'd `memory_diagnostics` (memory-profile only).

Examples: only `crates/panel/examples/catalog_watch.rs`.

## Capability classification

Audience: **user** = ordinary gameplay zip; **dev** = us / script
authors / contributors.

| Capability | Class | Build |
|---|---|---|
| Vault, profiles, Start/Stop script, walk, bank, combat, MultiBox | user | release |
| Immutable bind / hash identity / `814e5293` prepare | user | release |
| `--version`, `--check`, redacted support bundle (report 03) | user | release |
| Manual F12 GPU capture + honest sidecar (report 01) | user | release |
| Headed `--smoke` scene-2 receipt (report 03) | user | release |
| `BOT_DEBUG`, `BOT_CPU`, resource card CPU/RSS | user | release |
| Local-only debug cheats in the panel (runtime Prod no-op) | user, Local | release |
| Catalog JS `load` / V8 | user | release (required to play) |
| client `audio` (panel) | user | release |
| `--live script_*`, registry, `ScenarioRunner`, givebank seeds | dev | scenario |
| `null_raster` / `stress50*` | dev | scenario |
| Headed/TUI catalog proof, isolate-stores, qualify JSON (report 02) | dev | scenario |
| `catalog_watch` example | dev | scenario |
| e2e / host-play ignored LIVE cells | dev | scenario (e2e already depends) |
| `6c6bb2d5` terminal_shot labels + CoreWitness bank loop | dev | scenario |
| Counting allocator, `memory::Run`, `BOT_MEMORY_*`, N=32 | dev | memory |
| `BOT_MEMORY_DIAGNOSTICS` sidecars | dev | memory |
| SSH, fixture extract, staff givebank, remote cargo | outside | never in any binary |

Keep cheats as a **runtime** Local gate in release. Do not compile
them out: the packet path is shared, and a missing symbol on Local
would be a different product. Compile out the **fixture catalog**
(givebank seeds, minted proof accounts, automated runner).

Do not make a shell helper mandatory for F12, `--smoke`, `--check`,
or `--version`.

## Recommended arrangement (smallest useful)

One new Cargo feature named `scenario`. Reuse the existing
`memory-profile` name for the allocator/fleet overlay.

host-play:

```toml
[features]
scenario = ["dep:scenario"]
memory-profile = ["scenario", "script/memory-profile"]
memory-profile-no-alloc = ["memory-profile"]
```

panel / tui:

```toml
[features]
default = ["scenario"]          # stage 2; see migration
scenario = ["dep:scenario", "host-play/scenario"]
memory-profile = ["scenario", "host-play/memory-profile"]
memory-profile-no-alloc = ["memory-profile", "host-play/memory-profile-no-alloc"]
```

Same `[[bin]]` names. Packager for the user zip:

```sh
cargo build --locked --release --no-default-features \
  -p panel --bin panel-play -p tui --bin tui-play
```

Contributor / LIVE:

```sh
cargo build --locked --release --features scenario \
  -p panel --bin panel-play -p tui --bin tui-play
# or omit --features during stage 2, because default=["scenario"]
```

Memory:

```sh
cargo build --locked --release --features memory-profile \
  -p panel --bin panel-play -p tui --bin tui-play
```

Why not a new flag on one fat binary? `--live` is already a runtime
flag, but the registry, runner, `image` crate, and givebank seeds
link whether or not you pass it. Ordinary users should not pay that
at link time. Why not a duplicate executable? Same Play/script/GPU
paths would drift; report 02 already forbids a second engine. Why
not keep tying scenario to `memory-profile`? Script authors need
`--live` without installing a process-wide counting allocator.

### Narrow crate-boundary adjustment

`crates/scenario/src/shot.rs` is the only scenario API the release
binary should keep using (F12 / `--smoke` PNG). It pulls `image`
and naming helpers, not the runner. **Move shot write/stamp/run-dir
into panel** (the only GPU camera; report 01: TUI has no shot).
Keep a thin re-export or copy of the pure naming tests. e2e headed
shots, if any, call panel's writer or keep a `scenario` feature
`shot` later — do not add a third crate for this.

`--smoke` in release must **not** require `render_smoke` /
`ScenarioRunner`. LiveSmoke already latches `ingame &&
scene_state == 2` and waits for `pump_shots`. Implement release
`--smoke` as that watch plus F12-style enqueue. Scenario-build
`--smoke` may keep the registry path during migration, then converge
on the same watch so PASS cannot silently mean "runner Passed".

`host-play` `memory.rs` stays cfg `memory-profile`. Report 02
`RunController` / CoreWitness extract, when implemented, is cfg
`scenario`, not `memory-profile`. Do not fold catalog proof into
`memory::Run`.

### Disabled-command behavior

On a release (`--no-default-features`) binary:

| Input | Behavior |
|---|---|
| `--live …` / non-empty `BOT_LIVE` | exit 2, stderr: built without scenario; rebuild with `--features scenario` |
| `BOT_MEMORY_N` set | exit 2, same class of message for `--features memory-profile` (fail closed; do not silently become interactive play) |
| `--smoke` | still runs (scene-2 + GPU shot + receipt) |
| F12 | still queues a manual shot |
| unknown flags | unchanged exit 2 |
| `--help` | list only flags this binary actually has |

Do not print a fake PASS. Do not ignore `--live`. Scripted
contributor runs that today are `cargo run --release -p panel --
--live script_thiever` keep working during stage 2 because
`default = ["scenario"]`. After stage 3 (optional) they must pass
`--features scenario`. Update `docs/harness.md` in the same
implementation card that flips defaults.

`catalog_watch` example: `required-features = ["scenario"]`.

host-play tests that `use scenario`: `required-features =
["scenario"]`. Catalog cell that also samples allocators:
`required-features = ["memory-profile"]`.

## Same product, not a fork

The split is allowed only if the scenario binary drives the **same**
`Play` slot thread, `script_start_load`, Driver, nav Traveller,
snapshot publish, and panel renderer as release.

Proof plan (implementation later; not claimed here):

1. No `#[cfg(feature = "scenario")]` on host-play protocol,
   snapshot, walk, bank, login, or `ingame && scene_state == 2`
   gates. Those stay unconditional.
2. Panel/TUI lib tests for Start/Stop, prepare, identity mismatch
   compile and pass **without** the scenario feature.
3. Scenario-feature tests call the same `Play` APIs; they do not
   grow a parallel client.
4. Catalog identity cells stay on `--features scenario` (or
   `memory-profile`) and still require CoreWitness after exactly one
   Start (report 02). Headed `--live` must not keep
   `RunnerStatus::Passed` as catalog success.
5. `6c6bb2d5` terminal labels and full bank-fletcher loop remain
   scenario-crate data. Release does not ship them and does not
   need them.
6. `814e5293` prepare stays in the release panel. Native Mac/Windows
   paint-during-hash remains root-owned; this report does not claim
   it.
7. Resource identity stays three-pass hash (report 03). No mtime
   shortcut behind either feature.

A cfg that removes a runtime check from release only is a product
fork. Treat that as a blocking review finding.

## How this fits 01 / 02 / 03

- **01 capture:** GPU camera and F12 envelope are release. Catalog
  `terminal_shot` arms are scenario. Honest empty-snapshot refusal
  is release (F12 today queues `GameSnapshot::new()`). No second
  backend.
- **02 controller:** extract + `RunController` compile only with
  `scenario`. Isolation pair and sequential selected runs are
  scenario. N=32 stays `memory::Run`. Interactive Pause/Stop stay
  release.
- **03 platform:** `--version` / `--check` / support bundle / smoke
  receipt are release. User zip features default /
  `--no-default-features` after stage 2; never `memory-profile`.
  host-play is still a library — do not invent a host-play.exe in
  this design. SSH/fixtures stay outside.
- **05 game-data:** `tools/game-data` stays a build-time operator
  tool, not a binary feature. Serde load of revisioned JSON on
  `ServerProfile` is ordinary gameplay (Alcher cost, food heals) and
  belongs in **release**. Do not hide generated facts behind
  `scenario`.

Synthesis (`t_14b7a7e3`) should treat the packager command as
`--no-default-features` for user artifacts, `--features scenario`
for proof artifacts, and `--features memory-profile` only when a
memory run is requested. Size/startup deltas are hypotheses: do not
put a byte or millisecond claim in the combined plan.

## Staged migration (implementation later)

Not authorized by this card. Suggested ownership for D:

1. **host-play feature split + required-features.**
   `crates/host-play/Cargo.toml`, cfg on `memory.rs` unchanged,
   tests `catalog_boundary_live` / `world_boundary_live` get
   `required-features = ["scenario"]`. Panel/tui still
   unconditional. No user-visible change. Verify:
   `cargo test -p host-play --lib` (default) and
   `cargo test -p host-play --features scenario --test catalog_boundary_live -- --list`.
2. **Move `shot.rs` into panel; release `--smoke` without runner.**
   `crates/scenario/src/shot.rs` → `crates/panel/src/shot.rs` (or
   `window.rs` neighbor). Panel F12/`pump_shots` stop importing
   `scenario::shot`. Unit tests move with the file. `--smoke`
   watch uses SlotStatus + enqueue. No LIVE.
3. **Optional `scenario` on panel/tui with `default = ["scenario"]`.**
   Gate `--live` parser, `LiveBoot::{Script,Null,Stress}`,
   `live_prepare_script`, TUI twin. Disabled-command tests for
   `--no-default-features -- --live script_thiever` (exit 2).
   Packager doc: `--no-default-features` for user zip. `docs/harness.md`
   still shows default `cargo run -- --live` (works).
4. **Memory compose.** Confirm `memory-profile` implies `scenario`
   on all three crates; `BOT_MEMORY_N` on a release binary exits 2;
   counting allocator still only on frontend bins.
5. **Optional later:** drop `default = ["scenario"]` once in-tree
   scripts pass `--features scenario`. Stop here unless that
   surprise is wanted.

**Suggested stopping point:** stages 1–4. That yields a lean user
zip without breaking current `cargo run -- --live` muscle memory.
Stage 5 is a docs/habit change, not a capability.

Artifact naming (packager, not this crate):

- `panel-play-{triple}` / `tui-play-{triple}` — user, default
  features / `--no-default-features`
- sidecar JSON already named in report 03 (host/client commit,
  SHA-256, triple, rustc, **feature list**)
- proof binaries, if retained: same names plus sidecar
  `features: ["scenario"]` — do not ship in the user zip
- memory binaries: never the user zip

Which variants actually ship: **user zip = release build only.**
Scenario/memory binaries stay in the contributor tree / CI cache.

## Verification matrix (risk-proportional, no LIVE here)

| Check | macOS | Windows | Linux |
|---|---|---|---|
| `cargo build --locked --release --no-default-features -p panel -p tui` | yes | yes | yes |
| nm/strings: no `givebank`, no `ScenarioRunner`, no `CountingAllocator` in user binary (hypothesis until measured; implementation card may sample) | later | later | later |
| `--live` exit 2 message | unit | unit | unit |
| `BOT_MEMORY_N=1` exit 2 on user binary | unit | unit | unit |
| `--smoke` / F12 still link | compile | compile | compile |
| `--features scenario` `--live` parser accepts `script_thiever` | unit | unit | unit |
| `cargo test -p panel --lib` default (gameplay/prepare) | yes | yes | yes |
| `cargo test -p host-play --lib` default (no scenario tests) | yes | yes | yes |
| `cargo test -p host-play --features scenario --test catalog_boundary_live` list/offline witnesses | yes | as CI | as CI |
| `cargo test -p tui --lib` | yes | yes | yes |
| Do not `cargo test --all-features` as the gate | — | — | — |
| LIVE catalog/world/e2e | root, scenario binary | root | root |

No benchmark. No byte-size claim.

## Tradeoffs

- **default=["scenario"] vs default off.** Default on preserves
  today's contributor commands and makes the packager responsible
  for `--no-default-features`. Default off makes `cargo run`
  lean and breaks `docs/harness.md` until edited. Pick default on
  (stage 2) because the audience split is about the *shipped zip*,
  not about tripping contributors.
- **Moving shot.rs vs feature-gating inside scenario.** An internal
  `shot` feature still compiles the 3.7k-line registry if panel
  depends on the crate. Optional-dep of the whole crate plus moving
  shot is the actual lean path.
- **Keeping Local cheats in release.** Slightly larger attack
  surface if someone points a "release" build at Local; already
  true today, already no-op on Prod. Compiling them out would hide
  TutSkip from Local operators and risk cfg drift on the cheat
  packet.
- **V8 stays in release.** It is how catalog scripts run. Gating
  `load` would ship a client that cannot Start. Not lean in a
  useful way.
- **host-play remains a lib.** Report 03's packager `-p host-play`
  does not produce a gameplay binary. Do not add one here.

## Risks

1. Headed `--live` keeps `RunnerStatus::Passed` as catalog success
   (report 02). Feature split does not fix that; controller extract
   does.
2. `--all-features` CI silently tests the no-alloc allocator.
3. Forgetting `required-features` leaves default host-play tests
   red.
4. cfg'ing a Play invariant behind `scenario`.
5. Shipping memory-profile as the "support" zip (report 03 risk 6).
6. Treating hypothetical binary-size drop as acceptance.
7. Stage 5 default-off without updating harness.md / Python
   launchers.
8. Enabling client `window` or other FR features "for diagnostics".

## Stopping point (human)

Give ordinary users the same `panel-play` / `tui-play` names with
default features stripped at packager time, and give contributors
`--features scenario` (and `memory-profile` when measuring). Stop
before extra binaries, FR feature forwarding, compiling Local cheats
out, or claiming RSS/startup wins. This note does not change the
current release configuration.
