# Architecture

> **Developer reference.** This page is for people who change 274bot.
> To use 274bot, start at the [README](../../README.md); to find another page, see the [docs index](../README.md).

274bot is a Rust bot host over a vendored client library. Ownership is crate
and runtime boundaries, not a line-count budget and not a copy of a TypeScript
lint stack.

0.2.8 Beta 2 is the current release. Revision 289 is the production revision: the product is
built, tested and claimed for it. Revision 274 is best-effort: it may run,
but it is neither tested nor claimed.

The allowed crate edges are defined in the operator-owned
[`274bot-arch` policy](https://github.com/acfrazier/274bot-arch/blob/main/policy.toml)
and checked by its `check.py`. Runtime semantics that a manifest
cannot prove stay **review requirements**. Changing a crate edge is a
deliberate rule change in `274bot-arch` (proposed as a patch or branch bundle
for the operator to review and push) plus documentation and review here; do
not sneak a reverse or convenience dependency through an alias, target table,
or optional rename.

## Local command

Python **3.11+** (stdlib `tomllib`). No Rust rebuild. The checker, its
tests, and the graph-creating policy live only in [`acfrazier/274bot-arch`](https://github.com/acfrazier/274bot-arch)
`main`, as decided in ADR 0010. See the
[checker README](https://github.com/acfrazier/274bot-arch/blob/main/README.md)
for its runtime pins and rule-change process. This repo holds the
local-convenience-only runner `tools/architecture/run.sh`, which fetches
`274bot-arch` `main` into a local cache and runs it against a tree. CI
does not use the runner and never executes a candidate-tree script. The
fine data the checker reads — exactly `tools/architecture/ownership.json`,
`tools/architecture/inventories.json`, and
`tools/architecture/foreign_types.json` — stays in this repo beside the
runner and is reviewed like code. Everything that creates or activates the
graph stays protected: workspace members, edges/kinds/migrations, the
configuration feature/activation fields (including optional-dependency
activation), and the graph provenance baseline.
Agents and Main use the same runner locally.

```bash
tools/architecture/run.sh
tools/architecture/run.sh --root /path/to/tree
```

With no `--root`, the runner checks the product root it lives in.
`ARCH_REPO` is a local-only override of where the rules come from: a URL
(the default remote is always `main`) or a local clone, whose selected
local `HEAD` is used as pre-push proof of an unpushed rule change. CI has
no `ARCH_REPO` override. `ARCH_CACHE_DIR` overrides the local fetch cache,
a bare Git cache defaulting to
`${XDG_CACHE_HOME:-$HOME/.cache}/274bot-arch`.

The runner checks the rust-analyzer executable on `PATH`: SHA-256
`fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49`,
`rust-analyzer 0.3.3081-standalone`. The checker reads
workspace, member, and protected path-package `Cargo.toml` files. It scans
`[dependencies]`, `[dev-dependencies]` / `[dev_dependencies]`, and
`[build-dependencies]` / `[build_dependencies]`, including target tables.
It resolves `optional`, `workspace = true` aliases, `path`, and `package`
renames, plus path `[patch]` and `[replace]` entries. It does not compile
candidate code or parse `.rs` with regex.

Product membership uses explicit `workspace.members` paths plus a root `[package]`
when present (Cargo's implicit root member). Product workspace `members`,
`default-members`, and `exclude` entries must resolve to protected manifests.
Product member globs fail closed.
Every non-member path
package must have a canonical manifest binding in protected
`policy.toml` `[workspace.path_packages]`. Unlisted path packages and symlink
substitutions are refused. Every listed manifest is scanned, even when unused.
Edges from listed packages need ordinary protected allowances.
The checker refuses a `git` key in any dependency table, target table,
`[workspace.dependencies]`, `[patch.*]`, or `[replace]` entry.
The refusal names the manifest file and the entry.
The checker never reads a git crate's manifest.
A git crate could carry a crate edge or a feature request into a local package.
Only root `[patch.*]` and `[replace]` entries listed by name and path in
protected `policy.toml` `[root_patches]` may appear in the product root.
The list now holds only `dear-imgui-wgpu` at `vendor/dear-imgui-wgpu-0.15.1`.
A root patch makes a local package reachable from every external crate.
Any other root patch or replacement needs an operator commit that adds it.
Registry dependencies stay allowed. A crates.io crate cannot name a local path package without a root patch, and the root patch list covers that route.

Every `Cargo.toml` below `vendor/fr-client-rust/` is pinned in protected
`274bot-arch/vendor-manifests.json`. One digest covers the sorted path and exact-byte pairs.
The digest is identity data for a protected input, like policy data, not bookkeeping.
The checker names every added, removed, or changed manifest and requires an operator commit to update the pin.
It does not interpret the client workspace's members, exclusions, globs, or inheritance.
The pinned set has no `workspace = true` dependencies, and such dependencies are refused.
A fork pointer change with byte-identical manifests needs no pin update.
Print the digest with `python3 -B vendor_manifests.py /path/to/274bot` from the arch repository.
Generate a reviewed replacement with
`python3 -B vendor_manifests.py --emit-pin /path/to/274bot > vendor-manifests.json`.

The checker refuses `.cargo/config` and `.cargo/config.toml` anywhere in the
candidate tree, including vendored trees and symlinks. Directory and file names
are matched without ASCII case distinctions. These files can override the graph.
The checker also refuses an ASCII case variant of `Cargo.toml` anywhere in the tree, because it replaces the scanned manifest on a case-insensitive file system.
Unicode look-alike names are outside this refusal because Cargo on Linux does not read them.
One limit stays with review. A product build script can rewrite a vendored
manifest on disk during a CI job, between steps.
The checker reads committed manifests only. It does not run build scripts.
A new `build.rs` needs an ownership row, so a reviewer sees it.
Dependency aliases are tracked by package name, not by alias or version.
Default-feature and dev-dependency activation is checked for every protected
member and listed path package, even when no named configuration reaches it.

GitHub Actions pins Python **3.14.8** and runs the operator-owned check
inline in one `architecture` job on pushes and pull requests: it keeps the
pinned rust-analyzer and `flatc` installation steps, then the check step
itself runs `git clone --depth 1 --branch main
https://github.com/acfrazier/274bot-arch.git "$RUNNER_TEMP/274bot-arch"`,
verifies the rust-analyzer binary (SHA-256
`fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49`,
`rust-analyzer 0.3.3081-standalone`), then runs `python3 -B
"$checker_root/check.py" --self-test` and `python3 -B
"$checker_root/check.py" --root "$GITHUB_WORKSPACE"`. CI never executes a
candidate-tree script (including `tools/architecture/run.sh`) and has no
 `ARCH_REPO` override. It also installs the pinned `flatc` and diffs
 `crates/script-js/schema/generated` against a fresh `flatc --rust` of
 `crates/script-js/schema/isolate.fbs`. That job does **not** replace `fmt`,
`clippy`, or `test`. Those gates stay as they are.

The explicit allowlist is [`policy.toml` in `274bot-arch`](https://github.com/acfrazier/274bot-arch/blob/main/policy.toml):
workspace members, edges with their kinds and migrations, and the
configuration feature/activation fields, all protected. The fine data lives
beside the runner and is reviewed like code:
[`tools/architecture/ownership.json`](../../tools/architecture/ownership.json),
[`tools/architecture/inventories.json`](../../tools/architecture/inventories.json),
[`tools/architecture/foreign_types.json`](../../tools/architecture/foreign_types.json).
The checker loads that data from the checked tree, refuses malformed or
stale data, and cannot widen the protected edge policy.
Every workspace member must appear in protected `policy.toml`. Every edge
the manifests declare needs an `[[allow]]` or migration with its kind.
Unknown members and unlisted edges fail closed. crates.io and other
non-path crates are ignored. `client`
is a workspace *alias* to `vendor/fr-client-rust/crates/client`, not a 274bot
workspace member; it is listed as an architecture external.

## Production crate ownership

| Crate | Owns | Must not become |
| --- | --- | --- |
| `vault` | Encrypted profile store, secret strings, owner-only file helpers | A dependant of api/host/ui |
| `api` | Host-facing read/act/settle types mapped onto `client`; selected game facts | A dependant of host, nav, script, or UI |
| `host` | Native slot/tick APIs, login FIFO, guardian | A dependant of nav, script, host-play, or UI |
| `nav` | Packed world, router, map data | Script isolate or operator UI |
| `nav-executor` | Pollable `Traveller` follow run, walk and transport hops | Planner search, script isolate, or operator UI |
 | `script` | Native card machines (Quester, Gatherer, Combat), slot lifecycle and typed native facades over the packed world | A production dependant of `client`; a JS runtime owner |
 | `script-js` | JS runtime: V8 isolate, compat machines (including Sherlock and the clue machine), catalog and library, isolate wire schema and JS assets | A native card owner; a production dependant of `client` except through dev fixtures |
| `host-play` | Shared `Play` lifecycle over host/script/nav/vault | A second panel or client renderer |
| `frontend-core` | Operator lifecycle shared by panel and TUI: vault, fleet membership and logout latch, selected bot, Load/Log in/Log out/Remove, non-blocking removal, script Start/Stop settlement, operation results, the structured operator log; script coordination (catalog, per-profile assignment and parameters, Start all/Stop all, reload, Apply to all); fleet marks and the commands on marked bots (Assign, log in/out, group walk); the fleet/detail projections (phase, queue place, retained errors, newest operation) and the one process resource sampler both front ends render | A second `Play`, login queue, world or script runtime; a dependant of panel/tui or of window/terminal libraries; a per-row or per-bot sampler |
| `scenario` | Shared headed/headless live scenario runner and Quester fixtures | Panel/TUI chrome |
| `panel` | Native ImGui UI, winit/wgpu window, game blit | The client 3D renderer or isolate runtime |
| `tui` | Headless operator view over the same `frontend-core` session | A second kernel or GPU loop |
| `e2e` | Wide **test orchestration** (library, `e2e-suite` binaries, live tests) | Production UI or script-kernel ownership |
| `client` (external) | 274/289 client lib, GPU/CPU raster, last-FBO | Bot action API or 274bot crates in its repo |

Authoritative edges and reasons live in the policy file. The current
graph, derived from the manifests:

- `api` → `client`
- `host` → `api`, `client`, `vault`
- `nav` → `api`, `client`
- `nav-bake` → `api`, `client`, `nav`
- `nav-executor` → `api`, `nav`, `client`; `nav` also **dev** for planner test-support fixtures
 - `script` → `api`, `nav`, `vault` (runtime); `client` **dev-only**
 - `script-js` → `script`, `api`, `vault` (runtime); `client` **dev-only**; `nav-planner` **dev-only** (test fixtures only; production JS code uses the Core `script` nav facade)
 - `host-play` → `api`, `client`, `host`, `script`, `script-js`, `vault`; `nav` runtime and **build**; `nav-bake` **build and dev**; `nav-executor` runtime; `scenario` **optional** (and **dev**); `script` also **dev**; `script-js` also **dev**
 - `scenario` → `api`, `client`, `nav`, `nav-executor`, `script`
 - `frontend-core` → `api`, `host`, `host-play`, `nav`, `script`, `script-js`, `vault`; `client`, `host-play`, `nav-bake`, `scenario` **dev-only**
 - `panel` → `api`, `client`, `frontend-core`, `host`, `host-play`, `nav`, `scenario`, `script`, `vault`; `nav-bake` **dev-only**; `frontend-core` and `host-play` also **dev**
 - `tui` → `api`, `client`, `frontend-core`, `host-play`, `nav`, `scenario`, `script`, `vault`; `host`, `nav-bake` **dev-only**; `frontend-core` and `host-play` also **dev**
 - `e2e` → `api`, `client`, `host`, `host-play`, `nav`, `nav-bake`, `scenario`, `vault`; `host-play` also **dev**

 `e2e` does **not** Cargo-depend on `script`, `script-js`, `panel`, or `tui`. The suite
 launches product binaries as child processes. That is orchestration, not a
 claim that `e2e` owns those crates' production behavior. Sherlock is available
 only in builds with the JS runtime. Sherlock was meant to be native. In practice
 it became a port of the compatibility clue machine, so it needs the JS runtime
 build until the native clue rewrite replaces that machine. Sherlock itself never
 creates a V8 isolate.

### Truthful exceptions

- **`api` → `client`** is intentional. `api` maps host types onto the
  vendored client. It is not a reverse `client` → `api` edge.
- **`host` → `client`** also has a **dev-only** edge for the real WebSocket test connector used by the idle WSS park regression; client `test-support` is never enabled in the normal graph.
- **`script` → `nav`** is a runtime edge: native machines read the packed
  world, routes and zones through typed `nav` values. `script` → `client` is
  `[dev-dependencies]` only; promoting it to a normal/optional/target
  dependency fails the checker, so the compiled script crate stays
  client-free.
- **`script`** has no `nav-executor` edge. Script walks run through the host's
  `Traveller` owner and read plans through `nav_planner::contract`. `panel` has no
  `nav-executor` edge. Its one executor test builds through the `host-play`
  test-support seam `WalkArm::with_latched_essence`.
- **`tui` → `host`** is `[dev-dependencies]` only. Production TUI composes
  through `frontend-core` and `host-play`.
- **`frontend-core` → `host`** carries `SlotInput`/`FrameBuf` for
  `Play::try_spawn_slot`, and `host::debug_enabled()` for the catalog-registry warning.
- **`frontend-core` → `api`** carries read-only value types (selected game
  facts, snapshot rows, random-event kinds) and the `api::hostlog` facade:
  the operator log store is its sink.
- **`frontend-core` → `nav`** carries the route options, tile and world-state
  types the shared group walk and the walk permissions use.
- **`host-play` → `scenario`** is optional (feature-gated harness:
  `memory-profile`, and `live-harness` for the fleet launch panel and TUI
  share), not a default required edge. A required `[target.*.dependencies]` edge does
  **not** satisfy an optional-only allow. `optional = true` on a target
  table stays optional. `{ workspace = true, optional = true }` is read
  from the **member** table. `[workspace.dependencies]` cannot set
  `optional` (Cargo: "workspace dependencies cannot be optional"); that
  form is rejected rather than inherited.
 - **`host-play` → `nav`** is both a runtime dependency and a build-dependency
   (bake/stage nav identity). Both kinds are listed.
 - **`script-js` → `client` and `script-js` → `nav-planner` are `[dev-dependencies]` only.** Production JS code uses the Core `script` nav facade. Promoting either to a normal dependency fails the checker.
 - **`host-play` → `script-js` and `frontend-core` → `script-js` are normal edges.** `host-play` also declares a dev edge for test hooks. The slot binds the JS surface factory from `script-js`; hosts install its cards with `script_js::install_cards()`.

For a new edge, kind, migration, optional-dependency activation, vendored path
package, or checker change, follow [Changing a boundary](#changing-a-boundary).
Include the regenerated baseline in the same arch commit as the protected
rule change. Outside the pinned client manifests, purely local features without
optional or cross-crate activation need no operator commit. Ownership, inventory, and foreign-type rows remain
ordinary product diffs. Do not add a second hidden graph.

## Cargo features

| Crate | Features |
| --- | --- |
 | `script` | `canvas-raster`: raster, fonts and composition; `path-schema`: Path JSON Schema generation; `test-hooks`: integration-only seams. No `load` feature and no JS dependencies. |
 | `script-js` | JS runtime crate: V8 isolate, compat machines, catalog and library, isolate wire schema. `test-hooks`: integration-only isolate seams; `memory-profile`: V8 heap and snapshot counters for the host-play harness. |
| `api` | `path-schema`, `debug-catalog`, `test-hooks` |
| `host` | `render-diagnostics`, `performance-profile`, `journal-paint-proof` |
| `host-play` | `live-harness` (pulls in `scenario`), `memory-profile` and `memory-profile-no-alloc` (the fleet memory harness), `test-support`, `live-probe`, `debug-catalog`, `journal-paint-proof` |
| `panel` | `render-diagnostics`, `memory-profile`, `memory-profile-no-alloc` |
| `tui` | `memory-profile`, `memory-profile-no-alloc` |
| `frontend-core` | `test-support` (arm-only spawn fixtures for front-end tests) |

## Per-crate module ownership

Paths are relative to `crates/<crate>/src`. Rows name a module or a family of
modules; a row with a glob (`catalog_*.rs`) covers every file it matches.
`*_tests.rs`, `tests.rs`, `*/tests.rs`, and `include!`d `*_roundtrip.rs` files
are test bodies, not production owners, and are not listed one by one. A
directory with its own facade file (`foo.rs` beside `foo/`) splits that module's
reasons to change.

### vault

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | encrypted profile store | vault file format, profile rows |
| `compiled_settings.rs` | versioned compiled-card settings | inside the per-account settings map |
| `private_file.rs` | owner-only persistence helpers | create/replace files for the vault and its state files |
| `secret.rs` | `Secret` string | zeroed on drop; passwords and the passphrase |

### api

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | declares every module below |
| `snapshot.rs` | snapshot storage and rebuild | generation orchestration, read accessors |
| `snapshot/views.rs` | snapshot view rows | typed row shapes |
| `snapshot/decode.rs` | client decoding into views | packet bytes to rows |
| `snapshot/context.rs` | read context over a snapshot | query-time context |
| `snapshot/native.rs` | borrowed native observations | readiness attached to the evidence stamp |
| `query.rs` | borrowing family queries | `Query` builder and per-family traits |
| `query/reach.rs` | scene reachability | `SceneQuery::can_reach` options |
| `query/widget_search.rs` | widget and button lookups | snapshot-level |
| `query/loc_approach.rs` | loc approach mask | footprint plus wall flags |
| `interact.rs` | send vocabulary and dispatch | `Driver` seam, accepted-send semantics |
| `interact/driver.rs` | client transport mapping | `doAction`, `tryMove`, `out`, ISAAC intact |
| `hostlog.rs` | host logging facade | `host_log!` categories, slot-log allowlist, stderr under `BOT_DEBUG`, one process sink |
| `../build.rs` | compact embedded revision inputs | deterministic gzip of generated JSON into `OUT_DIR`; no generator or pin changes |
| `game_data.rs` | generated game facts by revision | buffered streaming decode and original-byte digest, indexes, lookups |
| `selected.rs` | shared selected-fact values | loading is off-pump; gates never guess missing facts |
| `content.rs` | shared rock names and loot predicate | revision facts stay in `game_data` |
| `ent.rs` | Ent NPC identity facts | revision-independent ids and lifetime |
| `obj_names.rs` | item and loc definition views | id-to-name table for scripts |
| `prot.rs` | legal send table and builders | writes via `Out`, never raw opcode |
| `settle.rs` | pollable settle evidence | host drives polls per tick |
| `native_input.rs` | input identity and revoke mutex | not mouse policy |
| `line_of_sight.rs` | line-of-sight ray and query order | not the Reach flood |
| `random.rs` | cross-crate random types | detect and act stay in `host` |
| `run_policy.rs` | script-run overlay of the auto-run policy | host owns the current value |
| `prayer.rs` | prayer queries | over fact rows and observed stats |
| `quest_facts.rs`, `quest_facts/catalog.rs` | quest identity and prereqs; roster rows | stage resolution belongs to the Path |
| `quest_progress.rs` | compact journal/progress evidence | shared by runner, host and nav provider |
| `shop_facts.rs` | shop preset facts | ShopBuyout presets only |
| `clue_facts.rs` | clue row lookup | over the posted trail family |
| `clue_logic.rs` | held-step identify | first held step in posted order |
| `clue_pack.rs` | trail pack budgeting and keep predicate | frozen pack-plan arithmetic |
| `clue_puzzle.rs` | sliding-puzzle plan | frozen grouped search over caller rows |
| `cake_stall.rs` | Baker-stall pins | posted facts, selection in `script` |
| `cook_locations.rs` | cook-location inputs | curated bank cook surfaces and pairing constants |
| `family_asset.rs` | checked selected family assets | one manifest/digest/schema admission, run once when a family is prepared |
| `gather_methods.rs` | gather JSON query adapters | rows and coverage over the typed catalog; absence is a token |
| `gather_methods/catalog.rs` | typed gathering catalog and queries | spot regions, pages, nearest, zones, rocks; unknown never empty |
| `gather_methods/wire.rs` | `gathering.json` decode | fail-closed schema-1 decode into the catalog |
| `gather_methods/cache.rs` | prepared-catalog lifecycle | embedded family, weak shared cache, off-pump preparation |
| `gather_tools.rs` | gather-tool identity, use, wield | posted facts |
| `named_banks.rs` | frozen bank catalog, eligibility and selected-content access rows | stand resolution lives in `nav` |
| `bank_memory.rs` | last-seen whole bank of one account | the type alone; the host fills it from the open bank |
| `stock.rs` | planning kernel over a frame's item pages | one exact-id count, one slot policy, one tri-state |
| `debug_commands.rs` | content-derived debugproc and engine-cheat metadata | |

Each revision's parsed game facts are shared once per process through `OnceLock`
and `Arc`. The executable retains only the compressed revision JSON, not a
second multi-MiB uncompressed source representation. Decode/hash uses a 64 KiB
buffer and verifies the same generated byte length and SHA-256 against the
unchanged manifest; compression does not change selected-content identities.

### host

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | slot pump and crate facade | one OS thread per slot |
| `render.rs` | neutral render reservation protocol (D12, §23 §2) | metadata only; no render thread; one process instance, owned by `render_head.rs` |
| `render_head.rs` | per-slot render heads over the process `RenderService` (T4.2 wrap) | `HeadLease` per drawing slot; GPU scene submissions under a `FrameLease` through the client's scene-workspace hook, CPU heads take none; the client's `GPU_SCENE_LOCK` wait is retained until T4.4 |
| `slot.rs` | per-slot drain pump | gens diff, tick synthesis, dirty families |
| `slot_io.rs` | frame and input channels | mailbox (accepts only its attached head's frames), input, park and wake |
| `login_queue.rs` | login FIFO and backoff | production rate limits |
| `auto_run.rs` | auto-run toggle | run-energy threshold |
| `performance_profile.rs` | per-slot host work timers | opt-in, `performance-profile` feature |
| `random.rs` | random detection and shared vocabulary | stateless detect, caller-owned cooldowns |
| `random/guardian.rs` | act, hold, and dialog state machine | trap holds, cooldown writes, knock edge |
| `random/maze.rs` | maze graph, route, and phase | port of the frozen maze logic |
| `random/maze/layout.rs` | static maze layout data | copied layout rows |

Render-only scratch follows the draw-head lifetime (T4.3 shed). The client's
`RenderWorld` owns the per-frame occlusion cache. The world creates its bounded
dynamic draw-sprite list only on first use and releases it during draw-off
cleanup. Attach reconstructs a head over the same Client and connection.
Ground stamps and simulation observation generations remain on the world.
An attached head still preserves its last 3D scene while `scene_state == 1`.
The synchronous GPU workspace wait remains a T4.4 limitation.

### nav-planner

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | declares every module below |
| `arrival.rs` | arrival detection | on tile or adjacent to solid |
| `bank_fetch.rs` | BankBudget fetch-and-wear plan | plan only, router stays fail-closed |
| `camera.rs` | path-facing orbit yaw | pure, host writes yaw per observe |
| `canlight.rs` | canlight lookup and identity binding | static policy and checked sidecar identity |
| `collision.rs` | runtime collision models | four planes, walk words, and shared packing primitives |
| `essence.rs` | essence-mine return latch | session-gated return hop |
| `grid.rs` | walkability grids | step-grid surface |
| `manifest.rs` | runtime manifest and identity schemas | hashes, verification, and `NavIdentityRow` |
| `map.rs` | bounded map-data boundary | errors, capped records/text and header-first checks; no UI or game actions |
| `map/identity.rs` | map identity byte layouts | independent image/catalogue keys; exact encoder/dependency policy |
| `map/poi.rs` | source-tagged discovery evidence | classifier, physical placements and separate nav-bound stands; not account eligibility |
| `map/spatial.rs` | map geometry and selection | north-up transforms, 24-slot LOD selection, radius-16 optional snap |
| `map/formats.rs` | checked map payload codecs | client catalogue, image manifest, shipped-terrain description and data-only `274P`/1 navpois |
| `map/cache.rs` | publication readers and checkpoints | typed partial/ready entries; bounded reads and requested-payload verification |
| `map/records.rs` | offset-only maps.bin access | header index plus one reusable record buffer, not an archive/world copy |
| `map/client_cache.rs` | bounded client-cache decoding | seek index shared by the terrain and catalogue producers |
| `map/catalogue.rs` | client-only POI derivation | isolated from terrain policy so classifier changes do not invalidate baked imagery |
| `map/raster.rs` | terrain raster and PNG pyramid | deterministic, sparse, bounded mapsquare neighbourhoods |
| `map/producer.rs` | pure producer entry points | host owns leases, locking, cancellation and publication |
| `map/services.rs` | navpois policy identity | one shared digest formula |
| `named_banks.rs` | selected-world bank stand resolution | captured account eligibility stays with the caller, no `script` dep |
| `pack.rs` | runtime pack decoding | version, geometry, length and identity checks |
| `pack/banks.rs` | bank schema and decoding | booth/NPC access data |
| `pack/sidecars.rs` | sidecar decoding and binding | flags, paint-reach, canlight |
| `pack/zones.rs` | zone table decoding | validated exclusion geometry |
| `paint.rs` | nav-paint buffers | no imgui, no client draw |
| `quest_gates.rs` | quest-stage gates on transport edges | typed gates over selected progress signals |
| `router.rs` | world-space Dijkstra | exact-coordinate compact storage, shared synchronous/cooperative kernel, total-ticks cost and gated edges |
| `movement.rs` | shared directional step rule | router and release producer use the same `step_ok` |
| `router/grid.rs` | compatibility step-grid A-star | retained for fixture tests; production travel uses the collision+transport router |
| `router/resources.rs` | resource labels on walk nodes | change only at paid hops |
| `tile.rs` | tile coords and distance | x, z, level |
| `transport.rs` | transport graph orchestration | types, ordering, shared cost and geometry |
| `transport/` | runtime fare, web-action and Wilderness helpers | source derivation belongs to `nav-bake` |
| `executor_adapter.rs` | raw forwarding surface for `nav-executor` only | plan-count/leg helpers, web/door predicates, shared constants; not part of `contract` |
| `test_support.rs`, `test_support/source_scan.rs` | planner fixture builders and source scanner | test scope plus `test-support` feature gates |
| `walk_destinations.rs` | shared walk-destination pins | town tiles for WalkTo confirm |
| `required-content-{274,289}.tsv` | shared canonical content inputs | Quester coverage, nightly staging, and the baker use the same data |
| `world.rs` | bound world handle | packed collision plus transport graph |
| `world_state.rs` | search gating facts | fail-closed edge requirements |
| `zones.rs`, `zones/curated.rs` | exclusion data, spatial index and search exemptions | shared runtime hazard facts |

### nav-bake

This producer depends on the planner. Runtime planning never depends on this crate.
The combined future v18 schema is in the [baked reachability contract](baked-reachability.md).
BR1 retains v17 and makes one generator-identity change to `nav-bake-3`.

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `bake.rs` | release bake orchestration and zone derivation | shared by the CLI and application build |
| `bundle.rs` | staging, fingerprints, stamps and required inputs | runtime identity schemas remain in planner |
| `manifest.rs` | generated navigation manifest assembly | no runtime schema copy |
| `collision.rs` | collision construction and stamping | shared runtime models remain in planner |
| `pack/` and `pack.rs` | source parsers, bank derivation and encoding | pack and sidecar output bytes |
| `canlight.rs` | static canlight producer | reads shared runtime policy identity |
| `services.rs` | navpois and hunter inputs | jm2 placements, handlers and labels |
| `zones.rs` | curated group generation inputs | shared hazard facts remain in planner |
| `transport/` and `transport.rs` | transport derivation and parser support | all 26 source families, nested helpers and embedded procedures |
| `bin/nav-pack.rs` | custom-input bake CLI | explicit output paths |
| `bin/nav-input-audit.rs` | offline input audit | developer tool, not runtime routing |
| `bin/cache-content-id.rs` | offline identity export | reads the selected cache and snapshot |

`host-play` admits build and dev dependencies on the baker. Panel, frontend-core,
and TUI admit only producer-fixture dev edges. The latter two include host-play's
shared map and upgrade fixtures. The e2e tool admits its input-audit edge.
The host retains its planner build dependency for schemas, hashes and `FORMAT_ID`.
An environment skip flag does not remove nonoptional Cargo build dependencies.


The planner's and baker's `clippy.toml` files deny action types, functions, methods, and
macros by compiler identity. `tools/architecture/nav_clippy.py --write`
generates the lists and their inventory from rustdoc JSON.
`python3 -B tools/architecture/nav_clippy.py --check` checks the inventory
and runs the external-crate probes. The Clippy CI job runs this check.
The inventory reads `api` with default features and `client` with default and
`test-support` features. This includes feature-gated seed items such as
`client::io::client_stream::connect_ws_plain` in the generated deny list.

The inventory includes public constants and statics, which Clippy cannot
deny as values. The compiler check rejects new wrappers of denied types
that these lints cannot detect. Source hashes also detect changes to
inactive, feature-gated seed code.

The exact compiler-lint limit is a value-only read of an external constant
or static: `let _ = &api::S`, `let _ = api::C`, or
`let _ = api::S::C`. These expressions cannot act. Calls through the
resulting values remain subject to the method deny list.

The read-only cache loader has one narrow type-lint exception on its
`ClientRevision` import in `nav/src/map/catalogue.rs`. The revision enum
is public through a seed module, but this import only selects cache
data. The import-level `#[expect]` does not exempt any action method.
The real-pack integration tests use the same import-level exception to
select 289 fixture data. These test imports do not exempt action methods.
World-state unit tests use import- and function-level expects to construct
a cache-less client and populate snapshot families. These method expects
are test-only. The production planner has no method-lint exception.

Both scans read the standard Cargo roots: `build.rs`, `src/main.rs`,
`src/lib.rs`, each direct `src/bin/*.rs`, each direct `examples/*.rs`, and
each direct `tests/*.rs`. They also follow attached modules and `include!`
files. They report unsupported source forms and unattached Rust files under
`src`.

The `nav` scan rejects production paths to const, static, and associated-
constant values in the generated `api` and `client` seed inventory. It
ignores those value paths in test scope. It does not replace Clippy's
compiler-resolved action checks.

The `nav-executor` scan enforces separate production and test allowlists for
`nav` paths in executor source. It does not scan planner source in `nav`.

### nav-executor

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | `mod traveller;` plus the root re-exports, including `BlockedWhy`; the `traveller` module stays private |
| `traveller.rs` | follow facade and scheduler | `FollowRun` start and step, shared state, typed blocked reasons |
| `traveller/snapshot.rs` | snapshot-query helpers | scene reads for hops |
| `traveller/dialog.rs` | dialog and teleport-send helpers | chat and teleport sends |
| `traveller/walk.rs` | walk-hop execution and reporting | walk targeting |
| `traveller/transport_hop.rs` | transport-hop execution | approach and door execution |
| `traveller/npc_hop.rs` | NPC-backed Boat/Npc/Glider approach and recovery | live scene operability, reachable-cost selection, bounded retry |

#### Shared bounded routing

The loaded navigation lease supplies separate carry, worn and diagnosis bounds.
`max_plan_item_rows` is the smaller of the full diagnosis bound and the sum of
the carry and worn bounds. Bank planners use this effective item bound, not
the full search budget. Captured bank rows remain a separate dimension.
Cursor construction does not allocate. Cursor growth uses checked limits and
fallible reservations.

Route execution separates completed searches into private jobs in the existing
Route family. A later job charges retained routes, diagnosis and plan storage
at their exact sizes as input. Each whole job remains within 192 MiB.
The shared owner-result ledger holds continuation custody between jobs.
Cancellation and request-generation checks apply across each gap.
Public Search/Commit phases and Path/NoPath results do not change.

#### Cooperative search storage

All search goals use one packed table and one bounded doubling and rehash
method. Records have dense IDs. Hashing selects a three-byte bucket head,
not a position across the record array. There is one bucket per twelve records.
The multiply-high reduction does not need a power-of-two bucket count.

Cooperative cursors reserve their complete settle-cap backing once. Slot
fields use a zero-valued tuple of primitive arrays and integers. Each record
remains 28 bytes with four-byte alignment. Bucket heads and next links share
a separate array of three-byte links. Each link uses only the ID bits its
table capacity needs; the remaining bits hold a hash fingerprint. A fingerprint
mismatch skips the full record read. Matching fingerprints still require full
key equality. Growth rebuilds the collision chains for the new bucket count.
It keeps the same dense IDs, but it can change the order of records in a chain.

Each table also keeps 1,024 recent positive record IDs in a fixed 4 KiB array.
The unused upper ID bits hold a ten-bit hash tag, so unrelated cache entries
do not require a full record read. Matching tags still require the full tile
and balance. Insertions and positive lookups refresh this array. Removal
invalidates its entry, and growth preserves IDs. This avoids repeated collision
scans without another allocation. The array is part of the admitted future's
measured inline size, not the vector backing bound.

Safe `Vec` construction obtains zeroed backing without an explicit
initialization pass under the pinned toolchain. The operating system can make
pages resident as the search touches them. Admission charges the complete
backing, not the observed resident bytes. Fingerprints use existing link bits
and do not increase that charge.

Legacy synchronous facades start with six settles and grow the same table
only when needed. Their reverse proofs also retain demand-driven storage.
Cooperative reverse proofs reserve their bounded capacities once, so an
ordinary slice never grows or rehashes a complete proof.

Cooperative searches reserve their complete settle-capacity heap once.
Synchronous searches start with the record table's initial capacity and grow
the heap when that table grows. Each required growth allocates a fresh buffer,
copies only live entries, and drops the old buffer. This avoids reallocating
the heap in place and copying unused capacity. Admission still charges the
complete heap bound, not the committed pages.

The frontier starts as a binary min-heap. Only bounded-frontier eviction
converts it to a min-max heap, in place and with one checkpoint per parent.
Ordinary heap operations share the caller's expansion or transport unit.
A sift has at most 22 levels under the packed capacity limit.
The comparison order, decrease-key behavior and worst-entry eviction stay unchanged.

One expansion includes its eight fixed collision steps and the first hash-key
comparison for each relaxation. Further hash-chain links and each transport-list
entry remain separate work units. Search budgets and live clock and cancellation
reads at each checkpoint remain unchanged.

Bank-pick qualification uses `bank_pick_work_live`, an integration target behind
`live-harness` and `test-support`. It calls the same native burst and captured
job replay as the library tests. The integration binary does not reference
`allocation-counter`, so its zeroed storage uses the product allocator.

#### Native map data boundary

The public `nav_planner::map` modules define the data/spatial contract independently of
the panel, TUI and worker lifecycle. `SourceSpace::ClientVisual` alone uses
`collision::game_plane`; collision and can-light stamping use that same helper.
Server NPC spawns and already-game-plane coordinates are never shifted.
The conservative classifier preserves one-based operation slots and distinguishes
bank candidates/map symbols from service evidence. It does not change the frozen
catalog-facing bank roster or execute an operation.

Image/catalogue schema 1 identities use domain-separated SHA-256 binary preimages
(documented in `map/identity.rs`), with decoded client identity rather than
transfer CRCs. Image policy includes exact encoder/compression-library versions.
Nav/service changes affect the merged catalogue key, not terrain identity.
`274bot.navpois` uses a 77-byte `274P`/1 header (length, record count, identity
binding, payload hash) followed by bounded typed JSON. Nav bake generates the
sidecar from jm2 NPC rows, rs2 bank-service handlers and `maps/labels.txt`;
its whole-file digest is stamped on `NavManifest` / `BakeStamp` / identity rows.
Packaging copies the data-only file with nav resources. Frozen bank APIs stay.

Disk consumers use `ClientPois::decode`, `ImageManifest::decode`,
`ServicePois::decode_navpois`, `ReadyCatalogue::open` and `ReadyImages::open`,
not unchecked serde construction. JSON is capped at 1 MiB; lists/text are bounded,
keys sorted and unique, and filenames derived from typed tile keys. Image headers
are fixed to 256-pixel interiors with one-pixel gutters, RGBA8 noninterlaced PNG.
`TileReceipt::verify_png` verifies the receipt and IHDR before pixel allocation;
the PNG decoder still owns full chunk/CRC validation.

`PartialEntry` provides checked synchronous resume. The admitted host cache job
decodes checkpoint metadata, then verifies one completed payload per host unit
before exposing its receipt to the producer cursor. Ready readers require a
canonical complete directory and validated manifest. Ordinary image open does
not hash or decode the pyramid. The cache-job owner (`host-play`) owns the
prepared-cache `Arc`, locks, atomic publication, cancellation and quotas.

Release packages ship the 289 catalogue and terrain: release packaging bakes the nav
bundle's pinned client cache through the production map-cache path (the image
demand derives its catalogue first, so one bake yields both) and ships
the published directories as `map/<revision>/catalogues/<key>/` and
`map/<revision>/images/<key>/` under the install resource root (beside
`nav/`), described by `map/<revision>/274bot.mapcatalogue.json`
(`ShippedCatalogue`: catalogue identity, key, the catalogue manifest's
receipt, record and payload totals) and
`map/<revision>/274bot.mapimages.json` (`ShippedImages`: image identity, key,
the image manifest's receipt, tile totals). The cache job installs each
artifact only for exactly the bound client cache's identity for that
artifact: every tile's length and SHA-256 is checked against the pinned
image manifest as it is copied into the key's `.partial` directory, and the
catalogue payload is checked against its pinned manifest's receipt and
record count; both are recorded by receipt and published by the same writer
and rename as a local derive or bake, so readers never see a second loader.
A published key is never replaced, and shipped files are only read. Foreign
shipped data is ignored; damaged or partial shipped files fail closed (the
catalogue derive runs locally at once; the local terrain bake stays behind
the operator's consent and adopts the tiles already verified).

### script

| module | owns (one reason to change) | notes |
| --- | --- | --- |
 | `lib.rs` | crate facade: `ScriptCtx`, `SlotScript`, registry descriptors | no `load` feature and no JS dependencies |
 | `ctx.rs` | host-owned per-tick input and shared walk options | compiled cards receive `native::NativeTick`, not this driver |
 | `native.rs`, `registry.rs` | typed card/configuration/output contract and one descriptor registry | metadata, native request kind, preparer and factory. Hosts install the JS cards with `script_js::install_cards()`. Slots refresh their static card view when the publication generation changes |
| `native/actions.rs`, `native/ledger.rs`, `native/owner.rs`, `native/walk.rs`, `native/walk_wait.rs` | host action ledger and native walk correlation | receipts, revocation identities, route terminal versus arrival |
| `native/death.rs`, `native/thieve.rs`, `native/thieving_core.rs` | shared death latch; native thieving machine and its core | content-derived respawn square |
| `native_bank.rs`, `native_equipment.rs`, `native_production.rs`, `native_shop.rs`, `trade_screen.rs`, `bank.rs`, `bank/ops.rs`, `bank/npc.rs` | snapshot-driven cores shared by compiled cards and compat families | one bank transfer kernel, wear/remove, production, shop purchase and player-trade screen |
| `slot.rs` | per-uid runner lifecycle | intent and presence gates, tick edge |
| `surface.rs`, `surface/test_support.rs` | runtime-neutral surface cards, run contract and core test factory | slot lifecycle owns trait objects and typed facts, not JS or wire state |
| `native/recovery_hints.rs` | shared recovery-anchor cell | slot-owned hints survive surface retirement and restart |
 | `slot/compiled.rs` | off-pump compiled preparation, fenced install/configure and shared output | lazy per-active-card state; compiled XOR JS surface run |
| `slot/api.rs`, `slot/progress.rs` | API-owned Gatherer seat; one-attempt progress policy | registration and authority stay native |
| `slot/combat.rs` | API-owned combat seat; one `api.combat.fight` session per slot | mirrors the gather seat; Pause, reconnect and manual movement cancel at any point |
| `slot/bank.rs`, `bank/seat_open.rs`, `bank/seat_transfer.rs` | native bank seat, opening and transfer continuations | shared `bank/ops.rs` and `bank/npc.rs` kernels; hooks use seat prompts, not a second isolate behavior |
| `slot/pending.rs` | bank settlement records | host-owned pending accessors |
| `machine.rs` | step-machine host | one multi-tick behavior in Rust |
| `acquire/` | shared acquisition planning, typed block evidence and one bank-trip runner | `plan.rs` pure per-need decisions; `run.rs` native bank, walk and wear execution; authored Quester sequences retain their existing order and refusal text |
| `quester/` | Quester card, Path types and compiler, runner, queue, pair and gang, eligibility, provisioning, watchdog, quest handlers and step families | `card.rs` card; `path.rs`, `compile.rs`, `schema.rs` Path values, lazy compile and JSON Schema; `runner.rs` colour-first tick loop with journal evidence; `registry.rs` bundled documents plus optional folder; `families/` one step family each (combat, dialogue, gather, partner, reach, thieve, ...) |
| `gatherer/` | Gatherer card and runner | woodcutting, mining and fishing from live observation: `select.rs` target choice, `gather.rs`, `drop.rs` power drops, `supply.rs`, `recover.rs` death recovery, `area.rs`, `widen.rs`, `settings.rs`, `status.rs`, `oneop.rs` |
| `combat/` | shared observed-state combat machine | melee, ranged and magic on one planner (`machine.rs`, `style/`); `policy.rs`, `prayer.rs`, `guard.rs` WalkGuard, `threats.rs`, `schedule.rs`, `select.rs`, `tables.rs`, `request.rs` |
| `combat/risk.rs`, `combat/risk/{facts,geometry,input,replay}.rs` | pure conservative route assessment | shared combat facts and food picker; complete route geometry; persistent poison input; no admission, route-worker or runtime escape wiring |
| `api_gather.rs`, `api_progress.rs`, `api_session.rs` | JS API v2 session wire and output types | Gatherer sessions and quest progress for Load scripts |
| `api_combat.rs`, `combat_session.rs` | JS API v2 combat session wire and its native card | typed request decode and resolve; drives the shared combat machine once per session |
 | `observed.rs` | per-isolate decoded scene (JS runtime) | lives in `script-js` as `src/scene/` materialization; delta-merge, owned rows |
 | `wire/isolate_fb.rs` | isolate wire codec (JS runtime) | lives in `script-js` as `src/wire/isolate_fb.rs`; flatc-generated bindings (`crates/script-js/schema/generated`) plus delta/`IsolateBuf` domain layer; one verified root per message |
 | `tooling/host_js.rs` | generated Host JS types (JS runtime) | lives in `script-js`; from verb tables, not rs2b0t names |
 | `tooling/compat_dts.rs` | generated compat declarations (JS runtime) | lives in `script-js`; JS API v1 `.d.ts` from the frozen typed source |
 | `channel.rs` | bounded structured-clone subset for the BroadcastChannel broker | bytes over the FlatBuffer wire |
| `content.rs` | curated sites and hostile predicate | bank aliases posted via the shim payload |
| `ent.rs` | Ent lookup over caller rows | no scene scan |
| `bank/request.rs`, `bank/compat.rs`, `bank_select.rs`, `bank_withdraw.rs` | typed bank arguments, local calculations, catalog materialization and thieving bank sequencing | bank actions use the native seat; no isolate-owned bank transfer, access or selection loop |
| `bank_hints.rs` | persisted last-seen bank hint | one schema-1 file per account; planning before any bank opens in a new process |
 | `clue.rs`, `compat/clue/isolate.rs` | native recovery declaration; JS-run clue session machine (JS runtime) | lives in `script-js`; isolate token, clock, identify, verbs stay private to that crate. Sherlock is available only in builds with the JS runtime. Sherlock was meant to be native. In practice it became a port of the compatibility clue machine, so it needs the JS runtime build until the native clue rewrite replaces that machine. Sherlock itself never creates a V8 isolate |
 | `compat/clue/scene.rs`, `compat/clue/puzzle.rs`, `compat/clue/entrana.rs`, `compat/clue/acquire.rs`, `compat/clue/shop.rs`, `compat/clue/talk.rs`, `compat/clue/combat.rs`, `compat/clue/verbs.rs`, `compat/clue/family.rs` | clue phase helpers (JS runtime) | live in `script-js`; one phase slice each |
 | `compat/sherlock.rs` | Sherlock card wiring (JS runtime) | lives in `script-js`; marshals the frame, maps verbs, no second solver. Sherlock is available only in builds with the JS runtime (see above) |
 | `compat/duel.rs` | clue Duel Arena protocol (JS runtime) | lives in `script-js`; partner matching, rules, timing, travel legs |
 | `compat/hunt.rs` | hunt framework on the machine host (JS runtime) | lives in `script-js`; sessions and runs, child nesting |
 | `compat/hunt/wait.rs`, `compat/hunt/teleport_out.rs`, `compat/hunt/acquire.rs` | hunt family slices (JS runtime) | live in `script-js`; three distinct families |
 | `compat/hunt_bank.rs`, `compat/hunt_bank/plan.rs` | hunt bank runtime and planning (JS runtime) | live in `script-js`; stages in root, plan projection in child |
 | `compat/hunt_fight.rs` | fight aggregate and shared projection (JS runtime) | lives in `script-js`; policy, clocks, field pick |
 | `compat/hunt_fight/hold.rs`, `compat/hunt_fight/retreat.rs`, `compat/hunt_fight/walk_spot.rs` | fight auxiliary machines (JS runtime) | live in `script-js`; one machine plus its Kind each |
 | `compat/hunt_cell.rs`, `compat/hunt_key.rs`, `compat/hunt_lair.rs`, `compat/hunt_leave.rs` | single-phase hunt machines (JS runtime) | live in `script-js`; one phase chain each |
 | `compat/hunt_catalog.rs` | hunt tables and small rules (JS runtime) | lives in `script-js`; coordinates and pure helpers, no sends |
 | `compat/cake_stall/runtime.rs`, `compat/cook_locations.rs` | stall selection and steal loop; cook-location resolution (JS runtime) | live in `script-js`; facts in `api`, sequence there |
 | `compat/production.rs` | chat-make sequencing (JS runtime) | lives in `script-js`; Make-X and panel family |
 | `compat/fire.rs` | fire lighting and lane ranking (JS runtime) | lives in `script-js`; frozen light loop |
 | `compat/food_policy.rs`, `compat/supply_v2.rs`, `compat/boost_potions.rs`, `compat/prayer.rs`, `compat/autocast.rs`, `compat/ranged.rs`, `compat/special.rs`, `compat/melee_weapons.rs`, `compat/escape_runes.rs`, `compat/attack_clock.rs`, `compat/fight_upkeep.rs` | combat and supply policy (JS runtime) | live in `script-js`; descriptors, predicates, arm sequencing |
 | `compat/gather_tools.rs` | gather-tool selection (JS runtime) | lives in `script-js`; policy over the posted rows |
 | `compat/dialog.rs`, `compat/dialogue_outcome.rs`, `compat/teleport.rs`, `compat/trade.rs`, `compat/partner_trade.rs`, `compat/drive_partner_trade.rs` | dialog, teleport, and trade sequences (JS runtime) | live in `script-js`; machine families |
 | `compat/death_recovery.rs` | death latch and recovery run (JS runtime) | lives in `script-js`; chat latch plus walk-back |
 | `compat/periodic_bank.rs` | periodic bank run (JS runtime) | lives in `script-js`; validate and execute |
 | `compat/walk.rs`, `compat/walk_wait.rs`, `scene/reach.rs`, `scene/reach_entity.rs`, `scene/scene_query.rs`, `scene/inspect_wait.rs`, `scene/task_clock.rs`, `scene/watchdog.rs`, `compat/anchor_return.rs`, `compat/boat_fare.rs`, `scene/event_signal.rs` | walk, wait, and reach helpers (JS runtime) | live in `script-js`; tick waits, arrival, boat-fare recovery, return-to-anchor |
 | `scene/line_of_sight.rs` | line-of-sight over the posted table (JS runtime) | lives in `script-js`; isolate-scene reads |
 | `compat/keep_list.rs` | combat keep-list composition (JS runtime) | lives in `script-js`; compatibility list |
 | `compat/modals.rs` | modal helpers (JS runtime) | live in `script-js`; dialog page helpers |
 | `quest_journal.rs`, `quest_journal/native.rs`, `compat/quest_journal/isolate.rs` | native journal contract and native journal click machine; JS-run journal machine (JS runtime part) | the isolate half lives in `script-js`; one click, validate, capture, one close |
 | `shop.rs`, `market_catalog.rs` | shop and market descriptors | preset and catalog tables |
 | `loadout_plan.rs`, `loadouts_store.rs`, `settings_store.rs` | loadout and settings stores | operator-facing descriptors |
 | `library/rs2b0t_registry.rs`, `library/rs2b0t_registry/paths.rs`, `rs2b0t_registry/settings.rs`, `tooling/declared_abi.rs`, `module_imports.rs`, `identity.rs` | catalog card registration and identity (JS runtime library) | live in `script-js`; import scan, paths, ABI names |
 | `runtime/isolated_env.rs`, `library/js_cache.rs`, `runtime/memory_profile.rs`, `scene/events.rs` | isolate env, cache, and diagnostics (JS runtime) | live in `script-js`; support, no game actions. The profiling module keeps its root public path |
 | `runtime/` | Load facade, shapes, cards and isolate (JS runtime) | lives in `script-js` as `src/runtime/` plus `src/library/`, `src/materialize/` and `src/api_v2/`; feature-gated library and isolate |
 | `runtime/platform.rs` | process-wide V8 platform bootstrap (JS runtime) | lives in `script-js`; protected on embedder main; unprotected only in tests |
 | `runtime/bindings.rs` | isolate bootstrap registration (JS runtime) | lives in `script-js`; register order and wrappers |
 | `materialize/callback_v8.rs` | caller-callback invocation (JS runtime) | lives in `script-js`; one mechanism for typed helpers |
 | `materialize/*_v8.rs` | typed local V8 marshalling (JS runtime) | live in `script-js`; one native call each, no machines |
 | `materialize/area.rs`, `materialize/buyout_plan.rs`, `materialize/distance.rs`, `materialize/reach_query.rs`, `materialize/shape.rs`, `materialize/line_of_sight.rs`, `materialize/wait_clock.rs` | small typed V8 helpers (JS runtime) | live in `script-js`; Area rect/circle compat constructor, planning, geometry marshalling, paused-time wait clock |
 | `runtime/canvas_tape.rs` | canvas op tape (JS runtime) | lives in `script-js`; typed flush onto the recorder |
 | `runtime/snapshot.rs` | snapshot-to-V8 materializer (JS runtime) | lives in `script-js`; typed settings and event rows |
 | `runtime/paint_chrome.rs`, `runtime/paint_jive.rs` | paint payload builders (JS runtime) | live in `script-js`; chrome and jive paints |
 | `canvas/mod.rs` | canvas recorder | state and recording API |
 | `canvas/geom.rs`, `canvas/raster.rs`, `canvas/style.rs`, `canvas/compose.rs` | canvas geometry, raster, style, composition | geometry and parsers are always available. Raster, fonts and composition require `canvas-raster`, which `frontend-core` enables by default |
 | `shim/mod.rs` | import-remap facade | re-exports, JS bytes untouched |
 | `interact.rs`, `paint.rs` | runtime-neutral interaction and paint vocabulary | shared by native and loaded scripts |
 | `shim/content.rs`, `shim/rows.rs`, `shim/modules.rs` | shim Rust producers (JS runtime) | live in `script-js`; content, V8 rows, and module remapping |
 | `shim/*.js` | embedded JS sources (JS runtime) | live in `script-js` as `src/shim/*.js`; name maps, coercion, await plumbing only |

Native bank booth approach borrows the current scene and the host's cached
exact reach plane. The snapshot query shares the existing footprint,
force-approach, directional-wall and stand-ranking rules. It does not copy
the scene, build a second flood, or allocate a temporary stand vector.

 The Core crate root holds the data the native modules embed: `paths/289/` (the built-in
 Quester Path documents) with `paths/path.schema.json`, `fonts/` and `examples/`.
 The JS runtime crate root holds its own data: `crates/script-js/schema/isolate.fbs`
 with its checked-in `crates/script-js/schema/generated` bindings, `crates/script-js/compat-js/`
 (the JS API v1 name maps and generator) and `crates/script-js/host-js/index.d.ts`
 (the generated JS API v2 declarations).

 V8 embedders call `script_js::bootstrap_v8_platform()` on process main before
 creating any slot, worker or control thread that might create a JS runtime,
 including validation and restarts. The protected platform's Linux
 memory-protection-key permissions are inherited only by descendants created
 after initialization; a once-only call on the first bot cannot authorize
 pre-existing sibling slots. Missing bootstrap returns
 `LoadIsolateError::PlatformNotBootstrapped` before spawning an isolate.
 Stable libtest workers are pre-created siblings. `script-js` unit tests and debug
 test consumers with the dev-dependency `script-js/test-hooks` feature use V8's
 documented unprotected test platform instead. The function and fallback are
 gated by `cfg(test)` or `all(feature = "test-hooks", debug_assertions)`:
release panel/host-play/tui dependencies cannot compile that path, even if the
feature is enabled. Their main entrypoints always bootstrap the protected
platform. The harness-free `v8_platform` subprocess test also initializes the
protected platform on main, so it tests production ancestry rather than the
libtest exception.

### host-play

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | `Play` state and crate facade | declares the lifecycle modules below |
| `main.rs` | `host-play` CLI | profile bind, vault unlock, run |
| `play_bootstrap.rs` | live account and passphrase minting | `live<token>_<i>` names, ephemeral vault, loopback check |
| `play_login.rs`, `play_slots.rs`, `play_status.rs`, `play_wires.rs` | session lifecycle | login FIFO, slot spawn/stop/reap and tick pump, the status rows, cheat and wire queues |
| `play_scripts.rs` | per-slot script transactions | start (compiled and Load), pause, stop, loadout validation |
| `profile.rs`, `profile_binding.rs`, `profile_options.rs`, `profile_selection.rs` | launch-input resolution | world, vault and script paths; profile class and flags; the selected-server binding and its verified inputs; the generated-facts trust decision with its recorded reason |
| `servers.rs`, `public_worlds.rs` | server profiles and public endpoints | `~/.274bot/servers.json`; operator-owned public roster loaded once |
| `passphrase.rs`, `instance_lock.rs` | vault passphrase prompt; process lock | never from env or argv; `~/.274bot/instance.lock` |
| `cache.rs` | runtime cache preparation order | progress stages, client owns mechanics |
| `catalog_core.rs`, `catalog_core_hunt.rs`, `catalog_core_ranging.rs`, `catalog_*.rs` | shared full-core proof witness and its per-family cases | headed and headless proof paths; hunt and ranging-guild witnesses; one file per case family |
| `paired_core.rs`, `pair_*.rs` | paired full-cycle proof witness and its cases | Air, Mule, Flax, Duel |
| `combat_proof.rs`, `debug_replies.rs` | combat capture for live proofs; bounded observation of debug-command replies | temporal reply candidates, never acknowledgements |
| `live_gate.rs` | live proof gate for every scenario watch | which shared witness a run qualifies under (paired proofs refuse without the pair gate), per-poll core-gate verdicts, the PASS hold: Pending core first, then the 45 s clean-stop grace; panel and `tui-play` both call it |
| `live_start.rs` | live catalog Start transaction | stashed isolate Starts fired on StartScript, witness armed immediately before each actual Start, setup failures fail the armed witness; the fleet launch panel and `tui-play` both stash (`fleet_catalog_starts`, feature `live-harness`) |
| `script_runtime.rs` | per-slot observe, dispatch, and nav continuation | script wall and walk continuation |
| `script_observe.rs`, `script_snapshot.rs`, `script_interact.rs`, `script_nav.rs`, `script_walk.rs`, `script_bank.rs`, `script_paint.rs`, `script_channels.rs` | per-slot script pump pieces | observation and snapshot posting, interact dispatch, cooperative route-inspection publication and walk continuation, WalkGuard, bank selection as a bounded cursor job on shared WorkAdmission using the shared compact search and one immutable `Arc<WorldState>` backing. Its 192 MiB whole-job charge covers captured input, cursor and result. At five seconds, walking requests use the route-only Single fallback after admission. Non-walking requests return the first candidate without a route, including demand still refused with Full. Full admission and owner-ledger pressure retain retry demand. Other admission refusals and oversized requests publish NoCandidate or walking NoPath. The remaining pieces own paint status and the BroadcastChannel broker. |
| `walk_arm.rs`, `walk_plan.rs`, `walk_permissions.rs` | operator walk arms and permissions | queued follow and bank-fetch steps, session routes, permission composition at admissions. Selected-bank walks use the same compact route adapter for Traveller and borrowed map paint, without expanding legacy route vectors. |
| `admission.rs` | shared route-admission seam | advisory survivability estimate under Proceed; no net or poison-clear observer claimed |
| `slot_bank_memory.rs` | slot thread's hand on its account bank memory | one shared handle per account; the slot thread is the only writer, never across file IO |
| `slot_condition.rs` | slot thread's condition fact | one owner in `NavBot`, independent of scripts; publishes a `Copy` fact only after a complete live frame |
| `evidence_writer.rs` | background live-evidence PNG and JSON writer | one thread per cell; the slot thread only hands over pixels and receipts |
| `route_inspect.rs` | inspect cooperative-work adapter and owner | Captures one shared `WorldState` and `NavLease`. It runs strict search, borrowed `MissingRequirementsView`, bank-fetch planning, stand and post-bank routing, and zone diagnosis within the 192 MiB job charge. It transfers terminal custody to the shared 256 MiB owner ledger before WorkService consumes the result. It keeps one admitted job, one replaceable pending demand, two replay terminals and one held terminal. It uses acknowledgement, eviction and identity/revision-bound cancellation. |
| `login_readiness.rs` | login-readiness gate | welcome-modal settle before script work |
| `nav_identity.rs` | bundled nav identities | build-published table plus checked-in rows |
| `walk_map.rs`, `walk_map/{actions,catalogue,observations,routes}.rs` | shared WalkTo map model | catalogue, selection, guarded Walk/Teleport, live/script/manual routes |
| `map_cache.rs`, `map_cache/shipped.rs`, `map_cache/fixture.rs` (`test` / `test-support` fixtures), `map_producer.rs` | WalkTo map cache lifecycle | one `WorkService` MapBake family with resumable bounded producer cursors; demand, bake/publication, installing release-shipped terrain that matches the bound image identity; no private bake worker |
| `map_bind.rs` | process-wide map demand | manager at first open (shipped terrain under the install root's `map/`), packaging bake, bounded shared-worker terrain PNG decode (258×258 RGBA8; 512 KiB input plus inline job, Arc refcounts and refcount-alignment padding; `png::Limits` allowance of one additional raw RGBA row, 2 MiB scratch covering the overlapping ≤512 KiB eXIf buffer and metadata copy, ≤128 KiB raster buffer, row scratch and bounded inflater tables; 266,256-byte result; text/ICC metadata is skipped) |
| `bundled-nav-identities.json`, `known-cache-identities.json` | checked-in identity rows | data, not code |
| `external_loader.rs` | external loader smoke witness | proof infrastructure, disabled by default |
| `memory.rs` | opt-in memory harness | `BOT_MEMORY_N` only |
| `memory_diagnostics.rs` | memory-run diagnostics | bounded, never drains logs |
| `memory_startup.rs` | memory-run startup timeline | process-relative milestones, render-callback gaps and one adapter record, atomics only |
| `memory_slots.rs` | memory-run per-slot outcomes | ready latch and the failure/boundary record |
| `audio.rs` | focused-slot speaker gate | at most one speaker |
| `rss.rs` | process RSS and CPU sample | harness and the core's resource meter |
| `resource_view.rs` | live-slot facts and panel-ui prefs | live worker count/traffic the core meter samples, background count, `panel-ui.json` |
| `scatter.rs` | seed tiles for the wall | nav world, else Lumbridge |
| `progress.rs` | preparation progress values | small, copy-free |
| `quest_pair.rs` | Play-owned pair leases | bounded; never takes a slot lock or retains a world |
| `*_live.rs`, `*_live_probe.rs`, `*_live_tests.rs` | live cells run through `cargo test` | ignored; see [the harness](harness.md) |

The slot condition consumes game messages with no sender and raw hitmarks only
at PLAYER_INFO edges. Player chat cannot supply poison or cure evidence.
The applied NPC_INFO counter can complete a proven onset source inside that
edge's observation window. An unlatched onset has no attributed pulse.
Ambiguous lines use the strongest selected poison pulse immediately.
A later smaller poison mark lowers the reserve unless proven immunity covers it.
`CombatTables` caches the selected revision's unknown poison pulse once.
Route replay reads that shared bound without changing the observed `RiskInput`.
Every antipoison drink line credits only the ordinary 120-tick lower bound,
including super antipoison. Queued poison damage cannot revoke that credit
through its inclusive end tick. Later poison marks replace it.
Inventory loss without a drink line grants no credit.
Admission and native actions read the current stamped fact, not private poison
memory. Reconnect starts at the reset session tick and current client chat cursor.
It hides the fact until a complete live frame with selected combat tables exists.
Missing tables leave food and cures unknown, not known empty.
Unknown poison stays unknown through 33 consecutive clean observed ticks after
the last host-dispatched interaction. It clears on the next clean tick, with
an accepted input inside the window. Each outgoing operation, use or interface
action restarts the count while poison is unknown. The slot observes actual
packets through the existing outgoing decoder before the next socket flush.
Movement packets prove accepted input but do not restart the count.
The bound includes the 30-tick timer, one queued-damage tick and the existing
two-tick observation margin. Modal and guardian holds reset the window.
Continuous interactions therefore retain unknown poison. A pause or movement-only
run can finish the window. Native manual input inside the client loop is outside
this host-dispatch observation until the T1.4 client-adapter transport cutover.

`WalkGuard::tick` takes the same-frame condition and the current cure policy.
Its one decision body checks a held cure before every food branch, including
food-only walks. Combat applies the same policy before all eating branches.
Both consumers add one next poison fire to their eating lines and reject
a bite at full hitpoints. Strange fruit shares the content's food clock:
after a bite at T, neither consumer sends fruit before T+3. Its receipt
does not overwrite a pending health-food receipt.
The Gatherer boundary-eating path is separate until the runner survival step lands.

The vault stores the Survival and supplies profile group. `SetSurvival` saves
that group through the profile writer. Only a durable write updates the slot's
atomic mirror and writes its audit line. Each observed frame reads that mirror
once, then posts the policy to native action contexts and the host walk guard.
The script policy type remains `script::survival::CurePoison`.

The live mirror proof reads the real host policy after that per-frame copy.
Its read-only post-host-frame observer exists only with `test-support`; product
builds have neither its storage nor its callback. A pre-copy read cannot prove
the one-frame deadline and is rejected by the proof oracle.
The arm and host mirrors must occur at or after the durable save, within one frame.
The native policy must appear at the next observed game tick after the host
mirror. Its frame must fall within the sampled frames for that tick.
The guard's accepted antipoison must follow the host mirror's snapshot tick.

The survival eating oracles compare a captured action's explicit `snapshot_tick`
with the slot condition's tick through one checked time-window helper. The host
script counter is a different clock; it is neither a fallback nor an offset.
All three eating cells use that helper for bite and heal deadlines. An action
without a snapshot clock fails the oracle. Saved walk receipts can be read back
and judged by the same oracle without changing their original evidence.
Each live receipt records its git commit. Combat proof matches the native
fight action id as well as the run, since one run can contain several fights.

Map-cache census retains at most 1,030 root summaries. Completion transfers that
allocation without a replacement. A new scan releases previous census storage
first. Mutation generations fence census, publication and bounded pruning. Entry
caps are 3,490 per artifact root and 3,594,700 across the cache.
Disk cache hits pass the same admitted preflight before the manager publishes a
ready lease. Joined demand can add bake consent but cannot revoke it. Host
progress uses the frozen map-bake phase indices in both directions.
The generation counter and empty key-lock files coordinate live processes.
They are not crash-durable artifact metadata and do not need file syncs.
New artifact directories still synchronize their changed parent entries.
A refreshed census resumes the exact producer/publication stage and rebases a
live writer's whole-cache totals. If pruning removed its unpublished partial,
the job discards the stale receipts before rebuilding. Clean cursor yields do
not rewrite an unchanged checkpoint.
A terrain probe that finds no published or matching shipped terrain creates
no image directory or key-lock file. Later bake consent restarts path
preparation and the census before mutation. Catalogue preparation writes no
empty checkpoint. Normal completion writes its single-payload manifest directly.
The manifest's parent sync also commits the already-synchronized payload rename,
without first writing a publishing checkpoint that the manifest supersedes.
Explicit checkpoints, observed cancellation and incomplete work keep their
checkpoint flush. The payload and checkpoint file data reach disk first.
One parent sync then commits both renames before the checkpoint is durable.
Catalogue placement and marker-candidate work groups at most 64 operations
per admission unit. Each operation checks the live deadline and cancellation.
Archive reads, land decoding and finalization remain individual units.
The shared unit allowance and the 10 ms deadline do not change.
Catalogue derivation retains at most 32,768 tree or quest-location candidates
in source order, with their original placement and bridge flag. The first
scan also records each square's candidate end offset. The charge includes
both fixed storage bounds, without vector growth overlap. The 274 and 289
content census found 14,844 and 15,059 candidates. The limit rounds twice
the larger count up to a power of two. Overflow releases that prefix and
uses the existing second scan. It neither refuses input nor changes output.
Each 64-tile terrain checkpoint batch synchronizes each changed tile directory
and each newly created directory's parent once, from children to parents.
At most 63 completed tiles await a batch checkpoint. Observed cancellation
and completion flush the remainder.
Individual PNG data reaches disk first. The checkpoint file is written and
synchronized next. Its parent sync also commits the new terrain directory.
The writer retains only fixed directory bitsets between worker invocations.
Resume keeps valid partial tiles and regenerates missing or damaged payloads.
Checkpoint rows remain sorted when regenerated tiles fill earlier holes.
A payload that contradicts its durable checkpoint receipt is removed and its
parent directory synced before the native cursor can adopt uncheckpointed tiles.
Verification reuses one format-bounded payload buffer. Its reservation includes
the decoded-row capacity and the verified rows while both allocations overlap.
After admitted validation, publication reuses an immutable ready owner still
retained by a consumer.

### scenario

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | shared layer facade | seed, steps, proof; two runners |
| `runner.rs` | shared step state machine | tick bookkeeping, no sleeps |
| `proof.rs` | proof predicates | named assertions over a snapshot |
| `evidence.rs` | JSON evidence record | terminal snapshot plus predicate |
| `fixture.rs` | fixture prepare and run modes | offline sav versus live seed |
| `shot.rs` | window shot files | shared naming for both runners |
| `catalog.rs` | scenario catalog | names and lookup |
| `quester.rs` | Path-backed Quester qualification fixtures | `QuesterStage`, `FIXTURE_PROFILES`, miniquest and role stages; never product gameplay |
| `quest_fast.rs` | shared world tick-speed request and fresh confirmation | one `::speed` command with its chat sequence, sent only for an explicitly requested change; Engine Q only |
| `tutorial.rs` | post-relog tutorial reseed for fresh live accounts | fresh same-session `tutorial 1000` confirm before the cell continues |
| `render_betty_views.rs` | fixed-orbit render captures | station times yaw and pitch |
| `scenarios/mod.rs` | scenario facade | re-exports families |
| `scenarios/production.rs` | production facade and shared vocabulary | seed and watch helpers |
| `scenarios/production/*.rs` | production script families | one file per script family (agility, cooking, fletching, runecrafting, smelting, thieving, ...) |
| `scenarios/combat.rs` | combat facade and shared kit | witness builders, quest and equipment kit |
| `scenarios/combat/*.rs` | combat target families | field and bank variants each |
| `scenarios/shop.rs`, `scenarios/shop/buyout.rs` | shop scenarios and buyout | teleport factories in root |
| `scenarios/pair.rs`, `scenarios/pair/travel.rs` | trade and companion scenarios | preparation and travel kit in child |
| `scenarios/navigation.rs` | nav-edge probes | probe helpers and closer-slot machinery |
| `scenarios/quester.rs`, `scenarios/gatherer.rs`, `scenarios/boat_fare.rs` | Quester, Gatherer and boat-fare scenarios | cheats allowed only in the fixture |
| `scenarios/acquire_key.rs`, `scenarios/bank.rs`, `scenarios/cell.rs`, `scenarios/clue.rs`, `scenarios/enter_lair.rs`, `scenarios/fight_field.rs`, `scenarios/hold_spot.rs`, `scenarios/leave_lair.rs`, `scenarios/line_of_sight.rs`, `scenarios/prayer.rs`, `scenarios/ranging_guild.rs`, `scenarios/render.rs`, `scenarios/retreat_spot.rs`, `scenarios/route_inspect.rs`, `scenarios/script_basics.rs`, `scenarios/walk_spot.rs`, `scenarios/actor_observation.rs` | single-scenario probes | one probe each |

### frontend-core

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade | re-exports the session vocabulary |
| `session.rs`, `session/native_settings.rs` | operator lifecycle over `Play` | vault (staged view), play, selection, slot IO, Load/Log in/Log out/Remove, poll (also refreshes the projections and runs the meter), script Start/Stop settlement, profile write settlement; native settings preparation as a persistence prerequisite |
| `fleet.rs` | fleet membership and the fleet model | ordered members, logout latch, focus neighbour; `FleetModel`: one row per profile identity in the saved Load order (`profile_order.rs`), loaded or not, each with an explicit loaded flag, the marks (mark operations over the shown rows and status predicates: `MarkOp`, `MarkPredicate`, the tri-state `ShownMarks`), the filter and order, the fleet card (`FleetCard`: each bot's own, the focused bot's, or a pinned card, with a label rebuilt only when the card, focus or heading changes), the shown rows and their counts, rebuilt only when the core's rows or the vault's profiles moved. Each front end holds one and lends it through `Models::with_fleet`; the panel's Fleet window, rail and Grid and the TUI fleet pane read it |
| `fleet_plan.rs`, `session/fleet_plan.rs` | fleet decisions | `PlanFleetAction` freezes a fleet action (`FleetAction`) into a `FleetPlan`: one staged profile (`stage_plan_target`, a world bubble or the TUI cursor), else the marked identities (hidden ones too) or, with no marks, the loaded shown rows; the resolved card; the bots it would interrupt; the skipped rows with reasons; one prompt text; and whether it asks first (no-marks scope, an interruption, a settings copy or a world switch). One plan waits; a newer one replaces it and says so. The plan's own command with `Targets::Plan` runs over the frozen identities and card (never the live marks or Browse pick); `CancelFleetPlan` drops it and releases a copy's prepared scope. `FleetResult` follows the run's command record (failures, skips, done, pending) and logs one line when it settles |
| `session/world_switch.rs` | Switch world | the owner of each `SwitchWorld` member: the world save first (the profile's world field), then, only once it is durable, the existing `SwitchWorld` intent, whose turnover relogs the bot through the login queue; the member settles from the relog (a new session in game on the requested world), fails with what happened and is cancelled by a later Log in, Log out or Remove; the choices are the bound server's public worlds, read once at play start (none on a one-world server). One small record per switching bot, only while its switch runs. A switch plan (`FleetAction::SwitchWorld`, one staged bot or the marks) always asks first |
| `profile_order.rs` | saved Load order | the saved order in the shared preferences (`profile_order`), stored as one small member per profile keyed by its vault uid with a sort key (`"<uid>": <key>`, about 25 bytes per profile; a value holds at most 4,096 bytes, so the preferences service holds a large order one member per row, and one save replaces every changed or removed member in one file replacement, all or none): seeded in vault (alphabetical) order without a write, reconciled on each vault change (append new; a deleted profile's member stays stored, unordered, while its deletion is not durable, so a failed deletion gets its place back, and is removed by the next save once durable; a rename keeps the identity and so the place, whether it commits or fails); `OperatorSession::fresh_uid` allocates above every uid the vault and the order have held, so a new profile never takes a deleted one's place; a move changes only the moved profiles' keys (gaps; renumbered when spent, when a key would leave `±2^52`, or when a stored key is outside it); `ReorderOp` moves profiles by identity before or after a target in the full order, or puts the marked ones on top in their order. The session orders members, Load all and marked targets by it and writes it through the preferences service (`ReorderProfiles`: pending until durable, one audit line, the last durable order back on a failure) |
| `selection.rs`, `marked.rs`, `bulk.rs` | marked-bot selection and commands | marks keyed by profile identity; Assign, Assign & restart, log in/out on marked rows; one outcome per marked row |
| `group_walk.rs` | group walk | one WalkTo destination for every marked bot; panel picker and TUI map both call it |
| `walk_permissions.rs`, `nav_prefs.rs`, `map_bake.rs` | durable walk and nav preferences; terrain-bake consent | shared by both front ends; read fail closed |
| `map_decode.rs` | cooperative WalkTo image decode | one accepted shared-work ticket, one replaceable pending demand, and generation/image/key-fenced terminal custody |
| `operations.rs` | operation results | ids, per-member outcomes, bounded book, outcome journal the poll logs as `op#<id>` lines |
| `views.rs`, `progress.rs`, `login_notice.rs` | fleet and detail projections; run-progress wording; per-bot login notice | one row per member (phase, queue place, login-error retry wait vs operator hold, script state, walking, retained error, current login notice, newest operation, narrow label), fleet counts, and the selected slot's detail; rows re-derived only when their facts change; the login notice keeps a failed login apart from the phase until a successful login, a newer failure or the shared 15 s `ErrorExpiry` timeout; runtime, time since progress and levels gained |
| `resources.rs` | process resource meter | one probe thread sampling at 1 Hz (never on the UI frame); CPU, current and peak process RSS (peak never below current), summed traffic with per-slot-lifetime baselines; measuring / unavailable / error values, plus one narrow line for a header |
| `profiles.rs` | durable profile writes | one writer thread, ordered, last write per profile wins; results settle in `poll` |
| `profile_saves.rs` | operation-owned form-save records and structured write failures | a form save is registered under its write's operation id and the session is the only place it lives: `saves_in_flight` while unwritten, `take_settled_saves` once the write settled it; `WriteFailure` names the operation, profile and edit that failed |
| `profile_form.rs` | profile edit form feedback shared by the panel form and the TUI settings popup | keeps only the form generation and the notice; `saving(core)` tells a front end the form's save is unsettled, so leaving it asks first |
| `surface.rs` | front-end slot adapter | `SlotSurface`, `HeadlessSurface` |
| `log.rs` | structured operator log | per-slot and process rings (500 each), redaction, filtered `LogView`, Save log… |
| `log_file.rs` | per-session log file | shared off-by-default preference, background writer, size rotation, session pruning |
| `scripts/mod.rs` | script coordination | card library and catalog fill, per-profile assignment and pending Browse, parameter bags (legacy claim, typed edits pushed live once durable), Start / Start all / Stop all, Start settlement (assignment on Ready), notices |
| `scripts/native.rs`, `scripts/parameter_edit.rs` | native card schema, settings and typed parameter edits | presentation and fenced controls |
| `scripts/start_admit.rs`, `scripts/start_tally.rs` | paced Start-all admission and its running report | the login FIFO's shape applied to script Starts, so a bulk Start cannot dispatch every isolate at once |
| `scripts/marked.rs` | what marked commands need from the coordinator | resolve a card to its assignment; queue a restart behind a stop |
| `scripts/reload.rs` | reload and catalog-refresh transaction | worker validation, warning with exact runs and generations, confirm/cancel, fenced replacement |
| `scripts/sync.rs` | Apply to all (bulk parameter sync) | frozen same-card scope, per-profile writes, generation-fenced live push, separate persistence/live counts; confirmation holds new Starts, and accepted native copies retain that hold through preparation and the writer's durable completion (JS copies release after staging). A native copy keeps at most 48 saves and deliveries in flight and queues the rest, written as room frees; the hold lasts until the last queued save is accepted, and Cancel reports the still-queued bots skipped |
| `quester_paths.rs` | durable Quester Path configuration | asynchronous folder reload |

The walk-map decode corpus fixture compares every manifest-listed producer tile
through the shared controller with the legacy test-only decoder. It compares
serialized identity, generation, tile key and RGBA bytes, and checks ticket
release after every result. The legacy decoder is not a production fallback.

Accepted native copies keep their Start hold on the existing preparation and
profile-write operations, not on a separate completion tracker. Preparation
failure, Cancel, vault lock or slot removal releases the affected hold without
publishing the draft. A 30-second preparation/persistence wait limit lets a
stalled copy fail rather than strand queued Starts. A preparation that times out
cannot publish a late result; an already-submitted disk write cannot be recalled
and may still become durable after its wait fails. Such a late commit updates
the saved profile but is never pushed into a run admitted on the pre-copy bag.
Overlapping confirmations/copies share the host's hold token until every owner
has released it.

### panel

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | declares every module below |
| `main.rs` | `panel-play` entrypoint | flag parsing, window loop |
| `app.rs` | panel shell and frame composition | docking shell on the owned window loop |
| `session.rs`, `session/chooser.rs` | native adapter over `frontend_core::OperatorSession`; chooser credential scratch | `PanelSurface` (slot IO, draw/audio policy), render `Focus` mirror, nav paint, harness state; edit buffers and vault helpers |
| `session_catalog.rs` | catalog discovery and warmup | loading and transpile warmup |
| `profile_script.rs` | panel adapter over `frontend_core::Scripts` | focused profile, heading and catalog root; banner notices; external-loader Start failures |
| `window.rs`, `work_area.rs`, `ime.rs` | GPU and winit shell; monitor work area; IME bridge | config, surface, errors; launch and rail-grow fit; single-window physical-pixel IME |
| `game_view.rs` | game image texture | mailbox frames to texture |
| `input_capture.rs` | game input capture | keyboard queue and mouse streaming |
| `picker.rs` | WalkTo picker chrome | pan/zoom/plane/search; Walk/Teleport/selection via `host_play::walk_map` |
| `walk_map.rs`, `walk_map/overlay.rs`, `walk_map/labels.rs`, `walk_map/fixtures.rs` | app-owned WalkTo renderer | ≤24 terrain slots, one overlay (768×512 / 1.5 MiB cap, never per-tile quads), bounded shared-worker decode demand, label ranking, small PNG/POI fixtures |
| `nav_paint_cache.rs` | nav-paint cache | per-frame reuse of path paint |
| `grid.rs` | MultiBox grid layout | cell geometry |
| `fleet_toolbar.rs`, `fleet_toolbar_tests.rs` | fleet toolbar | the one toolbar draw function for the rail's top (sticky over the scrolling tiles), the strip's `fleet` section (while the rail is hidden: Grid, MultiBox off) and the Fleet window: the fleet card combo, the tri-state shown-marks box and Marks menu (with Move marked to top), filter and sort, the nine buttons labelled with their scope (labels rebuilt only when the model's generation moves; equal columns, pairs or one per row by width, never cut), the core plan's prompt with Confirm/Cancel and its one result line; a fold header (`fleet_toolbar_open`) keeps the summary, plan and result. Walk opens WalkTo in Group mode, Switch world the world picker; the rest plan through `fleet_actions.rs` |
| `fleet.rs`, `fleet_actions.rs`, `fleet_columns.rs` | Fleet window | the wide table over the shared fleet model (every profile, loaded or not) under the fleet toolbar: a name click focuses a loaded bot, its checkbox only marks it; toggleable status columns. `fleet_actions.rs` submits `PlanFleetAction`, then the plan's command with `Targets::Plan`; the Scripts window's copy entry uses the same flow and draws the plan row only while no toolbar is on screen. The rail and the Grid draw the same model's shown loaded rows (`app.rs`) |
| `world_picker.rs` | world picker | the one popup every Switch world entry opens (a rail or Grid world marker, the Fleet World/login cell, the toolbar's Switch world button); lists the core's cached public worlds with the bot's own ticked, labels built when it opens; a pick plans `FleetAction::SwitchWorld` through the toolbars' plan row |
| `rail.rs` | sidecar rail chrome | geometry and the status-dot colour of a row's light |
| `chrome.rs` | app chrome | menus and banners |
| `overlay.rs` | queue-card overlay | each image's own queue card from its fleet row |
| `paint.rs` | script-paint overlay | structured paint over the chatbox rect |
| `focus.rs` | focus tracking | focused slot and pane |
| `clipboard.rs` | native clipboard backend | OS pasteboard bridge |
| `build_info.rs` | deploy fingerprint | release label (`RELEASE`) and git stamp |
| `loadouts.rs` | loadout editor | equipment and supply CRUD |
| `script_picker.rs` | script browse picker | category order and badges |
| `name_picker.rs` | shared name-hit picker rows | Loadouts search and Debug name picker |
| `nav_settings.rs` | nav settings pane | find opt-ins and pack selection |
| `debug_panel.rs` | Debug command panel | content-derived command catalog; UI state only |
| `log_pane.rs` | log section | filters, search, follow, Copy and Save log… over the shared log |
| `live_harness.rs` | headed live and smoke watch | tick and capture qualification; gate decisions come from `host_play::live_gate` |
| `headed_record.rs` | opt-in headed video recording | `HEADED_RECORD*`; see [the harness](harness.md) |
| `ui_state.rs` | persisted UI prefs | focused profile and collapsed maps |
| `theme.rs` | theme tokens | colors and metrics |
| `wall.rs` | wall UI state | chooser, grid, render-all warning (membership is `frontend_core::Fleet`) |
| `srgb_present.rs` | present-path test helper | test-only |
| `manual_click_live_tests.rs`, `test_support.rs` | live regressions and fixtures | grouped, not owners |
| `test_alloc.rs` | test allocator | the panel test binary's allocator: counts allocation calls per thread for the warm-frame checks; test-only |

### tui

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | second view of `Play`, no GPU |
| `main.rs` | `tui-play` entrypoint | flags and run modes |
| `bin.rs` | headless session and dispatch | `OperatorSession<()>` + headless surface; copies the core projection into the app when it moved; hands every terminal event to the app and dispatches the returned action onto the core; `--live` (with `--catalog-core` / `--pair-core`) decides through `host_play::live_gate`; `--map-bundle` release bake |
| `app.rs` | view model and pane behaviour | app state, including copies of the core projection (fleet rows and counts, selected detail, meter); map, script and chat pane keys and clicks |
| `layout.rs` | shell geometry | size classes (compact below 100x30 with 80x24 the floor, standard from 100x30, large from 160x45), pane rects, the last draw's hit regions |
| `shell.rs` | shell drawing | header (fleet counts, compact meter), fleet and detail panes, log drawer, footer naming the keyboard scope |
| `input.rs` | key and mouse routing | one model: popup/overlay, then text field, then global chords, then the focused pane; mouse hit-tests the last draw |
| `commands.rs` | operator command vocabulary | labels, target scope, availability and reason, per-pane shortcut tables shared by keys, buttons, palette and help |
| `fleet.rs` | fleet table | cursor, scroll and filter edit over the core fleet model's shown rows (marks, filter and order are the model's); the cached header line (card, sort, marked count) with the cells of its card and sort parts, which a click opens or cycles |
| `fleet_card.rs` | fleet card picker | Own, Focused, then the Browse cards by category; Enter pins, Esc keeps the card |
| `world_picker.rs` | world picker | `W` on the cursor row or the palette's Switch world of marked…; the core's cached public worlds with the row's own ticked; Enter plans `FleetAction::SwitchWorld`, whose prompt shows in the plan overlay |
| `overlay.rs` | app overlays | help, palette, the fleet plan's confirmation (the core prompt verbatim), context menus, message viewer, manual walk |
| `palette.rs` | command palette | filtered commands with scope, key and unavailable reason |
| `help.rs` | help overlay | focused pane's keys first, searchable |
| `log_pane.rs` | log view | Logs tab and log drawer over the shared `frontend_core::log` |
| `map.rs` | WalkTo map widget | dots, route polyline, selection, cell/tile mapping |
| `chat.rs` | chat and dialogue pane | chat ring, modal continue and answer |
| `status.rs` | status pane | the selected slot's projected detail and the full meter |
| `script_params.rs` | script parameter editors | schema-driven editors |
| `script_shape.rs` | script pane widgets | browse, start, pause, stop, load |
| `loadouts.rs` | loadouts popup | worn gear and carry CRUD |
| `settings.rs` | settings popup | random toggles and nav opt-ins |
| `stderr_capture.rs` | fd-2 redirect while the alternate screen is up | client worker `eprintln!` must not corrupt the display |
| `manual_click_live_tests.rs`, `test_support.rs` | live regressions and fixtures | grouped, not owners |

### e2e

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade | exposes `suite` |
| `suite/mod.rs` | suite driver facade | selection, launch, validate, ledger |
| `suite/manifest.rs` | frozen suite manifest | reference rows plus adapter map |
| `suite/select.rs` | case selection | level and `--only` semantics |
| `suite/child.rs` | child process ownership | construction, supervision, logging |
| `suite/child/config.rs` | launch configuration | profile and environment |
| `suite/cli.rs` | CLI verbs and reporting | list, dry-run, run |
| `suite/identity.rs` | run identity and resume gate | receipt binding, unchanged resume |
| `suite/ledger.rs` | run ledger | durable attempt records |
| `suite/receipt.rs` | receipt parsing and validation | terminal lines and captures |
| `suite/receipt/external.rs` | external-loader protocol qualification | loader receipts only |
| `bin/e2e-suite.rs` | suite entrypoint binary | flag list in help |
| `bin/e2e-suite-fixture.rs` | disposable receipt fixture child | offline verification only |

The crate's `tests/` hold the engine-backed live cells (`nav_*`, `scenario_walk`,
`panel_view`, `booth_capture`) and the offline `suite_offline` test.

## Native script machines

The native cards are compiled into the host and use no V8 isolate. They drive
host verbs directly, hold per-slot state in bounded structs, and read the
snapshot Rust already holds.

| Machine | Card | What it does |
| --- | --- | --- |
| Quester (`script/src/quester/`) | `Quester` | Runs built-in quest Paths for revision 289: the Path compiler turns one authored quest document into stage-gated steps, the runner executes them on host verbs (bank, shop, production, gather and combat adapters), and progress resolves from journal evidence. Paths are documented in [quest-paths.md](quest-paths.md). |
| Grinch (`script/src/grinch.rs`) | `Grinch` | Collects selected visible base ground-item IDs inside a same-plane anchor square, deposits selected IDs only, then returns to the exact anchor. |
 | Combat (`script/src/combat/`) | shared machine, no card | One observed-state planner for melee, ranged and magic: prayers, sips and eating. Quester combat steps drive it through the typed Path adapter; Load scripts drive it through `api.combat` sessions (`slot/combat.rs`, `api_combat.rs`, `combat_session.rs`). |
 | Sherlock (`script-js/src/compat/sherlock.rs` on `script-js/src/compat/clue/`) | `Sherlock` | Runs clue trails on the one clue step machine, one phase slice each, on the slot's pump thread. Sherlock is available only in builds with the JS runtime. Sherlock was meant to be native. In practice it became a port of the compatibility clue machine, so it needs the JS runtime build until the native clue rewrite replaces that machine. Sherlock itself never creates a V8 isolate. |

## Compatibility shim boundary

Catalog scripts written for the frozen catalog API run unmodified through
the compat isolate (JS API v1). The shim maps those names onto host verbs,
step machines or `not impl`: name maps, value coercion and await plumbing
only. Loops, tables, decisions, retries and sequencing live in Rust step
machines that read the snapshot Rust already holds. Scripts written for the
host API run through the same isolate as JS API v2 ([`api/js-api-v2.md`](../api/js-api-v2.md)).

 The only JS↔Rust wire is the FlatBuffer: `IsolateBuf` posts snapshots into
 each isolate and decodes the shim's interact batches back. The wire codec lives in
 `script-js` as `src/wire/isolate_fb.rs` with its schema at `crates/script-js/schema/isolate.fbs`
 and bindings at `crates/script-js/schema/generated`. Bounded
 synchronous Rust calculations with typed value marshalling are the one
 authorized exception; they compute and never send a game action.

## Navigation pack and server profiles

The nav pack is format v17 (magic `274V`). Decoding rejects v16 and older as
`BadVersion`: rebake custom packs with `nav-pack`. Three sidecars ride beside
the pack: raw flags (`274F`/1), paint-reach (`274R`/1) and static canlight
(`274L`/1); `274bot.navpois` (`274P`/1) is the fourth. Application builds bake
and stage the selected revision's navigation automatically
(`BOT_NAV_REVISION`, default 289; `BOT_NAV_BUILD=require|skip`); `nav-pack` is
only for deliberate custom-input bakes, selected with `--nav-pack`.

`--profile NAME` picks a server profile from `~/.274bot/servers.json`,
created on first use with `local-274`, `local-289` and `rs2b2t`. `--rs2b2t`
selects the public profile; local profiles need an engine directory
(`--engine`, `ENGINE_DIR`, or the profile's `engine_dir`) and public profiles
need nothing. Profile files and the instance lock are owned by `host-play`
(`servers.rs`, `profile_*.rs`, `instance_lock.rs`).

## Per-bot memory principles

- Immutable content is shared once per process (`Arc`; parsed game facts
  through `OnceLock` and `Arc`), never copied per bot.
- Per-bot state is bounded and inline: fixed-size structs with asserted
  size budgets (for example `Combat`, `WalkGuard`, `Threat`), no per-tick or
  per-frame allocation on hot paths.
- Measurement never equates allocation counts with RSS. The opt-in
  `BOT_MEMORY_N` harness (see [harness.md](harness.md)) reports RSS per
  additional bot; sample fields are separate domains and are never summed.

## Runtime ownership

These are product boundaries. The crate checker (in `274bot-arch`, run via `tools/architecture/run.sh`) does not prove them.

| Boundary | Owner | Not the owner |
| --- | --- | --- |
| Operator window, ImGui chrome, MultiBox, game blit, input into slots | `panel` | client applet UI, script isolate |
| Headless operator view (raster Off) | `tui` | GPU renderer, a second `Play` |
| Operator lifecycle: vault, fleet membership and latch, selected bot, marked-bot commands, Load/Log in/Log out/Remove intent, removal settlement, operation results, script assignment/parameters/reload/bulk coordination; fleet/detail projections and the process resource meter | `frontend-core` | panel/tui chrome, a second login queue or script runtime, per-row samplers |
| Slot workers: spawn/stop/reap, connection and readiness, login FIFO, tick pump, script start/pause/stop/load execution | `host-play` (`Play`) | panel/tui chrome, `frontend-core`, `e2e` |
| Native per-slot host APIs, snapshot/think, random-event guardian | `host` | nav internals, JS |
 | Script kernel: native card machines and slot lifecycle | `script` | JS policy/routers, a foreign runtime, the V8 isolate |
 | JS runtime: V8 isolate, compat machines, catalog and library, isolate wire schema | `script-js` | native card ownership, native slot lifecycle |
| GPU 3D / CpuPix3D (`BOT_CPU=1`), packet/doAction Java shape | `client` | host bot-action API, 274bot crates in the client repo |

The JS shim must not grow its own API or shared policy. Compatibility maps
the JavaScript surface onto Rust host APIs.

### Admitted work execution

`api::work::WorkFamily::policy` declares execution and byte admission for every
registered family. `script::work_admission::WorkService` owns one identity and
custody ledger, regardless of where a job executes.

Route, Inspection, BankPick and MapBake use one short-lived thread per accepted
request. Their original thread names and default stack sizes remain unchanged.
A request drives its cooperative job to a terminal on that same thread.
Yielded steps never enter the shared background queue. Card, catalogue, profile
and walk-map decode jobs keep their existing background execution until their
separate owner migrations.

The ledger retains the 64-job and eight-operation-handle limits. It accounts
for every declared input, scratch, result and progress reservation. Navigation
families keep their 192 MiB per-job bounds without a global navigation byte cap.
MapBake and background families consume the existing bounded byte allowance.
The published byte total includes both categories.

Thread creation precedes job transfer. A start gate prevents execution before
registration completes. Spawn failure returns the original job with
`ThreadStartFailed` and restores its capacity credit. Each request retains its
own active state, cancellation and contained-panic boundary.

Pump paths never wait for or join a thread. Shutdown uses one fixed deadline
and joins only threads that already returned. Unreturned request threads are
reported separately from the background worker. Their custody stays charged
until actual return, even if shutdown must detach their handles.

## Behavioral invariants (review unless noted)

The checker in `274bot-arch` does **not** pretend to prove the following. They remain
independent-review and test requirements unless a later, robust check exists.

- **Isolate ↔ host and game-action wire:** FlatBuffers only. No extra JS↔Rust
  host transport. Vault/settings/serde JSON elsewhere is unrelated and is
  not banned by word.
- **Typed synchronous Rust calculation helpers inside the isolate** are an
  authorized exception: bounded calculations with typed value marshalling.
  They are not permission for in-isolate game actions, extra host RPC, or JS
  policy.
- **No JS policy growth** in the shim. Fail-closed helpers and configuration
  stay in Rust.
- **Lifecycle ordering** (login, `ingame && scene_state == 2` live gates,
  start/pause/stop/reload, pair/external exclusivity) is behavioral. Review
  and live/harness tests own it.
- **Last-FBO freeze** while `scene_state == 1` is a client rendering
  invariant. Review and existing client tests own it; this graph check does
  not inspect renderer code.

## Tests versus production

Sibling `*_tests.rs` files are **layout**, not production decomposition.
Moving an inline test module into a sibling file does not split ownership.
The per-crate tables above mark those rows as test bodies.

Every crate except `vault` is split into the modules listed above; `host-play`,
`panel` and `tui` keep most files at the crate root with a few child
directories (`walk_map/`, `map_cache/`, `session/`). This checker does not
gate on sibling paths.

`e2e` and ignored live tests in `host-play` are wide orchestration. Linking
those crates for tests is not a claim that test code is the product owner of
nav, rendering, or the isolate.

## Lint suppression ratchet

`#[allow]` suppressions accumulate silently: each one turns a lint off
forever, including after the code that needed it is gone. Two mechanisms
stop that. Both live outside the crate-graph checker above; this section
documents them together.

**Workspace lints (per-crate opt-in).** The `[workspace.lints.clippy]` table
in the root `Cargo.toml` holds the set: `undocumented_unsafe_blocks` and
`unwrap_used` (warn), plus the suppression-hygiene denies
`allow_attributes` (suppressions must be `#[expect]`, so one that stops
firing is an error) and `allow_attributes_without_reason`, plus the six
approved lints `redundant_clone`, `significant_drop_in_scrutinee`,
`unchecked_time_subtraction`, `needless_collect`,
`large_types_passed_by_value` and `assigning_clones` (deny), plus `todo`
and `unimplemented` (deny), which stop a `todo!()` or `unimplemented!()`
stub from shipping as a runtime panic. A crate opts
in with `[lints] workspace = true` in its manifest when a refactor reaches
it and stays opted in; `vault` is the first. The two stub lints also hold in
every crate that has not opted in: CI passes `-D clippy::todo -D
clippy::unimplemented` to every main-workspace clippy command (the client
fork is a separate workspace and is not covered). Only `unwrap_used` is scoped
to non-test code (via `allow-unwrap-in-tests` in `clippy.toml`): clippy
1.98 offers no per-lint test exemption for the other lints, so opted-in
crates keep their tests clean too.

`nav-executor` also opts in. Its callback type alias and grouped test
geometry remove the `type_complexity` and `too_many_arguments`
suppressions. Its suppression baseline is zero.

**Suppression-count ratchet.** `tools/lint-ratchet/lint_ratchet.py`
(Python 3, stdlib only; run as `python3 -B`) counts `#[allow(...)]`,
`#[expect(...)]`, crate-level `#![...]` and `#[cfg_attr(.., ...)]`
suppressions in `crates/**/*.rs` per crate × lint, with test files counted
separately, and compares against the committed
`tools/lint-ratchet/baseline.json`. Any count above its baseline fails, as
does a suppression for a lint absent from that crate's baseline entry;
lowering a count is free. `--update` rewrites the baseline. The parser is
fail-closed: anything suppression-like it cannot read (an unparseable lint
path, an unbalanced attribute, a macro metavariable such as `$lint:meta` in
suppression position) is an error with file:line, and `--update` refuses to
write a baseline while such errors exist. It handles `reason = "..."`
strings containing commas and parentheses, multi-line attributes, nested
`cfg_attr` to any depth, whitespace around `::` and before the paren
(`clippy :: needless_collect`, `allow (dead_code)`). Before matching, one
normalization pass canonicalizes the token stream: whitespace/comments
between tokens are dropped (even between `#`, `!`, `[` and the path, so
`# [allow(..)]` counts), the `r#` raw-identifier prefix is stripped, and
Unicode format characters (`Cf`, e.g. U+200E/U+200F) are removed; any other
non-ASCII inside a path stays and fails closed. Suppressions written
inside macro bodies or passed as macro arguments count, conservatively: a
macro-emitted suppression is real, and over-counting hides nothing. The
 FlatBuffers output `crates/script-js/schema/generated/isolate_generated.rs` is
 explicitly exempt (regenerating it rewrites its own allows). The
`lint-ratchet` lane in the admissions matrix runs the check plus
`lint_ratchet.py --self-test` (the permanent parser tests in
`tools/lint-ratchet/test_lint_ratchet.py`), and
`tools/lint-ratchet/ci-snippet.yml` is the step to fold into the single
`architecture`/lint CI job.

Limit: the parser is a text tool, not the compiler. Exotic spellings beyond
the normalization above are covered by clippy's `allow_attributes` lint in
opted-in crates (the compiler sees every real attribute) and by the
reviewer. No further parser rounds.

**Adding a justified suppression.** Write
`#[expect(lint, reason = "...")]` with a reason that says why the code
needs it, and bump the baseline in the same diff:

```bash
python3 -B tools/lint-ratchet/lint_ratchet.py --update
```

## Changing a boundary

An operator commit in protected `274bot-arch` is required for a new crate
edge (including a new target cfg or cross-crate feature request), an edge kind,
a migration, an optional-dependency activation, a vendored path package, a client manifest change, a root patch or replacement entry, or a checker change.
The operator edits `policy.toml` where the rule change needs it.
First merge current `dev` into the branch being merged.
Regenerate the baseline from that merged branch tree in the same arch commit:

```bash
python3 -B check.py --emit-provenance-baseline /path/to/merged-branch > graph-provenance.json
```

Review removed records. Retain records that other active branches still need.

The operator publishes first; the product change lands after. Only the
operator can push `274bot-arch`; agents cannot. Propose the policy edit as
a patch or branch bundle for the operator to review and push.
Outside the pinned client manifests, purely local features with no optional dependency or cross-crate edge need no operator commit.
Fine ownership/seam/inventory data also needs no operator commit. Fine-data
changes (ownership rows, inventory rows, foreign-type rows) are ordinary
product diffs in `tools/architecture/`, reviewed like code — the checker
loads them from the checked tree, refuses malformed or stale data, and
cannot widen the protected edge policy.

1. Update the tables on this page so the public description matches.
2. Run the local command above (point local-only `ARCH_REPO` at a local
   `274bot-arch` clone to prove an unpushed policy change before review).
3. Get review. Do not land a graph change as a drive-by import.
