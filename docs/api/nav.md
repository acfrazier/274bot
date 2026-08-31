# Nav: whole-world routing, transports, and WalkTo

`crates/nav` is the single nav stack: a whole-world collision bake, a
content-derived transport graph, a Dijkstra router, and a pollable route
follower. It acts only through the kernel API (`api::interact`,
`api::settle`) and never deep-copies the world.

## Pack bake

`nav-pack` reads every Server mapsquare jm2 plus the door/loc/rs2 scripts
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
`u16` walk words per tile, row-major z-then-x) plus the derived
`TransportGraph`. Magic `b"274V"`, version byte **6**. Raw `u32` flags are
not on the v6 wire; the optional `274F` sidecar holds them for collision
paint. `decode` accepts version 6 only — v5 and older are
`BadVersion`. The `274N` grid decoder (`decode_grid`) stays for old
boolean-walk files.

## Collision (`nav::collision`)

`WorldCollision { origin, width, height, walk: Vec<u16>, flags: Option<Vec<u32>> }` bakes every
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
wilderness levers, Al Kharid toll / Shantay northbound, essence-mine
wizard **entry**, Elkoy maze escorts, Zanaris shed door with worn Dramen
req. A `TransportEdge` carries `kind` (Door/Ladder/Stairs/Boat/Teleport/
AgilityShortcut/Glider/SpiritTree/Npc), `from`/`to`, `loc_id`, the
1-based menu `option`, `ticks`, and requirement vectors including
`worn_req`. Spell teleports have no fixed origin: they live on
`TransportGraph::teleports` and stay out of Dijkstra unless
`FindOptions::allow_teleports`. Wilderness tiles stay out unless
`FindOptions::allow_wilderness`. Both default **off**.

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

`Traveller::follow` still walks door/ladder-style loc hops. **OP_NPC
execute** (cart / essence wizard / Elkoy), EssenceSession return, Shantay
free-exit, and tele **execution** are not in this tag: the pack can
contain the edges, the walker does not yet fire those ops.

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
`nav_elkoy`, `nav_zanaris`, `nav_collision`, `nav_seers_crabs`.

## Credit

Router/Traveller shape borrows from m8aq's api nav/travel and the
RuneLite `shortest-path` plugin (collision + transport graph + Dijkstra).
The Rust is our own; no rsmod wasm is vendored. Collision/transport truth
is the Server content, scoped to the 2004 surface.
