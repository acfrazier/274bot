# Changelog

All notable public changes to 274bot. Crate versions are `0.1.0` and
`publish = false` (not on crates.io).

## [0.1.0] — unreleased

Alpha tag of the **bot host + API + nav**. Honest bot scripts are not
part of this release.

### Host and panel

- One OS thread per client, 20 ms loop, login FIFO, AES-256-GCM vault.
- Native `panel-play` (dear-app / ImGui): credentials 2×2, status, log,
  MultiBox rail/grid/chooser, click-through capture.
- Headed default is the client submodule's **wgpu** 3D renderer
  (`BOT_CPU=1` forces CpuPix3D).
- WalkTo picker in the Game pane (north-up, click-to-pick, Recentre /
  Walk; **Teleport** cheat on loopback engines).
- Local-engine debug heading: TutSkip latch, Lumbridge (`~home`), maxme
  (`setstat`), Teles popup. DebugPanel v2 is a disabled stub.
- Script chrome Browse / Start / Pause / Stop + JS Load. Parameters Edit,
  Global settings, and Loadouts stay mocked.

### Nav

- Whole-world collision + transport pack (`274V`, version byte **5**).
- Dijkstra `find` / `find_with` (`FindOptions`: wilderness and teleport
  opt-in, both default off) and pollable `Traveller::follow`.
- Transport coverage includes doors, ladders/stairs, agility, gates,
  spirit trees, cart NPC hops, wilderness levers, Al Kharid toll / Shantay
  northbound, essence-mine **entry**, Elkoy escorts, Zanaris shed + worn
  Dramen. Cow-pen → Varrock uses the south gate (`INTERACT_RADIUS` 1).
- Traveller does **not** yet execute OP_NPC hops (cart / essence / Elkoy)
  or EssenceSession return / Shantay free-exit / tele **execution**.

### API

- Snapshot → query → interact → settle. No tick-end opcode; compiled
  scripts wake on the `PLAYER_INFO` gen edge.

### Client (`FR-client-bothost` `r274-bh-modular`)

- Bot-host fork: gens, skip-paint, shared cache, wgpu GPU 3D (CpuPix3D
  via `BOT_CPU=1`). No bot action API inside `client`.
- Login RSA is **runtime**: stock LC Java pair for local-dev; optional
  `$ENGINE_DIR/data/config/private.pem` or `LOGIN_RSAN` if you rotated
  keys. Public-world keys are baked in the client (unadvertised in alpha).

### Contributor fence (alpha)

You bring a local engine, pack cache, and RSA bake. No AI-slop drive-by:
the bar is “you know what you are doing with these tools.” Live rs2b2t
**turnkey** (fetch cache like rs2b0t, `Arc<Cache>` from the prod server)
is a **beta** goal, not this tag.

### Optimization

Each component should do one job well (old-school, Gower-era). Memory is
a first-class product concern — that is why this host is Rust. RSS
ladder stays the measurement surface; do not “optimize” by lying.

### After 0.1.0 (planned, not this tag)

- **v0.1.1 — nav strikes again:** Traveller OP_NPC execute, EssenceSession
  return, Shantay free-exit, tele execution, `find` reading edge reqs /
  `WorldState`. Nav lint/`-D warnings` belongs here, with live fixtures.
- **v0.1.5:** TS rs2b0t compatibility shim; some scripts hand-tested, not
  all. Pivot: not “port every bot.”
- **v0.2.0:** hot-load `.ts` as one-session tasks (Load kernel already
  exists; this is the productization).
- **v0.2.5 beta:** listed scripts written against **our** API or the
  **rs2b0t** API load and run. “Just work” means the scripts we name,
  not every historical rs2b0t bot.

Alpha gaps (honest, not this tag): DebugPanel v2, Global settings /
Loadouts / parameter Edit, crates.io publish.
