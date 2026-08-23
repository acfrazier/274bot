# Nav: routing, walking, and the WalkTo picker

`crates/nav` is the campaign nav stack: a baked walkability grid, A*
routing, a per-tick traveller, and (via the panel) a click-to-walk tile
picker. It acts only through the kernel API — `api::interact` (`walk`,
`op_loc`) — and never deep-copies the world.

## Pack bake

`nav-pack` reads Server mapsquare jm2 text plus the door/loc scripts and
writes a nav pack:

```bash
cargo run -p nav --bin nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR]
```

Defaults: `MAPS_DIR=/Users/acfrazier/experiments/Server/content/maps`,
`DOORS_CONFIG_DIR=/Users/acfrazier/experiments/Server/content/scripts/doors/configs`.
Two mapsquares are baked: Lumbridge `m50_50.jm2` and Catherby
`m44_53.jm2`. Output goes to `$NAV_PACK` or, by default,
`~/.274bot/274bot.navpack`. Missing mapsquares are skipped with a stderr
count; the remaining squares still produce a pack. The bake exits non-zero
when no door configs parse, no mapsquare bakes, or the pack file cannot be
written.

Collision comes from the Server jm2 text (MAP `fN` flags, LOC blockwalk
footprints, door configs), **not** from the client's `.lcnav.gz` webwalk
files. Openable wall doors (loc `op1=Open` / `category=door_closed`, shape
0) become directed `DoorEdge`s; their own tile stays unwalkable. Pack
format: magic `b"274N"`, version 1, origin/width/height, one walk byte per
tile (row-major z then x), then door entries (see
`crates/nav/src/pack.rs`).

## Router

`nav::router::find(grid, from, to)` is A* over the 4-neighbour grid
(N/E/S/W, cost 1, chebyshev heuristic) extended by door edges (cost 2).
Same level only. Returns `Route { legs, dest }` or `NoPath`; legs split
around doors: `Walk` … `Door { loc, loc_id, from, to }` … `Walk`.

## Traveller

`nav::traveller::Traveller::tick(driver, here, door_open)` drives a route
one hop per tick through the `Driver`; the caller supplies the player's
tile and the door's live open state each tick. Walk legs aim at the leg far
end when ≤ 20 tiles away, else a tile ~15 steps ahead so the client
re-routes a short fresh path; a rejected `tryMove` falls back to the next
leg tile. A door leg sends `op_loc` — the packed wall typecode via
`Driver::loc_typecode`, falling back to the loc id — and, when the caller
reports the door open, re-opens and walks through on the **same tick** so a
closing door cannot slam. Statuses emitted today: `Idle`, `Walking`,
`Door`, `Arrived`, `Budget` (60 ticks per hop, reset on any advance); the
enum also declares `Closest`, `Blocked`, and `Interrupted` for later.
Arrival is `nav::arrival::arrived`: standing on the dest, or adjacent to it
when the dest is solid — Traveller currently calls it with
`dest_walkable = true`, so solid-adjacent arrival is in place but not yet
active (every armed dest is walkable today).

## Catherby fixture

The closed range-house door: loc **1530** at tile **2816,3438,0**
(mapsquare 44,53, local 0,46), crossing (2816,3437) → (2816,3439). The
pack bake tests and the live `nav_door` harness both pin this door.

## WalkTo picker

The panel's main-chrome **WalkTo** button opens a collision-dot map
(`crates/panel/src/picker.rs`): walkable tiles from the loaded pack as
amber dots, drag to pan, click highlights the nearest walkable tile,
**Walk** arms the focused slot's traveller (`Session::arm_walk_on`) and
closes. The pack loads once per process from `$NAV_PACK` or the
`nav-pack` default; without it the window shows a "run nav-pack" hint.

The amber polyline on the Game Image is a **schematic** overlay: remaining
A* walk tiles drawn as a flat 52×34 grid centred on the player, not a
projection through the 274 camera. It will not sit on the 3D path you see.

## Live tests

```bash
LIVE=1 cargo test -p e2e --test nav_walk -- --ignored --test-threads=1
LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1
```

`nav_walk`: one slot, Lumbridge courtyard walk to (3220,3212,0);
`arrived` within 90 s of arm. `nav_door`: two slots in one process — the
walker stages outside the Catherby range house and walks through door 1530
to (2817,3443,0) while a tick-perfect closer keeps the door **closed**
(`op_loc` whenever the loc is not the closed id 1530); `chebyshev ≤ 1` of
dest within 120 s.

## Credit

Router/Traveller **shape** is a borrowed idea from
[m8aq](https://github.com/) apiv2 nav/travel — the Rust here is our own,
and m8aq's wasm is not vendored. The picker's **collision dots** visual
comes from rs2b0t's WorldMapPicker idea; it is not a port of
`src/bot/event/webwalk`. Collision bake is the Server jm2, not `.lcnav.gz`.
