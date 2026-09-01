# Changelog

All notable public changes to 274bot. Crate versions are `0.1.0` and
`publish = false` (not on crates.io). Git tags are `0.1.0`, `0.1.1`, …

## [0.1.1] — 2026-09-01

Nav **execute**. Headed gold: `panel-play --live script_nav_routes`
`PASS arrived(2817,3443,0)`. Honest bot scripts are still not this tag.

### Nav

- `WorldState` from the live snapshot fail-closes `find` (skills, items,
  quests, transmit-yes varps, worn). `worn_req` is **any-of**.
- `FindOptions.allow_bank_fetch` is named (checkbox + flag); fetch is not
  implemented.
- `Traveller::follow` executes packed OP_NPC (cart, essence wizard, Elkoy,
  sailors, glider pilots), EssenceSession return, Shantay both ways, and
  packed teles. WalkTo **Teleport** stays `::tele` on loopback.
- Boats: Talk-to the sailor onto the **deck**, then loc Cross the
  `_gangplank_disembark`.
- Gliders: take-off if varp 150 ≥ 160 **or** the Grand Tree journal is
  green; landings settle Chebyshev 1 (`map_findsquare` scatter).
- Slashable webs (loc 733): `oplocu` knife (`option` 0, `item_req` 946)
  and `oploc1` Slash (`option` 1) when any `slashattack_anim` blade is
  worn. Traveller trolls the 50% slash fail like a door.
- Agility shortcuts wait the packed `edge.ticks` after land.
- NPC-backed hops search radius 8 and walk to the **live** NPC tile
  (packed `at` is spawn; officers wander).
- Pack `274V` version **7**: 9-bit walk (`u8` face + `SQ_BLOCKED`). v6 is
  `BadVersion`. Rebake.
- Path-facing orbit yaw (rs2b0t `navCameraFollow` shape; host writes
  `orbit_camera_yaw`).
- Remaining transport hops get a short caption on the Game overlay.

### Scenario / live

- Unique live account per invocation; `--live` uses an ephemeral vault and
  does not write operator settings.
- FAIL dumps include chat (newest first) and `tile` `[x,z,level]`.
- `script_nav_routes` is the headed corpus; `nav_door` stays the Catherby
  door-troll gold fixture.

### Host and panel

- Auto-run (bothost IF_BUTTON) only after `ingame && scene_state == 2`.
- Headed live paints the scenario Follow route, not only WalkTo.
- Scenario nav overlay (paints, camera, find flags) is session-only.

### Client (`FR-client-bothost` `r274-bh-modular`)

- GPU keeps the last 3D texture while `scene_state == 1` (Java freeze;
  overlays such as `ship_journey` still draw).
- Orbit camera chases during that freeze; `LinkBelow` lifts pitch-clamp
  samples so bridges do not slam top-down.
- Logout clears the tutorial overlay (`tut_com` / flash / modals).
- Nav debug hop labels.
- Client CI: fmt + clippy `-D warnings` + test (same bar as the host).

### Git

- `origin/main` is this checkout’s commit history. The `0.1.0` tag remains
  the squash that first went public. Later tags are ordinary annotated
  tags on `main`. Do not squash-publish.

### After 0.1.1 (planned, not this tag)

- **v0.1.2:** headless TUI (ledger only).
- **v0.1.5:** TS rs2b0t compatibility shim; listed scripts, not all-ports.
- **v0.2.0:** hot-load `.ts` as one-session tasks.
- **v0.2.5 beta:** listed scripts against **our** API or the **rs2b0t**
  API.

Alpha gaps (honest, not this tag): DebugPanel v2, Loadouts / parameter
Edit, crates.io publish, 3-platform bins. Random-event handler is 0.1.2+.
Zone-reuse / extra map chunks are not this tag.

## [0.1.0] — 2026-08-31

Alpha of the **bot host + API + nav**. Honest bot scripts are not part of
this release. The script *kernel* (Browse / Start / Pause / Stop, JS Load)
and WalkTo are in-tree.

### Host and panel

