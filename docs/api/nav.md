# Nav: whole-world routing, transports, and WalkTo

`crates/nav` is the single nav stack: a whole-world collision bake, a
content-derived transport graph, a Dijkstra router, and a pollable route
follower. It acts only through the kernel API (`api::interact`,
`api::settle`) and never deep-copies the world.

## Pack bake

`nav-pack` reads every Server mapsquare jm2 plus the door/loc/rs2 scripts
and writes a **v2** nav pack:

```bash
cargo run -p nav --bin nav-pack [MAPS_DIR] [DOORS_DIR] [CONFIG_DIR]
```

Defaults: `MAPS_DIR=/Users/acfrazier/experiments/Server/content/maps`,
`DOORS_DIR=/Users/acfrazier/experiments/Server/content/scripts/doors/configs`,
`CONFIG_DIR=/Users/acfrazier/experiments/Server/engine/data/pack/config`.
Output goes to `$NAV_PACK` or `~/.274bot/274bot.navpack`.

The pack serializes the whole-world `WorldCollision` (one `CollisionFlag`
bitmask per tile, row-major z-then-x, u32 each) plus the derived
`TransportGraph` (doors, ladders, stairs, agility shortcuts, with their
requirements and tick costs). Magic `b"274V"`, version 2 — see
`crates/nav/src/pack.rs` `encode_v2`/`decode_v2`.

## Collision (`nav::collision`)

`WorldCollision { origin, width, height, flags: Vec<u32> }` bakes every
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
(openable doors), `ladders+stairs/` and `areas/` rs2 scripts
(`p_telejump`/`p_teleport`/`~climb_ladder`, `movecoord` landings),
`skill_magic/` teleports, and `skill_agility/` shortcuts. A
`TransportEdge` carries `kind` (Door/Ladder/Stairs/Boat/Teleport/
AgilityShortcut), `from`/`to`, `loc_id`, the 1-based menu `option`,
`ticks`, and `skill_req`/`item_req`/`quest_req`/`varp_req`. Boats and
teleport spells have no content-derivable fixed origin, so they are
skipped with a stderr count, never faked.

## Router (`nav::router`)

`find(collision, graph, from, to) -> Result<Route, RouteError>` is Dijkstra:
0-cost 8-directional tile steps through a deque, transport edges through a
min-heap at `edge.ticks` cost. Tile steps use the client's directional
`PL_WALK_*` masks (face + corner + scenery + ground), **not** the blanket
`walkable()`. `Route { legs, dest, ticks }`; `Leg::Walk { tiles }` runs
collapse, `Leg::Transport { edge }` is one per transport. `RouteError` is
`NoPath` or `BudgetExhausted` (a node-expansion cap). `find` is CPU-heavy;
run it off-pump (a short-lived worker) and arm the result.

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

The panel's main-chrome **WalkTo** button opens a collision-dot map
(`crates/panel/src/picker.rs`): walkable level-0 tiles from
`NavWorld.collision` as amber dots, drag to pan, click highlights the
nearest walkable tile, **Walk** arms `find` and the panel drives `follow`
on the focused slot's pump. `walk_status_text` mirrors the armed dest and
clears on any terminal outcome.

## Live tests

```bash
LIVE=1 cargo test -p e2e --test nav_full -- --ignored --test-threads=1
LIVE=1 cargo test -p e2e --test nav_walk -- --ignored --test-threads=1
LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1
```

`nav_full`/`nav_walk`: a `find` + `follow` route across formerly-missing
squares (Lumbridge courtyard → (3220,3264,0) for `nav_full`).
`nav_door`: two slots — the walker crosses Catherby door 1530 to
(2817,3443,0) with `troll_doors: true` while a tick-perfect closer keeps
the door closed; PASS on `Arrived`, FAIL on any other terminal outcome.

## Credit

Router/Traveller shape borrows from m8aq's apiv2 nav/travel and the
RuneLite `shortest-path` plugin (collision + transport graph + Dijkstra).
The Rust is our own; no rsmod wasm is vendored. Collision/transport truth
is the Server content, scoped to the 2004 surface.
