# Architecture

274bot is a Rust bot host over a vendored client library. Ownership is crate
and runtime boundaries, not a line-count budget and not a copy of a TypeScript
lint stack.

0.2.0 is Beta 1. Revision 289 is the production revision: the product is
built, tested and claimed for it. Revision 274 is best-effort: it may run,
but it is neither tested nor claimed.

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

GitHub Actions runs the same two commands in the `architecture` job, which
also installs the pinned `flatc` and diffs `crates/script/schema/generated`
against a fresh `flatc --rust` of `crates/script/schema/isolate.fbs`. That job
does **not** replace `fmt`, `clippy`, or `test`. Those gates stay as they are.

The explicit allowlist is [`tools/architecture/policy.toml`](../tools/architecture/policy.toml).
Every workspace member must appear there, and every edge the manifests
declare needs an `[[allow]]` with its kind. Unknown members and unlisted
edges fail closed. crates.io and other non-path crates are ignored. `client`
is a workspace *alias* to `vendor/fr-client-rust/crates/client`, not a 274bot
workspace member; it is listed as an architecture external.

## Production crate ownership

| Crate | Owns | Must not become |
| --- | --- | --- |
| `vault` | Encrypted profile store, secret strings, owner-only file helpers | A dependant of api/host/ui |
| `api` | Host-facing read/act/settle types mapped onto `client`; selected game facts | A dependant of host, nav, script, or UI |
| `host` | Native slot/tick APIs, login FIFO, guardian | A dependant of nav, script, host-play, or UI |
| `nav` | Packed world, router, Traveller, map data | Script isolate or operator UI |
| `script` | Native card machines (Quester, Gatherer, Combat, Sherlock), Load isolate, thin JS shim | A production dependant of `client` |
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
- `script` → `api`, `nav`, `vault` (runtime); `client` **dev-only**
- `host-play` → `api`, `client`, `host`, `script`, `vault`; `nav` runtime and **build**; `scenario` **optional** (and **dev**); `script` also **dev**
- `scenario` → `api`, `client`, `nav`, `script`
- `frontend-core` → `api`, `host`, `host-play`, `nav`, `script`, `vault`; `client`, `host-play`, `scenario` **dev-only**
- `panel` → `api`, `client`, `frontend-core`, `host`, `host-play`, `nav`, `scenario`, `script`, `vault`; `frontend-core` and `host-play` also **dev**
- `tui` → `api`, `client`, `frontend-core`, `host-play`, `nav`, `scenario`, `script`, `vault`; `host` **dev-only**; `frontend-core` and `host-play` also **dev**
- `e2e` → `api`, `client`, `host`, `host-play`, `nav`, `scenario`, `vault`

`e2e` does **not** Cargo-depend on `script`, `panel`, or `tui`. The suite
launches product binaries as child processes. That is orchestration, not a
claim that `e2e` owns those crates' production behavior.

### Truthful exceptions (enforced as written)

- **`api` → `client`** is intentional. `api` maps host types onto the
  vendored client. It is not a reverse `client` → `api` edge.
- **`script` → `nav`** is a runtime edge: native machines read the packed
  world, routes and zones through typed `nav` values. `script` → `client` is
  `[dev-dependencies]` only; promoting it to a normal/optional/target
  dependency fails the checker, so the compiled script crate stays
  client-free.
- **`tui` → `host`** is `[dev-dependencies]` only. Production TUI composes
  through `frontend-core` and `host-play`.
- **`frontend-core` → `host`** carries only the `SlotInput`/`FrameBuf`
  handles a surface passes to `Play::try_spawn_slot`.
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

If a future change needs a new edge or kind, edit the policy and this page
and get review. Do not add a second hidden graph.

## Cargo features

| Crate | Features |
| --- | --- |
| `script` | `load` (default): the V8 isolate, JS library and the load-only clue and journal machines; `path-schema`: Path JSON Schema generation; `memory-profile`; `test-hooks` |
| `api` | `path-schema`, `debug-catalog`, `test-hooks` |
| `host` | `render-diagnostics`, `performance-profile`, `journal-paint-proof` |
| `host-play` | `live-harness` (pulls in `scenario`), `memory-profile` and `memory-profile-no-alloc` (the fleet memory harness), `test-support`, `live-probe`, `debug-catalog`, `journal-paint-proof` |
| `panel` | `render-diagnostics`, `memory-profile`, `memory-profile-no-alloc` |
| `tui` | `memory-profile`, `memory-profile-no-alloc` |
| `frontend-core` | `test-support` (arm-only spawn fixtures for front-end tests) |