- One OS thread per client, 20 ms loop, login FIFO, AES-256-GCM vault.
  Login throttle matches Lost City production (`30` / 60 s per address,
  `4` then remaining of 15 s per device). Local engines default
  `production: false` and do not apply those counters; the host still
  stays under them.
- Native `panel-play` (dear-app / ImGui): profile picker, Log in / Logout,
  WalkTo, MultiBox rail/grid, click-through capture. Panel/rail widths
  stay 330/264; host-window resize is grid-only. Non-grid Game blit is
  native 765×503. Opening MultiBox grows the OS window if the blit would
  be covered. DPI is OS/winit.
- Per-slot none / GPU / CPU. GPU↔CPU or lowmem/highmem drops and
  reattaches the **renderer head** on the same `Client` — never a logout.
  Draw-off detaches GPU textures, chrome, and decoded overlay mesh; the
  socket and sim keep running. The next paint reattaches from parked
  stamps. Sidecar click is focus, not a restart.
- Game pane follows the **focused** slot. With only-render-selected (the
  stress50 default), unfocused rail members stay raster Off and cannot
  grow extra GPU heads. The focused member can take the GPU seat even if
  its rail tile is Off.
- `--live stress50` is the release 50-head RAM watch (cap-only rail, one
  GPU seat, Game 50 fps). `--live stress50_full` paints every member at
  50 fps. Neither fails on RSS size; PASS prints `rss=… up=50/50`.
- One process copy of IfType decode, fonts/media, and GPU pipelines.
  One OnDemand worker (and one update socket) per `(host, port)`. Occupied
  scene tiles are created on place; empty squares are holes. Loc geometry
  is a process LRU (`SceneModel::Shared`). Unheaded `map_build` stamps
  typecodes/heights/collision; the first headed paint materializes overlay
  and the minimap. Process-wide loc models are not unloaded per 104.
- Headed default is the client submodule's **wgpu** renderer
  (`BOT_CPU=1` is CpuPix3D).
- WalkTo picker in the Game pane (north-up, click-to-pick, Recentre /
  Walk; **Teleport** cheat on loopback engines).
- Local-engine debug heading: TutSkip, Lumbridge (`~home`), maxme,
  Teles. DebugPanel v2 is a stub.
- Script chrome Browse / Start / Pause / Stop + JS Load. Loadouts and
  parameter Edit stay mocked until the TS shim.

### Nav

- Whole-world collision + transport pack (`274V`, version byte **6**):
  compact `u16` walk words per tile. Optional `274F` flags sidecar for
  collision paint. Rebake; v5 streams are `BadVersion`, not silently
  loaded as v1.
- Dijkstra `find` / `find_with` (`FindOptions`: wilderness and teleport
  opt-in, both default off) and pollable `Traveller::follow`.
- Transport coverage includes doors, ladders/stairs, agility, gates,
  spirit trees, cart NPC hops, wilderness levers, Al Kharid toll / Shantay
  northbound, essence-mine **entry**, Elkoy escorts, Zanaris shed + worn
  Dramen. Cow-pen → Varrock uses the south gate (`INTERACT_RADIUS` 1).
- Reach overlay is **paint-only** (not `find`). Traveller does **not**
  yet execute OP_NPC hops (cart / essence / Elkoy) or EssenceSession
  return / Shantay free-exit / tele **execution**.

### API

- Snapshot → query → interact → settle. No tick-end opcode; compiled
  scripts wake on the `PLAYER_INFO` gen edge.

### Client (`FR-client-bothost` `r274-bh-modular`)

- Bot-host fork: gens, skip-paint, shared cache, wgpu GPU 3D (CpuPix3D
  via `BOT_CPU=1`). No bot action API inside `client`.
- Login RSA is **runtime**: stock LC Java pair for local-dev; optional
  `$ENGINE_DIR/data/config/private.pem` or `LOGIN_RSAN` if you rotated
  keys.

### Contributor fence (alpha)

You bring a local engine and pack cache. Live cache fetch / turnkey
public-world login is a **beta** goal, not this tag.

Shipped as the public squash tag `0.1.0`. Later history is on `0.1.1`.
