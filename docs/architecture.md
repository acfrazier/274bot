# Architecture

274bot is a Rust bot host over a vendored client library. Ownership is crate
and runtime boundaries, not a line-count budget and not a copy of a TypeScript
lint stack.

The Cargo workspace graph is **enforced**. Runtime semantics that a manifest
cannot prove stay **review requirements**. Changing a crate edge is a
deliberate policy edit plus documentation and review; do not sneak a reverse
or convenience dependency through an alias, target table, or optional rename.

## Local command

Python **3.11+** (stdlib `tomllib`). No Rust rebuild.

```bash
python3 tools/architecture/check.py
python3 tools/architecture/check.py --self-test
```

If `python3` is older than 3.11, use `python3.11` or newer. The checker reads
workspace and member `Cargo.toml` files, including `[dependencies]`,
`[dev-dependencies]`, `[build-dependencies]`, `[target.*.dependencies]`,
`optional`, `workspace = true` aliases, `path`, and `package` renames. It
does not parse `.rs` with regex and does not compile.

Membership is the explicit `workspace.members` path list plus a root
`[package]` when present (Cargo's implicit root member). Globs, `exclude`,
and other unsupported workspace shapes fail closed. A path dependency on a
crate that is not a listed member still appears in the scan and must be a
policy member or external.

GitHub Actions runs the same two commands in the `architecture` job. That job
does **not** replace `fmt`, `clippy`, or `test`. Those gates stay as they are.

The explicit allowlist is [`tools/architecture/policy.toml`](../tools/architecture/policy.toml).
Every workspace member must appear there. Unknown members fail closed. crates.io
and other non-path crates are ignored. `client` is a workspace *alias* to
`vendor/fr-client-rust/crates/client`, not a 274bot workspace member; it is
listed as an architecture external.

## What this is adapted from

rs2b0t's strict CI on `origin/main` makes **test, typecheck, lint, prose, and
prose extras** mandatory. ESLint `no-restricted-imports` / restricted globals
encode layer ownership (adapter-only client internals, api must not import
scripts/UI, data/geometry stay leaves). Scripts such as `audit:exports`,
`check:api-contract`, and `audit:e2e-split` exist but are **manual** — they
are not CI gates.

This repo adapts the **mandatory ownership lint** idea to the actual Rust
crate graph. It does not copy ESLint, Vale, TypeScript commands, export
audits, API-contract dumps, e2e-split auditors, or comment/line-budget
punishment.

## Production crate ownership

| Crate | Owns | Must not become |
| --- | --- | --- |
| `vault` | Encrypted profile store | A dependant of api/host/ui |
| `api` | Host-facing read/act/settle types mapped onto `client` | A dependant of host, nav, script, or UI |
| `host` | Native slot/tick APIs, login FIFO, guardian | A dependant of nav, script, host-play, or UI |
| `nav` | Packed world, router, Traveller | Script isolate or operator UI |
| `script` | Compiled cards, Load isolate, thin JS shim | A production dependant of `client` or `nav` |
| `host-play` | Shared `Play` lifecycle over host/script/nav/vault | A second panel or client renderer |
| `frontend-core` | Operator lifecycle shared by panel and TUI: vault, fleet membership and logout latch, selected bot, Load/Log in/Log out/Remove, non-blocking removal, script Start/Stop settlement, operation results, the structured operator log | A second `Play`, login queue, world or script runtime; a dependant of panel/tui or of window/terminal libraries |
| `scenario` | Shared headed/headless live scenario runner | Panel/TUI chrome |
| `panel` | Native ImGui UI, winit/wgpu window, game blit | The client 3D renderer or isolate runtime |
| `tui` | Headless operator view over the same `frontend-core` session | A second kernel or GPU loop |
| `e2e` | Wide **test orchestration** (library + suite binaries) | Production UI or script-kernel ownership |
| `client` (external) | 274/289 client lib, GPU/CPU raster, last-FBO | Bot action API or 274bot crates in its repo |

Authoritative edges and reasons live in the policy file. The current
intentional graph, derived from the manifests:

- `api` → `client`
- `host` → `api`, `client`, `vault`
- `nav` → `api`, `client`
- `script` → `api`, `vault` (runtime); `client`, `nav` **dev-only**
- `host-play` → `api`, `client`, `host`, `nav`, `script`, `vault`; `scenario` **optional**; `nav` also **build**
- `scenario` → `api`, `client`, `nav`
- `frontend-core` → `api`, `host`, `host-play`, `script`, `vault`
- `panel` → `api`, `client`, `frontend-core`, `host`, `host-play`, `nav`, `scenario`, `script`, `vault`
- `tui` → `api`, `client`, `frontend-core`, `host-play`, `nav`, `scenario`, `script`, `vault`; `host` **dev-only**
- `e2e` → `api`, `client`, `host`, `host-play`, `nav`, `scenario`, `vault`

`e2e` does **not** Cargo-depend on `script`, `panel`, or `tui`. The suite
launches product binaries as child processes. That is orchestration, not a
claim that `e2e` owns those crates' production behavior.

### Truthful exceptions (enforced as written)

- **`api` → `client`** is intentional. `api` maps host types onto the
  vendored client. It is not a reverse `client` → `api` edge.
- **`script` → `client` / `nav`** are `[dev-dependencies]` only. Promoting
  either to a normal/optional/target dependency fails the checker.
- **`tui` → `host`** is `[dev-dependencies]` only. Production TUI composes
  through `frontend-core` and `host-play`.
- **`frontend-core` → `host`** carries only the `SlotInput`/`FrameBuf`
  handles a surface passes to `Play::try_spawn_slot`.
- **`frontend-core` → `api`** carries only the `api::hostlog` facade: the
  operator log store is its sink.
- **`host-play` → `scenario`** is optional (feature-gated harness), not a
  default required edge. A required `[target.*.dependencies]` edge does
  **not** satisfy an optional-only allow. `optional = true` on a target
  table stays optional. `{ workspace = true, optional = true }` is read
  from the **member** table. `[workspace.dependencies]` cannot set
  `optional` (Cargo: "workspace dependencies cannot be optional"); that
  form is rejected rather than inherited.
- **`host-play` → `nav`** is both a runtime dependency and a build-dependency
  (bake/stage nav identity). Both kinds are listed.

If a future change needs a new edge or kind, edit the policy and this page
and get review. Do not add a second hidden graph.

## Per-crate module ownership

Paths relative to `crates/<crate>/src`, as they are at `bb51386a6`.
`api`, `e2e`, `host`, `nav`, `scenario`, `script` are split as listed.
`host-play`, `panel`, `tui` are listed as they are: one file per owner,
no split children. `*_tests.rs` rows are test bodies, not production
owners.

### vault

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | encrypted profile store | single-module crate |

### api

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | declares every module below |
| `snapshot.rs` | snapshot storage and rebuild | generation orchestration, read accessors |
| `snapshot/views.rs` | snapshot view rows | typed row shapes |
| `snapshot/decode.rs` | client decoding into views | packet bytes to rows |
| `snapshot/context.rs` | read context over a snapshot | query-time context |
| `query.rs` | borrowing family queries | `Query` builder and per-family traits |
| `query/reach.rs` | scene reachability | `SceneQuery::can_reach` options |
| `query/widget_search.rs` | widget and button lookups | snapshot-level |
| `query/loc_approach.rs` | loc approach mask | footprint plus wall flags |
| `interact.rs` | send vocabulary and dispatch | `Driver` seam, accepted-send semantics |
| `hostlog.rs` | host logging facade | `host_log!` categories, slot-log allowlist, stderr under `BOT_DEBUG`, one process sink |
| `interact/driver.rs` | client transport mapping | `doAction`, `tryMove`, `out`, ISAAC intact |
| `game_data.rs` | generated game facts by revision | decoding, indexes, lookups |
| `content.rs` | shared rock names and loot predicate | revision facts stay in `game_data` |
| `ent.rs` | Ent NPC identity facts | revision-independent ids and lifetime |
| `obj_names.rs` | item and loc definition views | id-to-name table for scripts |
| `prot.rs` | legal send table and builders | writes via `Out`, never raw opcode |
| `settle.rs` | pollable settle evidence | host drives polls per tick |
| `native_input.rs` | input identity and revoke mutex | not mouse policy |
| `line_of_sight.rs` | line-of-sight ray and query order | not the Reach flood |
| `random.rs` | cross-crate random types | detect and act stay in `host` |
| `prayer.rs` | prayer queries | over fact rows and observed stats |
| `quest_facts.rs` | quest identity and prereqs | over the posted family |
| `shop_facts.rs` | shop preset facts | ShopBuyout presets only |
| `clue_facts.rs` | clue row lookup | over the posted trail family |
| `clue_logic.rs` | held-step identify | first held step in posted order |
| `clue_pack.rs` | trail pack budgeting and keep predicate | frozen pack-plan arithmetic |
| `clue_puzzle.rs` | sliding-puzzle plan | frozen grouped search over caller rows |
| `cake_stall.rs` | Baker-stall pins | posted facts, selection in `script` |
| `gather_methods.rs` | gather-methods query | over the posted family |
| `gather_tools.rs` | gather-tool identity, use, wield | posted facts |
| `named_banks.rs` | frozen bank catalog, eligibility and selected-content access rows | stand resolution lives in `nav` |
| `game_data_tests.rs` | test body | logical `game_data::tests` |

### host

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | slot pump and crate facade | one OS thread per slot |
| `slot.rs` | per-slot drain pump | gens diff, tick synthesis, dirty families |
| `slot_io.rs` | frame and input channels | mailbox, input, park and wake |
| `login_queue.rs` | login FIFO and backoff | production rate limits |
| `auto_run.rs` | auto-run toggle | run-energy threshold |
| `random.rs` | random detection and shared vocabulary | stateless detect, caller-owned cooldowns |
| `random/guardian.rs` | act, hold, and dialog state machine | trap holds, cooldown writes, knock edge |
| `random/maze.rs` | maze graph, route, and phase | port of the frozen maze logic |
| `random/maze/layout.rs` | static maze layout data | copied layout rows |
| `lib_tests.rs`, `random_tests.rs`, `random/maze_tests.rs` | test bodies | grouped, not owners |

### nav

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | declares every module below |
| `arrival.rs` | arrival detection | on tile or adjacent to solid |
| `bake.rs` | shared world bake | one derivation for CLI and build script |
| `bank_fetch.rs` | BankBudget fetch-and-wear plan | plan only, router stays fail-closed |
| `bundle.rs` | build-time bundling and stamps | identities, no runtime state |
| `camera.rs` | path-facing orbit yaw | pure, host writes yaw per observe |
| `canlight.rs` | canlight sidecar bake | static allow-loc-add subset |
| `collision.rs` | whole-world collision bake | four planes plus walkable word |
| `essence.rs` | essence-mine return latch | session-gated return hop |
| `grid.rs` | walkability grids | step-grid surface |
| `manifest.rs` | bound pack manifest | bake identity checks |
| `map.rs` | bounded map-data boundary | errors, capped records/text and header-first checks; no UI or game actions |
| `map/identity.rs` | map identity byte layouts | independent image/catalogue keys; exact encoder/dependency policy |
| `map/poi.rs` | source-tagged discovery evidence | classifier, physical placements and separate nav-bound stands; not account eligibility |
| `map/spatial.rs` | map geometry and selection | north-up transforms, 24-slot LOD selection, radius-16 optional snap |
| `map/formats.rs` | checked map payload codecs | client catalogue, image manifest and data-only `274P`/1 navpois |
| `map/cache.rs` | publication readers and checkpoints | typed partial/ready entries; bounded reads and requested-payload verification |
| `map/records.rs` | offset-only maps.bin access | header index plus one reusable record buffer, not an archive/world copy |
| `map/tests.rs` | map contract boundaries | identity, malformed data, bridge planes, publication and spatial behavior |
| `named_banks.rs` | selected-world bank stand resolution | captured account eligibility stays with the caller, no `script` dep |
| `pack.rs` | pack wire codec and bake hub | pack encode and decode, stable exports |
| `pack/config_parse.rs` | pack config parsers | content config text |
| `pack/mapsquare.rs` | mapsquare parse and geometry | mapsquare bake rows |
| `pack/banks.rs` | bank table codec and derivation | booth tiles and stands |
| `pack/sidecars.rs` | sidecar formats and codecs | flags, paint-reach, canlight |
| `paint.rs` | nav-paint buffers | no imgui, no client draw |
| `router.rs` | world-space Dijkstra | total-ticks cost, gated edges |
| `router/grid.rs` | legacy grid A-star | `find_on_grid` for traveller and harness |
| `tile.rs` | tile coords and distance | x, z, level |
| `transport.rs` | transport graph orchestration | types, ordering, shared cost and geometry |
| `transport/condparse.rs` | shared condition text helpers | fail-closed, same behavior |
| `transport/index.rs`, `transport/script_text.rs` | shared placement readers and script text | producers, consumed by family derivers |
| `transport/observable.rs` | quest-journal observability proof | green-row rule |
| `transport/gates.rs`, `transport/quest_doors.rs`, `transport/doors.rs`, `transport/brass_key.rs`, `transport/membergate.rs`, `transport/webs.rs`, `transport/vertical.rs`, `transport/shortcuts.rs` | door, gate, vertical, and shortcut families | one family each |
| `transport/static_routes.rs`, `transport/npc_hops.rs`, `transport/gliders.rs`, `transport/spirit_trees.rs`, `transport/levers.rs`, `transport/toll.rs`, `transport/magic_guild.rs`, `transport/ranging_guild.rs`, `transport/zanaris.rs`, `transport/teleports.rs`, `transport/wilderness.rs` | fixed-route, guild, teleport, and wilderness-legality families | one family each |
| `traveller.rs` | follow facade and scheduler | `FollowRun` start and step, shared state |
| `traveller/snapshot.rs` | snapshot-query helpers | scene reads for hops |
| `traveller/dialog.rs` | dialog and teleport-send helpers | chat and teleport sends |
| `traveller/walk.rs` | walk-hop execution and reporting | walk targeting |
| `traveller/transport_hop.rs` | transport-hop execution | approach and door execution |
| `traveller/legacy_grid.rs` | legacy-grid execution | via the stable grid API |
| `walk_destinations.rs` | shared walk-destination pins | town tiles for WalkTo confirm |
| `world.rs` | bound world handle | packed collision plus transport graph |
| `world_state.rs` | search gating facts | fail-closed edge requirements |
| `bin/nav-pack.rs` | nav-pack CLI | whole-world bake entrypoint |
| `bin/nav-input-audit.rs` | input audit CLI | offline cache and content audit |
| `bin/cache-content-id.rs` | content identity export | read-only identity |
| `required-content-274.tsv`, `required-content-289.tsv` | canonical content inventory | build-checked data, not code |
| `arrival_tests.rs`, `bake_tests.rs`, `bank_fetch_tests.rs`, `bundle_tests.rs`, `camera_tests.rs`, `canlight_tests.rs`, `collision_tests.rs`, `essence_tests.rs`, `lib_tests.rs`, `manifest_tests.rs`, `named_banks_tests.rs`, `pack_tests.rs`, `paint_tests.rs`, `router_tests.rs`, `traveller_tests.rs`, `walk_destinations_tests.rs`, `world_tests.rs`, `world_state_tests.rs`, `bin/nav-pack/nav_pack_tests.rs`, `transport_tests.rs` | test bodies | grouped, logical `<owner>::tests` |

#### Native map data boundary

The public `nav::map` modules define the data/spatial contract independently of
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
the eventual PNG decoder still owns full chunk/CRC validation.

Only `PartialEntry` reads `.<key>.partial` checkpoints. Ready readers require a
canonical complete directory and validated manifest; ordinary image open does
not hash/decode the pyramid. Resume verifies completed units and their exact
policy identity. The cache-job owner still owns the prepared-cache `Arc`, locks,
atomic publication, cancellation and quotas. Renderer/UI integration is separate.


### script

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade: `Script`, `ScriptCtx`, `SlotScript` | `load` feature gates the isolate |
| `ctx.rs` | compiled `Script` trait and per-tick context | driver send-side, no `client` or world |
| `slot.rs` | per-uid runner lifecycle | intent and presence gates, tick edge |
| `slot/pending.rs` | bank settlement records | host-owned pending accessors |
| `machine.rs` | step-machine host | one multi-tick behavior in Rust |
| `observed.rs` | per-isolate decoded scene | delta-merge, owned rows |
| `isolate_fb.rs` | isolate wire codec | hand-written builder and reader, no JSON per tick |
| `host_js.rs` | generated Host JS types | from verb tables, not rs2b0t names |
| `content.rs` | curated sites and hostile predicate | bank aliases posted via the shim payload |
| `ent.rs` | Ent lookup over caller rows | no scene scan |
| `bank_access.rs`, `bank_deposit.rs`, `bank_op.rs`, `bank_open.rs`, `bank_withdraw.rs` | bank open, op, deposit, and withdraw steps | machine families, shared pieces |
| `clue.rs` | clue session machine | token, clock, identify, verbs |
| `clue/scene.rs`, `clue/puzzle.rs`, `clue/entrana.rs`, `clue/acquire.rs`, `clue/shop.rs`, `clue/talk.rs`, `clue/combat.rs`, `clue/verbs.rs`, `clue/family.rs` | clue phase helpers | one phase slice each |
| `sherlock.rs` | Sherlock card wiring | marshals the frame, maps verbs, no second solver |
| `hunt.rs` | hunt framework on the machine host | sessions and runs, child nesting |
| `hunt/wait.rs`, `hunt/teleport_out.rs`, `hunt/acquire.rs` | hunt family slices | three distinct families |
| `hunt_bank.rs`, `hunt_bank/plan.rs` | hunt bank runtime and planning | stages in root, plan projection in child |
| `hunt_fight.rs` | fight aggregate and shared projection | policy, clocks, field pick |
| `hunt_fight/hold.rs`, `hunt_fight/retreat.rs`, `hunt_fight/walk_spot.rs` | fight auxiliary machines | one machine plus its Kind each |
| `hunt_cell.rs`, `hunt_key.rs`, `hunt_lair.rs`, `hunt_leave.rs` | single-phase hunt machines | one phase chain each |
| `hunt_catalog.rs` | hunt tables and small rules | coordinates and pure helpers, no sends |
| `cake_stall.rs`, `cake_stall/runtime.rs` | stall selection and steal loop | facts in `api`, sequence here |
| `production.rs` | chat-make sequencing | Make-X and panel family |
| `fire.rs` | fire lighting and lane ranking | frozen light loop |
| `food_policy.rs`, `supply_v2.rs`, `boost_potions.rs`, `prayer.rs`, `autocast.rs`, `ranged.rs`, `special.rs`, `melee_weapons.rs`, `escape_runes.rs` | combat and supply policy | descriptors, predicates, arm sequencing |
| `gather_tools.rs` | gather-tool selection | policy over the posted rows |
| `dialog.rs`, `teleport.rs`, `trade.rs`, `partner_trade.rs`, `drive_partner_trade.rs` | dialog, teleport, and trade sequences | machine families |
| `death_recovery.rs` | death latch and recovery run | chat latch plus walk-back |
| `periodic_bank.rs` | periodic bank run | validate and execute |
| `walk.rs`, `walk_wait.rs`, `reach.rs`, `reach_entity.rs`, `scene_query.rs`, `inspect_wait.rs`, `task_clock.rs`, `watchdog.rs` | walk, wait, and reach helpers | tick waits and arrival |
| `line_of_sight.rs` | line-of-sight over the posted table | isolate-scene reads |
| `keep_list.rs` | combat keep-list composition | compatibility list |
| `modals.rs` | modal helpers | dialog page helpers |
| `quest_journal.rs`, `shop.rs`, `market_catalog.rs` | quest, shop, and market descriptors | preset and catalog tables |
| `loadout_plan.rs`, `loadouts_store.rs`, `settings_store.rs`, `params.rs` | loadout and settings stores | operator-facing descriptors |
| `registry.rs`, `rs2b0t_registry.rs`, `rs2b0t_registry/paths.rs`, `rs2b0t_registry/settings.rs`, `declared_abi.rs`, `module_imports.rs`, `identity.rs` | card registration and identity | import scan, paths, ABI names |
| `isolated_env.rs`, `js_cache.rs`, `memory_profile.rs`, `events.rs` | isolate env, cache, and diagnostics | support, no game actions |
| `load/mod.rs` | Load facade: shapes, cards, isolate | feature-gated library and isolate |
| `load/library.rs`, `load/library/catalog.rs` | JS card library and catalog apply | file store in root |
| `load/isolate.rs`, `load/isolate/thread.rs`, `load/isolate/teardown.rs` | V8 isolate thread and teardown | caller handle in root |
| `load/bindings.rs` | isolate bootstrap registration | register order and wrappers |
| `load/callback_v8.rs` | caller-callback invocation | one mechanism for typed helpers |
| `load/bank_tasks_v8.rs`, `load/boost_potions_v8.rs`, `load/clue_facts_v8.rs`, `load/clue_logic_v8.rs`, `load/clue_pack_v8.rs`, `load/combat_style_v8.rs`, `load/dialog_v8.rs`, `load/fire_v8.rs`, `load/gather_methods_v8.rs`, `load/hunt_v8.rs`, `load/loadout_v8.rs`, `load/machine_v8.rs`, `load/melee_weapons_v8.rs`, `load/partner_trade_v8.rs`, `load/quest_facts_v8.rs`, `load/scene_v8.rs`, `load/selected_facts_v8.rs`, `load/supply_v8.rs`, `load/targets_v8.rs`, `load/tools_v8.rs` | typed local V8 marshalling | one native call each, no machines |
| `load/buyout_plan.rs`, `load/distance.rs`, `load/reach_query.rs`, `load/shape.rs`, `load/line_of_sight.rs` | small typed V8 helpers | planning and geometry marshalling |
| `load/canvas_tape.rs` | canvas op tape | typed flush onto the recorder |
| `load/snapshot.rs` | snapshot-to-V8 materializer | typed settings and event rows |
| `load/paint_chrome.rs`, `load/paint_jive.rs` | paint payload builders | chrome and jive paints |
| `canvas/mod.rs` | canvas recorder | state and recording API |
| `canvas/geom.rs`, `canvas/raster.rs`, `canvas/style.rs`, `canvas/compose.rs` | canvas geometry, raster, style, composition | one slice each |
| `shim/mod.rs` | import-remap facade | re-exports, JS bytes untouched |
| `shim/content.rs`, `shim/paint.rs`, `shim/interact.rs`, `shim/modules.rs` | shim Rust producers | wire, paint, and content |
| `shim/*.js` | embedded JS sources | name maps, coercion, await plumbing only |
| `clue_tests.rs`, `slot_tests.rs`, `rs2b0t_registry_tests.rs`, `canvas/canvas_tests.rs`, `load/isolate_tests.rs` | test bodies | grouped, logical `<owner>::tests` |

### host-play

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | `Play` state and session lifecycle | unlock, spawn, login FIFO, tick pump |
| `main.rs` | `host-play` CLI | profile bind, vault unlock, run |
| `profile.rs` | launch-input resolution | world, vault, and script paths |
| `cache.rs` | runtime cache preparation order | progress stages, client owns mechanics |
| `catalog_core.rs` | shared full-core proof witness | headed and headless proof paths |
| `catalog_core_hunt.rs` | hunt proof witness | hold, retreat, spot, enter, leave, key, cell, bank |
| `catalog_core_ranging.rs` | ranging-guild proof witness | post-Start work only |
| `paired_core.rs` | paired full-cycle proof witness | Air, Mule, Flax, Duel |
| `script_runtime.rs` | per-slot observe, dispatch, and nav continuation | script wall and walk continuation |
| `route_inspect.rs` | inspect off-pump job | admission, calculation, publication |
| `login_readiness.rs` | login-readiness gate | welcome-modal settle before script work |
| `nav_identity.rs` | bundled nav identities | build-published table plus checked-in rows |
| `walk_map.rs` | shared WalkTo map model | catalogue, selection, guarded Walk/Teleport, live/script/manual routes |
| `bundled-nav-identities.json`, `known-cache-identities.json` | checked-in identity rows | data, not code |
| `external_loader.rs` | external loader smoke witness | proof infrastructure, disabled by default |
| `memory.rs` | opt-in memory harness | `BOT_MEMORY_N` only |
| `memory_diagnostics.rs` | memory-run diagnostics | bounded, never drains logs |
| `audio.rs` | focused-slot speaker gate | at most one speaker |
| `rss.rs` | process RSS and CPU sample | harness and resource card |
| `scatter.rs` | seed tiles for the wall | nav world, else Lumbridge |
| `progress.rs` | preparation progress values | small, copy-free |
| `lib_tests.rs` | test body | grouped, not an owner |

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
| `render_betty_views.rs` | fixed-orbit render captures | station times yaw and pitch |
| `scenarios/mod.rs` | scenario facade | re-exports families |
| `scenarios/production.rs` | production facade and shared vocabulary | seed and watch helpers |
| `scenarios/production/agility.rs`, `scenarios/production/alcher.rs`, `scenarios/production/ardy_thieving.rs`, `scenarios/production/bank_fletcher.rs`, `scenarios/production/chickens.rs`, `scenarios/production/climbing_boots.rs`, `scenarios/production/coal_trucks.rs`, `scenarios/production/cooking.rs`, `scenarios/production/dart_fletcher.rs`, `scenarios/production/door_opener.rs`, `scenarios/production/firemaking.rs`, `scenarios/production/flax.rs`, `scenarios/production/gem_cutter.rs`, `scenarios/production/gnome.rs`, `scenarios/production/herb_cleaner.rs`, `scenarios/production/herblore_secondaries.rs`, `scenarios/production/leather.rs`, `scenarios/production/potion_maker.rs`, `scenarios/production/runecrafting.rs`, `scenarios/production/smelting.rs`, `scenarios/production/smithing.rs`, `scenarios/production/superheater.rs`, `scenarios/production/tanner.rs`, `scenarios/production/thiever.rs`, `scenarios/production/vial_filler.rs` | production skill and script families | one family each |
| `scenarios/combat.rs` | combat facade and shared kit | witness builders, quest and equipment kit |
| `scenarios/combat/ardy_fighter.rs`, `scenarios/combat/auto_fighter.rs`, `scenarios/combat/chaos_druid.rs`, `scenarios/combat/fire_giant.rs`, `scenarios/combat/green_dragon.rs`, `scenarios/combat/hill_giant.rs`, `scenarios/combat/moss_giant.rs`, `scenarios/combat/rock_crab.rs` | combat target families | field and bank variants each |
| `scenarios/shop.rs`, `scenarios/shop/buyout.rs` | shop scenarios and buyout | teleport factories in root |
| `scenarios/pair.rs`, `scenarios/pair/travel.rs` | trade and companion scenarios | preparation and travel kit in child |
| `scenarios/navigation.rs` | nav-edge probes | probe helpers and closer-slot machinery |
| `scenarios/acquire_key.rs`, `scenarios/bank.rs`, `scenarios/cell.rs`, `scenarios/clue.rs`, `scenarios/enter_lair.rs`, `scenarios/fight_field.rs`, `scenarios/hold_spot.rs`, `scenarios/leave_lair.rs`, `scenarios/line_of_sight.rs`, `scenarios/prayer.rs`, `scenarios/ranging_guild.rs`, `scenarios/render.rs`, `scenarios/retreat_spot.rs`, `scenarios/route_inspect.rs`, `scenarios/script_basics.rs`, `scenarios/walk_spot.rs`, `scenarios/actor_observation.rs` | single-scenario probes | one probe each |
| `runner_tests.rs`, `scenario_tests.rs` | test bodies | grouped, not owners |

### frontend-core

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade | re-exports the session vocabulary |
| `session.rs` | operator lifecycle over `Play` | vault (staged view), play, selection, slot IO, Load/Log in/Log out/Remove, poll, script Start/Stop settlement, profile write settlement |
| `fleet.rs` | fleet membership | ordered members, logout latch, focus neighbour |
| `operations.rs` | operation results | ids, per-member outcomes, bounded book |
| `profiles.rs` | durable profile writes | one writer thread, ordered, last write per profile wins; results settle in `poll` |
| `surface.rs` | front-end slot adapter | `SlotSurface`, `HeadlessSurface` |
| `log.rs` | structured operator log | per-slot and process rings (500 each), redaction, filtered `LogView`, Save log… |
| `log_file.rs` | per-session log file | shared off-by-default preference, background writer, size rotation, session pruning |
| `session_tests.rs` | test bodies | real `Play` seam, no server |

### panel

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | declares every module below |
| `main.rs` | `panel-play` entrypoint | flag parsing, window loop |
| `app.rs` | panel shell and frame composition | docking shell on the owned window loop |
| `session.rs` | native adapter over `frontend_core::OperatorSession` | `PanelSurface` (slot IO, draw/audio policy), render `Focus` mirror, nav paint, harness state |
| `session/chooser.rs` | chooser credential scratch | edit buffers and vault helpers |
| `session_catalog.rs` | catalog discovery and warmup | loading and transpile warmup |
| `profile_script.rs` | per-profile script controls | assignment, start, reload, refresh |
| `window.rs` | GPU and winit shell | config, surface, errors |
| `game_view.rs` | game image texture | mailbox frames to texture |
| `input_capture.rs` | game input capture | keyboard queue and mouse streaming |
| `picker.rs` | WalkTo picker chrome | pan/zoom/plane/search; Walk/Teleport/selection via `host_play::walk_map` |
| `walk_map.rs` | app-owned WalkTo renderer | ≤24 terrain slots, one overlay, vector route, close lifecycle |
| `walk_map/overlay.rs` | viewport grid/collision overlay | one RGBA image, 768×512 / 1.5 MiB cap, never per-tile quads |
| `walk_map/fixtures.rs` | small PNG/POI fixtures | Lumbridge stand-in until image cache |
| `grid.rs` | MultiBox grid layout | cell geometry |
| `rail.rs` | sidecar rail chrome | geometry and status dot |
| `chrome.rs` | app chrome | menus and banners |
| `overlay.rs` | queue-card overlay | focused queue card over the image |
| `paint.rs` | script-paint overlay | structured paint over the chatbox rect |
| `queue_card.rs` | queue-card labels | pure label helpers, no ImGui |
| `focus.rs` | focus tracking | focused slot and pane |
| `clipboard.rs` | native clipboard backend | OS pasteboard bridge |
| `build_info.rs` | deploy fingerprint | release label and git stamp |
| `loadouts.rs` | loadout editor | equipment and supply CRUD |
| `script_picker.rs` | script browse picker | category order and badges |
| `nav_settings.rs` | nav settings pane | find opt-ins and pack selection |
| `live_harness.rs` | headed live and smoke watch | tick and capture qualification |
| `ui_state.rs` | persisted UI prefs | focused profile and collapsed maps |
| `theme.rs` | theme tokens | colors and metrics |
| `resource.rs` | resource formatters and sampler | pure formatters plus sampler |
| `wall.rs` | wall UI state | chooser, grid, render-all warning (membership is `frontend_core::Fleet`) |
| `srgb_present.rs` | present-path test helper | test-only |
| `app_tests.rs`, `session_tests.rs`, `input_capture_tests.rs`, `picker_tests.rs`, `walk_map_tests.rs` | test bodies | grouped, logical `<owner>::tests` |

### tui

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade and re-exports | second view of `Play`, no GPU |
| `main.rs` | `tui-play` entrypoint | flags and run modes |
| `bin.rs` | headless session and dispatch | `OperatorSession<()>` + headless surface; key actions onto the core |
| `app.rs` | view model and render root | polls statuses, routes keys and clicks |
| `map.rs` | WalkTo map widget | dots, route polyline, selection |
| `chat.rs` | chat and dialogue pane | chat ring, modal continue and answer |
| `status.rs` | status pane | focused slot rows plus guardian status |
| `script_params.rs` | script parameter editors | schema-driven editors |
| `script_shape.rs` | script pane widgets | browse, start, pause, stop, load |
| `loadouts.rs` | loadouts popup | worn gear and carry CRUD |
| `settings.rs` | settings popup | random toggles and nav opt-ins |

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
| `suite/child_tests.rs`, `suite/cli_tests.rs`, `suite/identity_tests.rs`, `suite/receipt_tests.rs` | test bodies | grouped, not owners |

## Runtime ownership

These are product boundaries. The crate checker does not prove them.

| Boundary | Owner | Not the owner |
| --- | --- | --- |
| Operator window, ImGui chrome, MultiBox, game blit, input into slots | `panel` | client applet UI, script isolate |
| Headless operator view (raster Off) | `tui` | GPU renderer, a second `Play` |
| Operator lifecycle: vault, fleet membership and latch, selected bot, Load/Log in/Log out/Remove intent, removal settlement, operation results | `frontend-core` | panel/tui chrome, a second login queue or script runtime |
| Slot workers: spawn/stop/reap, connection and readiness, login FIFO, tick pump, script start/pause/stop/load execution | `host-play` (`Play`) | panel/tui chrome, `frontend-core`, `e2e` |
| Native per-slot host APIs, snapshot/think, random-event guardian | `host` | nav internals, JS |
| Script kernel, isolate thread, shim coerce/marshal only | `script` | JS policy/routers, a foreign runtime |
| GPU 3D / CpuPix3D (`BOT_CPU=1`), packet/doAction Java shape | `client` | host bot-action API, 274bot crates in the client repo |

The JS shim must not grow its own API or shared policy. Compatibility maps
the JavaScript surface onto Rust host APIs.

## Behavioral invariants (review unless noted)

The checker does **not** pretend to prove the following. They remain
independent-review and test requirements unless a later, robust check exists.

- **Isolate ↔ host and game-action wire:** FlatBuffers only. No extra JS↔Rust
  host transport. Vault/settings/serde JSON elsewhere is unrelated and is
  not banned by word.
- **Typed synchronous Rust calculation helpers inside the isolate** are an
  authorized exception (operator 2026-09-18): bounded calculations with typed
  value marshalling. They are not permission for in-isolate game actions,
  extra host RPC, or JS policy.
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

Production splits that have landed are in the tables above: `api`, `e2e`,
`host`, `nav`, `scenario`, `script`. `host-play`, `panel`, and `tui` remain
one file per owner; their large files are still a layout concern, not a
second owner. This checker does not gate on sibling paths.

`e2e` and ignored live tests in `host-play` are wide orchestration. Linking
those crates for tests is not a claim that test code is the product owner of
nav, rendering, or the isolate.

## Changing a boundary

1. Edit [`tools/architecture/policy.toml`](../tools/architecture/policy.toml)
   with the new or removed `[[allow]]` and a reason.
2. Update the tables on this page so the public description matches.
3. Run the local commands above (including `--self-test`).
4. Get review. Do not land a graph change as a drive-by import.