## Per-crate module ownership

Paths are relative to `crates/<crate>/src`. Rows name a module or a family of
modules; a row with a glob (`catalog_*.rs`) covers every file it matches.
`*_tests.rs`, `tests.rs` and `*/tests.rs` files are test bodies, not
production owners, and are not listed one by one. A directory with its own
facade file (`foo.rs` beside `foo/`) splits that module's reasons to change.

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
| `slot.rs` | per-slot drain pump | gens diff, tick synthesis, dirty families |
| `slot_io.rs` | frame and input channels | mailbox, input, park and wake |
| `login_queue.rs` | login FIFO and backoff | production rate limits |
| `auto_run.rs` | auto-run toggle | run-energy threshold |
| `performance_profile.rs` | per-slot host work timers | opt-in, `performance-profile` feature |
| `random.rs` | random detection and shared vocabulary | stateless detect, caller-owned cooldowns |
| `random/guardian.rs` | act, hold, and dialog state machine | trap holds, cooldown writes, knock edge |
| `random/maze.rs` | maze graph, route, and phase | port of the frozen maze logic |
| `random/maze/layout.rs` | static maze layout data | copied layout rows |

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
| `map/formats.rs` | checked map payload codecs | client catalogue, image manifest, shipped-terrain description and data-only `274P`/1 navpois |
| `map/cache.rs` | publication readers and checkpoints | typed partial/ready entries; bounded reads and requested-payload verification |
| `map/records.rs` | offset-only maps.bin access | header index plus one reusable record buffer, not an archive/world copy |
| `map/client_cache.rs` | bounded client-cache decoding | seek index shared by the terrain and catalogue producers |
| `map/catalogue.rs` | client-only POI derivation | isolated from terrain policy so classifier changes do not invalidate baked imagery |
| `map/raster.rs` | terrain raster and PNG pyramid | deterministic, sparse, bounded mapsquare neighbourhoods |
| `map/producer.rs` | pure producer entry points | host owns leases, locking, cancellation and publication |
| `map/services.rs` | nav-bake producer of the `274bot.navpois` sidecar | jm2 NPC rows, bank-service handlers, labels |
| `named_banks.rs` | selected-world bank stand resolution | captured account eligibility stays with the caller, no `script` dep |
| `pack.rs` | pack wire codec and bake hub | pack encode and decode, stable exports |
| `pack/config_parse.rs` | pack config parsers | content config text |
| `pack/mapsquare.rs` | mapsquare parse and geometry | mapsquare bake rows |
| `pack/banks.rs` | bank table codec and derivation | booth tiles and stands |
| `pack/sidecars.rs` | sidecar formats and codecs | flags, paint-reach, canlight |
| `pack/zones.rs` | zone table codec | wire geometry and stable identity of exclusion zones |
| `paint.rs` | nav-paint buffers | no imgui, no client draw |
| `quest_gates.rs` | quest-stage gates on transport edges | typed gates over selected progress signals |
| `router.rs` | world-space Dijkstra | total-ticks cost, gated edges |
| `router/grid.rs` | legacy grid A-star | `find_on_grid` for traveller and harness |
| `router/resources.rs` | resource labels on walk nodes | change only at paid hops |
| `tile.rs` | tile coords and distance | x, z, level |
| `transport.rs` | transport graph orchestration | types, ordering, shared cost and geometry |
| `transport/condparse.rs` | shared condition text helpers | fail-closed, same behavior |
| `transport/index.rs`, `transport/script_text.rs`, `transport/rs2_syntax.rs` | shared placement readers, script text and a small RuneScript statement parser | producers, consumed by family derivers |
| `transport/observable.rs` | quest-journal observability proof | green-row rule |
| `transport/gates.rs`, `transport/quest_doors.rs`, `transport/doors.rs`, `transport/door_members.rs`, `transport/scripted_doors.rs`, `transport/stage_doors.rs`, `transport/stage_doors_forced.rs`, `transport/brass_key.rs`, `transport/membergate.rs`, `transport/members_guard.rs`, `transport/webs.rs`, `transport/vertical.rs`, `transport/shortcuts.rs` | door, gate, vertical, and shortcut families | one family each; stage doors evaluate the engine's door openers |
| `transport/engine_door_procs.rs2`, `transport/engine_forced_procs.rs2` | engine opener procedures | content text read by the stage-door deriver |
| `transport/static_routes.rs`, `transport/npc_hops.rs`, `transport/gliders.rs`, `transport/spirit_trees.rs`, `transport/levers.rs`, `transport/toll.rs`, `transport/magic_guild.rs`, `transport/ranging_guild.rs`, `transport/zanaris.rs`, `transport/teleports.rs`, `transport/wilderness.rs` | fixed-route, guild, teleport, and wilderness-legality families | one family each |
| `traveller.rs` | follow facade and scheduler | `FollowRun` start and step, shared state |
| `traveller/snapshot.rs` | snapshot-query helpers | scene reads for hops |
| `traveller/dialog.rs` | dialog and teleport-send helpers | chat and teleport sends |
| `traveller/walk.rs` | walk-hop execution and reporting | walk targeting |
| `traveller/transport_hop.rs` | transport-hop execution | approach and door execution |
| `traveller/npc_hop.rs` | NPC-backed Boat/Npc/Glider approach and recovery | live scene operability, reachable-cost selection, bounded retry |
| `traveller/legacy_grid.rs` | legacy-grid execution | via the stable grid API |
| `walk_destinations.rs` | shared walk-destination pins | town tiles for WalkTo confirm |
| `world.rs` | bound world handle | packed collision plus transport graph |
| `world_state.rs` | search gating facts | fail-closed edge requirements |
| `zones.rs`, `zones/curated.rs` | exclusion zones, their spatial index and the per-search exemption filter | content-derived; curated groups and hazards in the child |
| `bin/nav-pack.rs` | `nav-pack` CLI | whole-world bake entrypoint |
| `bin/nav-input-audit.rs` | input audit CLI | offline cache and content audit |
| `bin/cache-content-id.rs` | `cache-content-id` CLI | read-only identity export |
| `required-content-274.tsv`, `required-content-289.tsv` | canonical content inventory | build-checked data, not code |

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
the PNG decoder still owns full chunk/CRC validation.

