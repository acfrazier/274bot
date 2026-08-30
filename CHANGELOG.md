# Changelog

All notable public changes to 274bot. Crate versions are `0.1.0` and
`publish = false` (not on crates.io).

## [0.1.0] — unreleased

Alpha tag of the **bot host + API + nav**. Honest bot scripts are not
part of this release.

### Host and panel

- One OS thread per client, 20 ms loop, login FIFO, AES-256-GCM vault.
  Login throttle matches Lost City production (`30` / 60 s per address,
  `4` then remaining of 15 s per device). No 2.5 s inter-grant gap — the
  engine has none; local `production: false` does not apply the counters.
- Native `panel-play` (dear-app / ImGui): accent profile name + **Profiles**
  picker (user/pass only while editing), **Log in** / **Logout** above
  WalkTo (shown disabled if the vault is locked or no profile is
  focused), status, log, MultiBox rail/grid, click-through capture.
  Host-window resize is **grid-only**; panel/rail stay 330/264 wide and
  grow vertically. Non-grid Game blit is native 765×503, flush to the
  panel. Opening MultiBox **grows** the OS window if needed so the blit
  is not covered; it does not shrink a larger window. No dock splitters
  / tab-bar corner menu. DPI is OS/winit. Wrong passphrase never
  replaces the vault; **Reset vault** is explicit.
- Under WalkTo, above profile/debug: **General config** (**slot**
  capture + auto-login, **render** none/GPU/CPU + focused 50 fps,
  **global** sidecar 50), **Nav config** (its own window), **Loadouts**
  mocked until the TS shim. Add-bot is a window, not a blocking modal.
  Per-slot **none / GPU / CPU**; click lowmem/highmem for a sticky mem
  popup (like Teles). Status shows mem. **focused 50 fps** is Game-pane
  only (not capture, does
  not follow that client onto the rail). **sidecar 50 fps** is all
  rail/grid members. Switching GPU↔CPU or mem drops + reattaches the
  head on the same `Client` — never a logout or restart.
- Rail fold (`▂` / `▅`) next to ✕; focused blit is folded by default.
- Build line is `alpha 1 ·` git short SHA (`-dirty` if the tree was dirty).
  Hover is crate version (`0.1.0`) then full commit + build time.
- Headed `--live stress50` is the **release** 50-head RAM watch (cap-only
  rail, Game 50 fps). Rail members are **raster Off** (cannot grow a GPU
  `RenderWorld`); only `s00` is GPU. `--live stress50_full` is the same
  wall with every Game + sidecar renderer at 50 fps. Neither fails on RSS
  size; PASS prints `rss=… up=50/50`.
- Skip-paint RAM: overlay ground verts are inline (no 9 Vecs per tile).
  Headless 50-bot RSS was ~4 GB because every slot loaded a rustysynth
  sequencer + SF2 at spawn (panel `audio` feature); midi is lazy until
  `play`, and one process soundfont is shared. Empty ground-obj cells are
  fat pointers (~346 KB/client, was 3.8 MB of inline lists). One
  OnDemand worker (and one byte-15 update socket) is shared per
  `(host, port)` — fifty game logins no longer open fifty extra TCP
  connects (login code −1). `fill_base_level` no longer allocates 10816
  empty `Square`s (~5.4 MB/slot); occupied tiles are created on place.
  Occupied `Square` is under 200 B (was 496): loc/overlay records boxed,
  sprite slots packed as `u32`. The `IfTypeMut` overlay template is one
  `Arc` until a slot writes (`Arc::make_mut`). JagFX synth/delays are one
  process table per `cache_dir`; generate clones one `Sound` into per-slot
  scratch. Headed loc decode hits the process LRU as an `Arc` (`SceneModel::Shared`);
  two heads share the same geometry instead of cloning `Model` per tile.
  Live `rss_ladder` prints `ondemand=` / `tcp=` and fails if there is not
  exactly one OnDemand worker (does not fail on RSS size). OnDemand hub
  Completeds for models/anims are not dropped when the process provider
  requested them (`check_scene` loc wait).
  Sparse IfType slots are boxed (11k holes were 688 B each, cloned per
  client). Empty scene tiles / player / NPC slots are boxed
  pointers (was ~29 MB of unused `Square`s per client). After a snapshot
  inject, slots do not prefetch the whole 15 MB map archive; map-build
  byte buffers are dropped once the sim world is stamped.
- Snapshot inject (`~/.274bot/unpack`) is **once per process**. A later
  slot's `maininit` no longer wipes the process-wide model/anim stores or
  re-reads `models.bin` (that print was every client, not a 12 GB jag).
- The Game `Renderer` is a **head on the sim**, not a second client.
  Draw off (`set_draw(false)`) detaches it — GPU textures, chrome, and
  the decoded 3D scene are freed, the socket and sim keep running — and
  the next paint reattaches on the **same** `Client`. Clicking a sidecar
  line is focus (retarget the pane, wake draw), never a restart.
  GPU↔CPU / lowmem flips drop + reattach the head on the live `Client`.
- Immutable decode is **one process copy**: IfType tables, fonts/media,
  and GPU pipelines/shaders are shared `Arc`s. Fifty slots clone
  pointers; each headed slot pays one frame texture; each unheaded slot
  holds only its mutable sim. Unheaded `map_build` stamps
  typecodes/heights/collision, not overlay mesh — the first headed paint
  materializes the overlay from those.
- Headed default is the client submodule's **wgpu** 3D renderer
  (`BOT_CPU=1` still forces CpuPix3D on CLI slots without a raster pref).
- WalkTo picker in the Game pane (north-up, click-to-pick, Recentre /
  Walk; **Teleport** cheat on loopback engines).
- Local-engine debug heading: TutSkip is omitted once the profile is
  known skipped (`getvar tutorial` first when unknown), Lumbridge (`~home`), maxme
  (`setstat`), Teles popup. DebugPanel v2 is a disabled stub.
- Script chrome Browse / Start / Pause / Stop + JS Load. Parameters Edit
  stays mocked. Loadouts stays mocked until the TS shim.

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

### After 0.1.0 / beta (deferred)

- 3-platform bins; live cache fetch / turnkey rs2b2t; `--prod` as a
  supported scenario.
- Client source comments that still mention `$HOME/experiments/Server`
  as the TS port path; tests use `engine_dir` / `cache_dir` / `content_dir`.
- Nav OP_NPC execute (v0.1.1). Script UI polish (not this alpha).

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

Alpha gaps (honest, not this tag): DebugPanel v2, Loadouts / parameter
Edit, crates.io publish. Nav config is a window (not a blocking modal).
