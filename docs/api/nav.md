# Nav: whole-world routing, transports, and WalkTo

`crates/nav` is the single nav stack: a whole-world collision bake, a
content-derived transport graph, a Dijkstra router, and a pollable route
follower. It acts only through the kernel API (`api::interact`,
`api::settle`) and never deep-copies the world.

## Pack bake

Application builds bake the pack themselves. `crates/host-play/build.rs` runs
the shared baker (`nav::bake::bake_world`) for the selected release revision,
stages pack + flags + sidecar manifest + stamp under the install resource root
(`nav/<revision>/` — next to the built binary, `Contents/Resources` in a macOS
bundle, `BOT_NAV_RESOURCE_DIR` when a packager stages elsewhere) and publishes
the identity that the compile-time table `host_play::bundled_nav_identities()`
serves. Normal WalkTo in that build needs no manual step. The default revision
is **289**; `BOT_NAV_REVISION=274` builds the 274 artifact instead.

`nav-pack` remains for deliberate developer/custom-input bakes. It calls the
same baker and reads every Server mapsquare jm2 plus the door/loc/rs2 scripts
and writes the current whole-world pack (`encode` in
`crates/nav/src/pack.rs`):

```bash
cargo run -p nav --bin nav-pack [MAPS_DIR] [DOORS_DIR] [CONFIG_DIR]
```

Defaults are `$HOME/experiments/Server/content/maps` and siblings. Pass
the three dirs if yours lives elsewhere. Output goes to `$NAV_PACK` or
`~/.274bot/274bot.navpack`. `gates.loc` is derived from the maps dir's
parent (`content/scripts/general_use/configs/gates.loc`).

The pack serializes the whole-world `WorldCollision` (four planes, packed
9-bit walk per tile: `u8` face + `SQ_BLOCKED`, row-major z-then-x) plus
the derived `TransportGraph`. Magic `b"274V"`, version byte **8** (v8
appends the content-derived bank-stand table after the edges; raw `u32`
flags are not on the v8 wire, the optional `274F` sidecar holds them for
collision paint). `decode` accepts version 8 only — v7 and older are
`BadVersion`. The `274N` grid decoder (`decode_grid`) stays for old
boolean-walk files.

### Build-time selection, reuse and overrides

| Env | Default | Meaning |
| --- | --- | --- |
| `BOT_NAV_BUILD` | `require` | `skip`: no artifact baked/staged, checked-in identities only |
| `BOT_NAV_REVISION` | `289` | selected release revision (`274` supported) |
| `BOT_NAV_ENGINE_DIR`, `ENGINE_DIR` | revision's canonical engine | bake input root |
| `BOT_NAV_CONTENT_DIR` | `<engine>/../content` | canonical content tree |
| `BOT_CACHE_MANIFEST` | checked-in known cache identities | verified cache manifest |
| `BOT_NAV_RESOURCE_DIR` | cargo profile dir / bundle Resources | staging root |

Missing canonical inputs fail the build instead of shipping an app without nav,
and the guard names the input class: besides `maps/`, the door configs,
`gates.loc`, the loc `config` jag and the cache archives, the build requires
what the bake consumes implicitly — the `pack/loc.pack`, `pack/obj.pack` and
`pack/varp.pack` id tables, the files read by name (`scripts/ladders+stairs/`,
`area_gnome`'s spirit tree, `area_ardougne_east`'s levers,
`area_alkharid/configs/border_gate.loc`, `quest_zanaris`,
`skill_magic/configs/{magic_spells.dbrow,enchanted_jewelry.obj}`,
`interface_bank/configs/bank_booth.loc`), every file of the recursive script
scans (`scripts/**/*.rs2` door open scripts, agility shortcuts and jewellery
rubs, `scripts/**/*.constant`, `scripts/**/*.obj` blades, and the `*.loc` door
configs under `scripts/doors/configs`, `scripts/quests`, `scripts/areas` and
`scripts/general_use/configs`) and every mapsquare of the revision's canonical
map set. The scans are listed one file per row, not anchored by their
directories: removing a single consumed child (`shortcuts.rs2`, a jewellery
`*.rs2`, a `.loc` door config) fails the build even when its directory keeps
its siblings. That inventory is revision-owned data
(`crates/nav/src/required-content-{289,274}.tsv`, embedded in `nav::bundle` and
verified against the canonical trees — the `bundle::` tests compare the scan
classes and the map set to the actual trees; refresh the TSV when the canonical
content legitimately changes) and it is checked on every default build —
including one that would otherwise reuse a warm staged artifact set. It judges
presence only: a deliberate content edit or addition is not rejected, it flows
through the ordinary input fingerprints and rebakes. Files the bake never reads
are out of scope, and the baker stays tolerant — a content file added to a scan
class is baked, an inventory row is the file membership the canonical tree had
when it was listed. `BOT_NAV_BUILD=skip` and the custom-input `nav-pack` CLI
keep their behavior.