Only `PartialEntry` reads `.<key>.partial` checkpoints. Ready readers require a
canonical complete directory and validated manifest; ordinary image open does
not hash/decode the pyramid. Resume verifies completed units and their exact
policy identity. The cache-job owner (`host-play`) owns the prepared-cache
`Arc`, locks, atomic publication, cancellation and quotas.

Release packages ship the 289 terrain: release packaging bakes the nav
bundle's pinned client cache through the production map-cache path and ships
the published image directory as `map/<revision>/images/<key>/` under the
install resource root (beside `nav/`), described by
`map/<revision>/274bot.mapimages.json` (`ShippedImages`: image identity, key,
the image manifest's receipt, tile totals). The cache job installs it only for
exactly the bound client cache's image identity: every tile's length and
SHA-256 is checked against the pinned manifest as it is copied into the key's
`.partial` directory, recorded by receipt and published by the same writer and
rename as a local bake, so readers never see a second loader. A published key
is never replaced, and shipped files are only read. Foreign shipped terrain is
ignored; damaged or partial shipped files fail closed (the local bake stays
behind the operator's consent and adopts the tiles already verified).

### script

| module | owns (one reason to change) | notes |
| --- | --- | --- |
| `lib.rs` | crate facade: `ScriptCtx`, `SlotScript`, registry descriptors | `load` feature gates the isolate |
| `ctx.rs` | host-owned per-tick input and shared walk options | compiled cards receive `native::NativeTick`, not this driver |
| `native.rs`, `registry.rs` | typed card/configuration/output contract and one descriptor registry | metadata/preparer/factory, no duplicate defaults; cards are `Gatherer`, `Quester` and (with `load`) `Sherlock` |
| `native/actions.rs`, `native/ledger.rs`, `native/owner.rs`, `native/walk.rs`, `native/walk_wait.rs` | host action ledger and native walk correlation | receipts, revocation identities, route terminal versus arrival |
| `native/death.rs`, `native/thieve.rs`, `native/thieving_core.rs` | shared death latch; native thieving machine and its core | content-derived respawn square |
| `native_bank.rs`, `native_equipment.rs`, `native_production.rs`, `native_shop.rs`, `trade_screen.rs`, `bank.rs`, `bank/ops.rs`, `bank/npc.rs` | snapshot-driven cores shared by compiled cards and compat families | one bank transfer kernel, wear/remove, production, shop purchase and player-trade screen |
| `slot.rs` | per-uid runner lifecycle | intent and presence gates, tick edge |
| `slot/compiled.rs` | off-pump compiled preparation, fenced install/configure and shared output | lazy per-active-card state; compiled XOR Load |
| `slot/api.rs`, `slot/progress.rs` | API-owned Gatherer seat; one-attempt progress policy | registration and authority stay native |
| `slot/pending.rs` | bank settlement records | host-owned pending accessors |
| `machine.rs` | step-machine host | one multi-tick behavior in Rust |
| `quester/` | Quester card, Path types and compiler, runner, queue, pair and gang, eligibility, provisioning, watchdog, quest handlers and step families | `card.rs` card; `path.rs`, `compile.rs`, `schema.rs` Path values, lazy compile and JSON Schema; `runner.rs` colour-first tick loop with journal evidence; `registry.rs` bundled documents plus optional folder; `families/` one step family each (combat, dialogue, gather, partner, reach, thieve, ...) |
| `gatherer/` | Gatherer card and runner | woodcutting, mining and fishing from live observation: `select.rs` target choice, `gather.rs`, `drop.rs` power drops, `supply.rs`, `recover.rs` death recovery, `area.rs`, `widen.rs`, `settings.rs`, `status.rs`, `oneop.rs` |
| `combat/` | shared observed-state combat machine | melee, ranged and magic on one planner (`machine.rs`, `style/`); `policy.rs`, `prayer.rs`, `guard.rs` WalkGuard, `threats.rs`, `schedule.rs`, `select.rs`, `tables.rs`, `request.rs` |
| `combat/risk.rs`, `combat/risk/{facts,geometry,input,replay}.rs` | pure conservative route assessment | shared combat facts and food picker; complete route geometry; persistent poison input; no admission, route-worker or runtime escape wiring |
| `api_gather.rs`, `api_progress.rs`, `api_session.rs` | JS API v2 session wire and output types | Gatherer sessions and quest progress for Load scripts |
| `observed.rs` | per-isolate decoded scene | delta-merge, owned rows |
| `isolate_fb.rs` | isolate wire codec | flatc-generated bindings (`schema/generated`) plus delta/`IsolateBuf` domain layer; one verified root per message |
| `host_js.rs` | generated Host JS types | from verb tables, not rs2b0t names |
| `compat_dts.rs` | generated compat declarations | JS API v1 `.d.ts` from the frozen typed source |
| `channel.rs` | bounded structured-clone subset for the BroadcastChannel broker | bytes over the FlatBuffer wire |
| `content.rs` | curated sites and hostile predicate | bank aliases posted via the shim payload |
| `ent.rs` | Ent lookup over caller rows | no scene scan |
| `bank_access.rs`, `bank_deposit.rs`, `bank_op.rs`, `bank_open.rs`, `bank_withdraw.rs`, `bank_select.rs`, `banking_open.rs` | bank open, op, deposit, withdraw and select steps | machine families, shared pieces |
| `clue.rs`, `clue/isolate.rs` | native recovery declaration; load-only clue session machine | isolate token, clock, identify, verbs stay private to the crate |
| `clue/scene.rs`, `clue/puzzle.rs`, `clue/entrana.rs`, `clue/acquire.rs`, `clue/shop.rs`, `clue/talk.rs`, `clue/combat.rs`, `clue/verbs.rs`, `clue/family.rs` | clue phase helpers | one phase slice each |
| `sherlock.rs` | Sherlock card wiring | marshals the frame, maps verbs, no second solver |
| `duel.rs` | clue Duel Arena protocol | partner matching, rules, timing, travel legs |
| `hunt.rs` | hunt framework on the machine host | sessions and runs, child nesting |
| `hunt/wait.rs`, `hunt/teleport_out.rs`, `hunt/acquire.rs` | hunt family slices | three distinct families |
| `hunt_bank.rs`, `hunt_bank/plan.rs` | hunt bank runtime and planning | stages in root, plan projection in child |
| `hunt_fight.rs` | fight aggregate and shared projection | policy, clocks, field pick |
| `hunt_fight/hold.rs`, `hunt_fight/retreat.rs`, `hunt_fight/walk_spot.rs` | fight auxiliary machines | one machine plus its Kind each |
| `hunt_cell.rs`, `hunt_key.rs`, `hunt_lair.rs`, `hunt_leave.rs` | single-phase hunt machines | one phase chain each |
| `hunt_catalog.rs` | hunt tables and small rules | coordinates and pure helpers, no sends |
| `cake_stall.rs`, `cake_stall/runtime.rs`, `cook_locations.rs` | stall selection and steal loop; cook-location resolution | facts in `api`, sequence here |
| `production.rs` | chat-make sequencing | Make-X and panel family |
| `fire.rs` | fire lighting and lane ranking | frozen light loop |
| `food_policy.rs`, `supply_v2.rs`, `boost_potions.rs`, `prayer.rs`, `autocast.rs`, `ranged.rs`, `special.rs`, `melee_weapons.rs`, `escape_runes.rs`, `attack_clock.rs`, `fight_upkeep.rs` | combat and supply policy | descriptors, predicates, arm sequencing |
| `gather_tools.rs` | gather-tool selection | policy over the posted rows |
| `dialog.rs`, `dialogue_outcome.rs`, `teleport.rs`, `trade.rs`, `partner_trade.rs`, `drive_partner_trade.rs` | dialog, teleport, and trade sequences | machine families |
| `death_recovery.rs` | death latch and recovery run | chat latch plus walk-back |
| `periodic_bank.rs` | periodic bank run | validate and execute |
| `walk.rs`, `walk_wait.rs`, `reach.rs`, `reach_entity.rs`, `scene_query.rs`, `inspect_wait.rs`, `task_clock.rs`, `watchdog.rs`, `anchor_return.rs`, `boat_fare.rs`, `event_signal.rs` | walk, wait, and reach helpers | tick waits, arrival, boat-fare recovery, return-to-anchor |
| `line_of_sight.rs` | line-of-sight over the posted table | isolate-scene reads |
| `keep_list.rs` | combat keep-list composition | compatibility list |
| `modals.rs` | modal helpers | dialog page helpers |
| `quest_journal.rs`, `quest_journal/native.rs`, `quest_journal/isolate.rs` | no-load native journal contract and native journal click machine; load-only journal machine | one click, validate, capture, one close |
| `shop.rs`, `market_catalog.rs` | shop and market descriptors | preset and catalog tables |
| `loadout_plan.rs`, `loadouts_store.rs`, `settings_store.rs` | loadout and settings stores | operator-facing descriptors |
| `rs2b0t_registry.rs`, `rs2b0t_registry/paths.rs`, `rs2b0t_registry/settings.rs`, `declared_abi.rs`, `module_imports.rs`, `identity.rs` | catalog card registration and identity | import scan, paths, ABI names |
| `isolated_env.rs`, `js_cache.rs`, `memory_profile.rs`, `events.rs` | isolate env, cache, and diagnostics | support, no game actions |
| `load/mod.rs` | Load facade: shapes, cards, isolate | feature-gated library and isolate |
| `load/library.rs`, `load/library/catalog.rs` | JS card library and catalog apply | file store in root |
| `load/isolate.rs`, `load/isolate/thread.rs`, `load/isolate/teardown.rs` | V8 isolate thread and teardown | caller handle in root |
| `load/bindings.rs` | isolate bootstrap registration | register order and wrappers |
| `load/callback_v8.rs` | caller-callback invocation | one mechanism for typed helpers |
| `load/*_v8.rs` | typed local V8 marshalling | one native call each, no machines |
| `load/buyout_plan.rs`, `load/distance.rs`, `load/reach_query.rs`, `load/shape.rs`, `load/line_of_sight.rs`, `load/wait_clock.rs` | small typed V8 helpers | planning, geometry marshalling, paused-time wait clock |
| `load/canvas_tape.rs` | canvas op tape | typed flush onto the recorder |
| `load/snapshot.rs` | snapshot-to-V8 materializer | typed settings and event rows |
| `load/paint_chrome.rs`, `load/paint_jive.rs` | paint payload builders | chrome and jive paints |
| `canvas/mod.rs` | canvas recorder | state and recording API |
| `canvas/geom.rs`, `canvas/raster.rs`, `canvas/style.rs`, `canvas/compose.rs` | canvas geometry, raster, style, composition | one slice each |
| `shim/mod.rs` | import-remap facade | re-exports, JS bytes untouched |
| `shim/content.rs`, `shim/paint.rs`, `shim/interact.rs`, `shim/modules.rs` | shim Rust producers | wire, paint, and content |
| `shim/*.js` | embedded JS sources | name maps, coercion, await plumbing only |

The crate root holds the data the modules embed: `paths/289/` (the built-in
Quester Path documents) with `paths/path.schema.json`, `schema/isolate.fbs`
with its checked-in `schema/generated` bindings, `compat-js/` (the JS API v1
name maps and generator), `host-js/index.d.ts` (the generated JS API v2
declarations) and `examples/`.

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
| `script_observe.rs`, `script_snapshot.rs`, `script_interact.rs`, `script_nav.rs`, `script_walk.rs`, `script_bank.rs`, `script_paint.rs`, `script_channels.rs` | per-slot script pump pieces | observation and snapshot posting, interact dispatch, route worker and walk continuation, WalkGuard, bank selection, paint status, the BroadcastChannel broker |
| `walk_arm.rs`, `walk_plan.rs`, `walk_permissions.rs` | operator walk arms and permissions | queued follow and bank-fetch steps, session routes, permission composition at admissions |
| `route_inspect.rs` | inspect off-pump job | admission, calculation, publication |
| `login_readiness.rs` | login-readiness gate | welcome-modal settle before script work |
| `nav_identity.rs` | bundled nav identities | build-published table plus checked-in rows |
| `walk_map.rs`, `walk_map/{actions,catalogue,observations,routes}.rs` | shared WalkTo map model | catalogue, selection, guarded Walk/Teleport, live/script/manual routes |
| `map_cache.rs`, `map_cache/shipped.rs`, `map_producer.rs` | WalkTo map cache lifecycle | demand, bake/publication, installing release-shipped terrain that matches the bound image identity; the native producer |
| `map_bind.rs` | process-wide map demand | manager at first open (shipped terrain under the install root's `map/`), packaging bake |
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
| `fleet.rs` | fleet membership | ordered members, logout latch, focus neighbour |
| `selection.rs`, `marked.rs`, `bulk.rs` | marked-bot selection and commands | marks keyed by profile identity; Assign, Assign & restart, log in/out on marked rows; one outcome per marked row |
| `group_walk.rs` | group walk | one WalkTo destination for every marked bot; panel picker and TUI map both call it |
| `walk_permissions.rs`, `nav_prefs.rs`, `map_bake.rs` | durable walk and nav preferences; terrain-bake consent | shared by both front ends; read fail closed |
| `operations.rs` | operation results | ids, per-member outcomes, bounded book, outcome journal the poll logs as `op#<id>` lines |
| `views.rs`, `progress.rs` | fleet and detail projections; run-progress wording | one row per member (phase, queue place, login-error retry wait vs operator hold, script state, walking, retained error, newest operation, narrow label), fleet counts, and the selected slot's detail; rows re-derived only when their facts change; runtime, time since progress and levels gained |
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
| `scripts/sync.rs` | Apply to all (bulk parameter sync) | frozen same-card scope, per-profile writes, generation-fenced live push, separate persistence/live counts; confirmation holds new Starts, and accepted native copies retain that hold through preparation and the writer's durable completion (JS copies release after staging) |
| `quester_paths.rs` | durable Quester Path configuration | asynchronous folder reload |

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
| `walk_map.rs`, `walk_map/overlay.rs`, `walk_map/labels.rs`, `walk_map/fixtures.rs` | app-owned WalkTo renderer | ≤24 terrain slots, one overlay (768×512 / 1.5 MiB cap, never per-tile quads), label ranking, small PNG/POI fixtures |
| `nav_paint_cache.rs` | nav-paint cache | per-frame reuse of path paint |
| `grid.rs` | MultiBox grid layout | cell geometry |
| `fleet.rs`, `fleet_actions.rs`, `fleet_columns.rs` | Fleet window | identity-keyed marks, action bar over marked rows, toggleable status columns; the panel adapter over the shared marked-bot commands |
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
| `fleet.rs` | fleet table | cursor, row selection, filter, visible-window rows over the core's fleet rows |
| `overlay.rs` | app overlays | help, palette, confirmations with frozen targets, context menus, message viewer, manual walk |
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

The 0.2.0 headline scripts are compiled into the host. Quester and Gatherer
drive host verbs directly, with no isolate; they hold per-slot state in
bounded structs and read the snapshot Rust already holds.

| Machine | Card | What it does |
| --- | --- | --- |
| Quester (`script/src/quester/`) | `Quester` | Runs built-in quest Paths for revision 289: the Path compiler turns one authored quest document into stage-gated steps, the runner executes them on host verbs (bank, shop, production, gather and combat adapters), and progress resolves from journal evidence. Paths are documented in [quester-paths.md](quester-paths.md). |
| Gatherer (`script/src/gatherer/`) | `Gatherer` | Woodcutting, mining and fishing from live observation: held-tool loop, disposal including power-drop batching, bank trips and provisioning, over methods from the selected content facts. |
| Combat (`script/src/combat/`) | shared machine, no card | One observed-state planner for melee, ranged and magic: prayers, sips and eating. Quester combat steps drive it through the typed Path adapter. |
| Sherlock (`script/src/sherlock.rs` on `script/src/clue.rs` plus `script/src/clue/`) | `Sherlock` | Runs clue trails on the one clue step machine, one phase slice each, on the slot's pump thread; the clue machine is compiled with the `load` feature. |

## Compatibility shim boundary

Catalog scripts written for the frozen catalog API run unmodified through
the compat isolate (JS API v1). The shim maps those names onto host verbs,
step machines or `not impl`: name maps, value coercion and await plumbing
only. Loops, tables, decisions, retries and sequencing live in Rust step
machines that read the snapshot Rust already holds. Scripts written for the
host API run through the same isolate as JS API v2 ([`api/js-api-v2.md`](api/js-api-v2.md)).

The only JS↔Rust wire is the FlatBuffer: `IsolateBuf` posts snapshots into
each isolate and decodes the shim's interact batches back. Bounded
synchronous Rust calculations with typed value marshalling are the one
authorized exception; they compute and never send a game action.

## Navigation pack and server profiles

The nav pack is format v16 (magic `274V`). Decoding rejects v15 and older as
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

These are product boundaries. The crate checker does not prove them.

| Boundary | Owner | Not the owner |
| --- | --- | --- |
| Operator window, ImGui chrome, MultiBox, game blit, input into slots | `panel` | client applet UI, script isolate |
| Headless operator view (raster Off) | `tui` | GPU renderer, a second `Play` |
| Operator lifecycle: vault, fleet membership and latch, selected bot, marked-bot commands, Load/Log in/Log out/Remove intent, removal settlement, operation results, script assignment/parameters/reload/bulk coordination; fleet/detail projections and the process resource meter | `frontend-core` | panel/tui chrome, a second login queue or script runtime, per-row samplers |
| Slot workers: spawn/stop/reap, connection and readiness, login FIFO, tick pump, script start/pause/stop/load execution | `host-play` (`Play`) | panel/tui chrome, `frontend-core`, `e2e` |
| Native per-slot host APIs, snapshot/think, random-event guardian | `host` | nav internals, JS |
| Script kernel: native card machines, isolate thread; shim coerce/marshal only | `script` | JS policy/routers, a foreign runtime |
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

## Changing a boundary

1. Edit [`tools/architecture/policy.toml`](../tools/architecture/policy.toml)
   with the new or removed `[[allow]]` and a reason.
2. Update the tables on this page so the public description matches.
3. Run the local commands above (including `--self-test`).
4. Get review. Do not land a graph change as a drive-by import.