The cache identity is verified at build time exactly like the runtime verifies
it: a supplied manifest must match the cache bytes and the selected jag,
otherwise the captured identity must be one of the checked-in
`crates/host-play/src/known-cache-identities.json` rows.

Warm builds reuse unchanged artifacts: the staged `nav-build.json` stamp
records the cache identity, the pack/flags digests, the generator identity
(the manual id plus the bytes of `bake.rs`/`collision.rs`/`pack.rs`/
`transport.rs`) and a fingerprint (size + mtime) of every canonical input
(content tree, config jag, cache archives). Any change to those inputs, to the
pack format identity (`nav::pack::FORMAT_ID`), to the generator, to the cache
identity, or a missing/replaced staged artifact rebakes; nothing else re-hashes
the world at build or runtime. The bundled fast path keeps its cheap
revision/cache-identity check and reads + decodes the staged pack once
(`NavLoadCounters`), while `--nav-pack` / `NAV_PACK` / `--nav-flags` overrides
keep the external path and a differing cache identity falls back to it.

Real-artifact check (needs a default application build in this target profile
plus the canonical cache):
`cargo test -p host-play --test bundled_nav_build -- --ignored --nocapture`.

## Collision (`nav::collision`)

`WorldCollision { origin, width, height, walk: Vec<u8>, blocked: Vec<u64>, flags: Option<Vec<u32>> }` bakes every
mapsquare's `MAP fN` land flags and `LOC` placements into the client's
`CollisionFlag` bitmasks, mirroring the client's `CollisionMap`
`add_wall`/`add_loc` stamping (walls → per-direction `W_*` faces,
centrepiece footprints → `WALK_SCENERY`, active blockwalk ground decor →
`WR_GRND`, doors blocked-when-closed). `walkable(t)` is the blanket
standable check (`WALK_BLOCK_FLAGS | WALK_SCENERY | WR_GRND == 0`); the
**router does not use it** — it uses directional `PL_WALK_*` edge tests
(see below).

## Transports (`nav::transport`)

`TransportGraph { edges: Vec<TransportEdge>, from: HashMap<WorldTile, Vec<usize>> }`.
`derive_transports(content_root)` parses the 2004 content: `doors/*.loc`
(openable doors), `gates.loc` fence/gate hops, `ladders+stairs/` and
`areas/` rs2 scripts (`p_telejump`/`p_teleport`/`~climb_ladder`,
`movecoord` landings), `skill_magic/` teleports, `skill_agility/`
shortcuts, spirit trees, Shilo↔Brimhaven cart NPC hops, Ardougne
wilderness levers, Al Kharid toll / Shantay **both ways**, essence-mine
wizard **entry and EssenceSession return**, Elkoy maze escorts, Zanaris
shed door with worn Dramen req, slashable webs (knife `oplocu` or worn
slash blade), gnome gliders, and boat NPC + gangplank. A `TransportEdge`
carries `kind` (Door/Ladder/Stairs/Boat/Teleport/AgilityShortcut/Glider/
SpiritTree/Npc), `from`/`to`, `loc_id`, the 1-based menu `option`
(`0` = use first `item_req` on the loc), `ticks`, and requirement
vectors including `worn_req` (**any-of**). Spell teleports have no fixed
origin: they live on `TransportGraph::teleports` and stay out of Dijkstra
unless `FindOptions::allow_teleports`. Wilderness tiles stay out unless
`FindOptions::allow_wilderness`. Both default **off**. `find` also
fail-closes on live `WorldState`.

## Router (`nav::router`)

`find(collision, graph, from, to) -> Result<Route, RouteError>` is Dijkstra
with safe defaults (no wilderness, no any-tile teleports).
`find_with(..., FindOptions { allow_teleports, allow_wilderness })` opts
those in. Tile steps use the client's directional `PL_WALK_*` masks,
**not** the blanket `walkable()`. Transport take-off is any standable
tile within **`INTERACT_RADIUS` 1** of the edge `at` (adjacent only — a
radius of 3 let cow-pen routes “use” the north-west road gate through a
fence). `Route { legs, dest, ticks }`; `Leg::Walk { tiles }` runs
collapse, `Leg::Transport { edge }` is one per transport. `RouteError` is
`NoPath` or `BudgetExhausted` (a node-expansion cap). `find` is CPU-heavy;
run it off-pump (a short-lived worker) and arm the result.

`Traveller::follow` walks loc hops and fires packed OP_NPC, boats,
gliders, webs, EssenceSession, Shantay, and teles. NPC-backed hops use
the live NPC tile (search radius 8). Glider landings settle Chebyshev 1.
Agility waits packed `edge.ticks` after land.

## Traveller (`nav::traveller`)

`Traveller::follow(client, snapshot, route, &mut options)` is **pollable**:
call it once per delivered server tick; it returns `None` while in
progress and `Some(TravelOutcome)` at a terminal state
(`Arrived`/`Stalled`/`Refused`/`Blocked`/`GaveUp`). One driver send per
call. `TravelOptions { close_enough, budget_ticks_per_hop, max_hops,
on_leg, troll_doors }`.

- **Default door leg:** interact the door transport's menu option, then
  settle `arrived(to, close_enough)` — cheap, no per-tick door polling.
- **`troll_doors = true` (non-default, expensive):** per tick, read the
  door's open/closed state from the snapshot's `locs()`; when the door
  reads open, `op_loc` (re-open) and `walk` through in the **same tick**
  so a tick-perfect closer cannot slam it (the `2026-08-22-bot-nav.md`
  same-tick rule). Use only for the live door-troll fixture; ordinary
  routes pay the cheap default.

## WalkTo picker

The panel's main-chrome **WalkTo** button fills the Game pane
(`crates/panel/src/picker.rs`): north-up walkable tiles from
`NavWorld.collision` as amber dots, drag/wheel to pan, click (canvas rect,
`is_mouse_hovering_rect`) highlights the nearest walkable tile, footer
**Recentre** / **Walk** arms `find` and the panel drives `follow` on the
focused slot's pump. Local engines also get **Teleport** (cheat to the
pick). `walk_status_text` mirrors the armed dest and clears on any
terminal outcome.

## Live tests

```bash
LIVE=1 cargo test -p e2e --test nav_full -- --ignored --test-threads=1
LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1
```

`nav_full`: `find` + `follow` (Lumbridge courtyard → (3220,3264,0)).
`nav_door` is the gold fixture if something regresses: two slots — the
walker crosses Catherby door 1530 to (2817,3443,0) with `troll_doors:
true` while a tick-perfect closer keeps the door closed; PASS on
`Arrived`. Additional live tests under `crates/e2e/tests`: `nav_gates`,
`nav_cart`, `nav_spirit`, `nav_wildy`, `nav_toll`, `nav_essence`,
`nav_elkoy`, `nav_zanaris`, `nav_collision`, `nav_seers_crabs`. Headed
corpus: `cargo run --release -p panel --bin panel-play -- --live script_nav_routes`.

## Credit

Router/Traveller shape borrows from m8aq's api nav/travel and the
RuneLite `shortest-path` plugin (collision + transport graph + Dijkstra).
The Rust is our own; no rsmod wasm is vendored. Collision/transport truth
is the Server content, scoped to the 2004 surface.
