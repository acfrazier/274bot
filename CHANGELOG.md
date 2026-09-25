# Changelog

All notable public changes to 274bot. Host workspace crate versions are `0.1.8` and
`publish = false` (not on crates.io). Git tags are `0.1.0`, `0.1.1`, …

## [0.1.9] — 2026-09-24

### Rendering and client

- Logged-out title-screen brazier flames animate again on CPU and GPU. Full-rate
  views follow the 35 ms flame clock, 1 fps rail tiles catch up when painted,
  and draw-off bot slots remain raster-free.

### Slot lifecycle

- Slot startup, stop and restart now have one owned lifetime: per-slot
  registries are published before the worker starts, stopped and crashed
  workers release their queue place, and Log in or Login all recreates a
  terminal worker. Panel rail removal waits and reaps asynchronously instead
  of joining a worker on the UI thread; re-adding, re-seeding or logging in
  during that window cancels only the matching lifetime's removal and restores
  its IO.
- Login and logout commands are serialized by generation, so completion of an
  old logout cannot erase a newer Login all while a successful compatible
  handshake still consumes its one-shot intent. Retry notifications serialize
  with the wait predicate, and Logout latches at the command boundary,
  including while a slot is preparing or parked.
- Connected session state is distinct from scene/player readiness. Connection
  lights, loading labels, Logout and clean removal remain correct while a
  scene loads, while game actions and routing still require a current player
  and valid tile.
- Disconnect clears session byte/run counters but retains paused script paint;
  Stop and unload clear published paint even offline. Status-lock poison from
  a panicked worker is recovered at every access, and script-slot poison is
  contained at terminal retirement. Welcome dismissal keeps its spaced attempt
  bound and reports a visible failure after 10 eligible seconds; the window
  restarts while the scene cannot accept a close.

### Script host
- The FlourCollector catalog card now loads its four Murder Mystery area facts
  through the shim; EssMiner stays visibly dimmed until native Gatherer support
  replaces its pickaxe-acquisition dependency.

- Alcher and LeatherCrafter can load their reachable-bank selector again.
  Selection runs one bounded native multi-target search off the slot pump,
  keeps the same-plane radius-four shortcut, and falls back to air-nearest
  on an unavailable route or a five-second completion timeout. A typed v2
  `bankNearestReachable` helper exposes the same select-only capability.
  The complete 20-bank catalog now preserves stable order, base-skill/quest
  gates, default-off Mage Arena/Zanaris preferences, and object/NPC access.
  Selected-world placements resolve safe walk stands; unavailable Canifis
  content stays a gated air fallback rather than an invented reachable bank.
  Stand resolution retains usable counter-side floor tiles with directional
  wall faces (including Varrock West), while still excluding blocked footprints.
  The selection waiter also bounds requests dropped before host admission;
  explicit v2 opt-ins override settings for both routing and air fallback.
  Native `walk-nearest-bank` reuses the winning route instead of flooding again;
  Stop/reload invalidates pending picks without replacing an existing follow.
  World bank facts now require explicit binding, so an early read cannot freeze
  an empty placement roster.
- `Banking.open`, periodic banking and world bank opens now share a Rust-owned
  select/walk/access continuation. Nearby banks still beat distant presets;
  explicit destinations remain the no-scene fallback. NPC and object access
  metadata survive selection, and deposit callbacks wait for loaded bank stock.
  An NPC bank without a dialogue choice never selects an unrelated first option.
  A focused `alcher_dwarven_mine` scenario observes dungeon exit, the selected
  Falador East bank, real withdrawal and fresh High Alchemy XP.

- Baker-stall carried-food counting matches frozen substring patterns, so
  partial cakes (`2/3 cake`, `Slice of cake`) satisfy restock and eat gates
  the same way whole `Cake` does.

- Baker-stall restocking now awaits one Rust step machine; Rust owns selected
  stall facts, callback polling, waits, steal verbs, stand swaps and lockout
  sequencing while callbacks retain the options object as their receiver.
- The v2 clue session is begin plus one awaited run; Rust owns its continuation
  table, optional callbacks, waits and typed verb emission.
- The v2 quest journal is begin plus one awaited run; Rust owns its row click,
  modal acquisition, retry window, exact-pair close and timeout.
- The obsolete clue-verb JSON adapter is removed; the clue machine emits typed
  interact requests directly.

## [0.1.8.1] — 2026-09-24 — Alpha 3 patch

A patch on 0.1.8: a public-289 crash, both public worlds, the login queue and
three rendering bugs. Crate versions stay `0.1.8`; the tag and packages are
`0.1.8.1`.

### Crash

- Fixed an abort on public-289 when a queued account retried after its login
  cooldown had already expired (a negative duration in the login queue).

### Public worlds

- Public 289 world endpoints come from editable `~/.274bot/worlds.json`
  (w1/w2 defaults); account settings can pin a world or choose auto, and
  panel edits apply to an already-running slot at its next login handshake.
- Auto accounts move to the next world on a full-world login response, with
  a pause after all worlds report full. Panel and TUI show the world used by
  the current connection; `tui-play --world N` sets the default for auto
  accounts. The panel rail marks each member with its world number.
- Public login fetches the world's RSA modulus at runtime with a baked-key
  fallback and refreshes after a wrong-key response. The shared cache tries
  the next listed asset world if the first cannot be reached.
- Login response 21 (just left another world) shows the server's transfer
  countdown and then logs in again on the same world, instead of failing as
  an unexpected response.

### Login queue

- The queue is first come, first served in the order bots are ready to log
  in, with the focused bot moved to the front. Only a bot that is actually
  waiting holds a place, so an online or still-loading bot can no longer
  block everyone behind it (Login all could previously stall with
  "1 of n, 0 in front"). A bot that stops or crashes gives up its place.
- Each bot draws its own queue card ("k of n") on its own view; it appears
  only while that bot is held in the queue and clears on login.
- Every login attempt counts toward the server's per-address and per-account
  limits, a server "too many attempts" reply pauses all bots together, and
  retry waits are not cut short by clicks, focus changes or Login all.
- Turning auto-login off and on keeps an explicit Log in.

### Panel

- Rail and grid labels name the login step (starting, queued k/n, logging in,
  loading) instead of "logged out" until the bot is in game.
- The game view is centred horizontally in its pane.
- The script load-failure list is collapsed by default.

### Rendering

- Players and NPCs behind a wall are no longer drawn through it (Fishing
  Guild bank parapet, CPU and GPU): the scene painter completes each tile's
  back pass like the Java client.
- Walls sharing lighting with their neighbours no longer turn black when a
  ground item or a wall/floor decoration on the same tile changes or animates.

## [0.1.8] — 2026-09-24 — Alpha 3

JS API v1 compatibility with the frozen rs2b0t catalog on revision 289, and the
JS shim pulled back toward name maps: most loops, tables, decisions, retries and
sequencing now live in Rust step machines that read the snapshot Rust already
holds (residuals are listed under Known limits). Revision 274 remains
best-effort and untested in this release.

### Script runtime (fence)

- One decoded scene per isolate; Rust helpers read it instead of JS-posted
  pages. The runner, park list and a single paint pass per tick are Rust-owned.
- A Rust step-machine host (`runMachine`): each driver is one native start and
  one await, with exclusive supersede, reset/pause/hold semantics, join and
  watchdog claims, and one Rust→JS callback path (synchronous hooks stay
  synchronous, as in frozen). Teleport, prayer, autocast, special, modals, bank
  open/access/deposit/withdraw/close, PeriodicBank, DeathRecovery, bankNearest,
  light fire, shop, chat dialog (make/makeX/makeFromPanel/chooseOption), dialog,
  reach (npcDialog/entityOp), walkWithHops, walkResilient, trade, partner trade,
  clue/Sherlock and every hunt family run on it.
- The v2 hunt family is begin plus one awaited run with typed `.d.ts` inputs;
  the nine v2 hunt examples are rewritten to that contract.
- Paint: canvas ops record on a tape and flush once per pass; paint frames are
  shared by `Arc` with sender-side caps; the FlatBuffer paint codec is removed.
  `fmtDuration`/`fmtXpHr`/`etaHours`/`levelProgress`/`paintSkillShort` emit the
  rs2b0t strings.
- Isolate Start/Stop no longer block the UI thread; the isolate command backlog
  is bounded; no JSON text is evaluated as source on the tick path.

### Compatibility and behavior

- v1 `onPaint` runs only after `onStart` succeeds and while the reader is ready
  (in game, scene 2, a tile, stats loaded), as frozen `ScriptRunner.paintBot`.
- Walk arrival is one Rust rule equal to frozen `isArrived` (reach probes), used
  by the isolate wait, the host follow and every shim pre-check; a route that
  ends on its approach tile settles true (frozen "closest").
- `Tile.distanceTo` runs in Rust (NaN propagates as in `Math.max`).
  `Quests.points()` reads varp 101 and every nonzero varp is posted.
  `Equipment.unequip` and the `Equip` op use real host verbs.
- `liveCatalog()` items, `tradeable`, `displayName`/`clientName` and note links
  come from selected game data (game data gains `tradeable` and
  `stack_variant`). `ChatDialog.makeFromPanel`, `Shop.sell` with a pick
  predicate, `SolveClue.walkToBank`, `requiredThieving`, non-booth
  `Bank.openNearestAccess` and `Bank.openNpcAccess` are implemented.
- Declared-surface stubs throw `not impl` on use instead of returning fake
  values.
- A superseded `Game.teleport` resolves `false` in the tick it is superseded.
  v2 `api.tick` advances on every eligible tick, including while an async tick
  is pending. In the ResetSession window the v2 quest journal/status return
  `snapshot-unavailable`.
- `RunManager.override({ runAuto?, energyMin? })` supplies the matching host
  slot's per-session auto-run overlay. Missing fields fall through to host
  defaults (`runAuto: true`, `energyMin: 20`); Start or Stop clears the overlay.

### Gameplay fixes (live 289)

- Sherlock continues a giver dialog left open when a casket lands.
- Ardougne stalls: the frozen stealCakes loop runs in Rust; the combat lockout
  is waited out instead of counted as a refusal.
- Nav steps through `open_and_close_door2` doors (Tenzing's hut) instead of
  stalling the door hop; the host follow ends at the requested radius.
- A walk wait settles on a posted outcome that later snapshot deltas omit
  (wolf-pit recovery); `walkResilient` retries as frozen.
- DeathRecovery recovers only near its anchor (frozen `near`), and PeriodicBank
  returns with the bank open, as frozen does.

### Rendering and client

- Client `bc9b7f7`: side-step walk sequence names, zero-delay frames play for
  one cycle as in Java, GPU texture coordinates keep their sign across zero,
  GPU tests skip cleanly without an adapter, and clippy 1.98 cleanup.

### Navigation and packaging

- Navigation bakes are reproducible: the transport edges are put in a canonical
  order, so the same revision inputs produce byte-identical `.navpack`,
  `.navreach` and `.navcanlight` files on every run and platform. Previously
  door, ladder, stair, agility and teleport edges came out in hash-set or
  directory-listing order. The navigation source digest labels paths with `/`
  on every platform, so Windows records the same provenance.
- Release packages carry the local 289 navigation build; CI builds without
  navigation inputs (`BOT_NAV_BUILD=skip`). Internal campaign notes under
  `docs/compat` and `docs/harness-integration` are no longer tracked.

### Known limits

- Defence-1 `moss_giant`/`green_dragon`/`fire_giant` catalog cells cannot win
  their fights on 289 (each eat clears the attack); the prepared cells are the
  representatives. `moss_giant_bank`, `ardy_fighter_bank` and the earned-loot
  step of `rock_crab_bank` depend on the frozen Fight hold or drop RNG.
- Fence residuals for 0.1.9: `cake_stall.js` still pumps a Rust begin/next
  driver instead of one machine await; the v2 clue and quest-journal APIs are
  still begin/next; several in-isolate helpers still take untyped JSON
  arguments (no additional host wire).
- Lumbridge fountain banding on the GPU path needs a vertex-format change
  (0.1.9). Measurement-only performance claims are deferred to 0.1.9.

## [0.1.7] — 2026-09-16 — Alpha 2

### Server profiles and operator script controls

- Immutable per-process server profiles: `local-274`, `local-289`,
  `public-289` (`--profile` / `BOT_SERVER_PROFILE`) with revision-specific
  default engine roots, game/asset ports, vault paths and unpack dirs.
  `public-289` uses WSS/HTTPS on `w1.rs2b2t.com:443` with the baked public
  RSA. Conflicting profile/revision/prod combinations fail closed.
- Build-time nav bake/stage defaults to revision **289** next to the
  binary (`BOT_NAV_REVISION`, `BOT_NAV_BUILD=skip` opt-out); `nav-pack`
  remains for custom-input bakes.
- Content-addressed JS/TS transpile cache under `~/.274bot/js-cache`
  (raw-origin SHA-256). Per-profile script assignment and settings bags
  persist in the vault on successful Start.
- Native panel: manual **Reload**, catalog **Refresh catalog**, MultiBox
  **Start all / Stop all** (separate from Login all / Logout all). Reload
  confirm restarts matching running bots and Stops matching paused bots;
  unchanged origins skip transpile.
- Native ordered suite runner `e2e-suite` (profile/catalog selection,
  content-bound identity, process-tree ownership, capture contracts).
  Runner completion is not script qualification; visual captures remain
  `pending_visual_review` until readback. See [docs/e2e-suite.md](docs/e2e-suite.md).

### Release startup and panel fixes

- Drain buffered WSS messages during readiness checks and preserve payload reads
  across WebSocket control frames.
- Enable native clipboard paste into masked profile inputs.
- Fix Loadouts equipment-grid widget IDs and quantity editing; enable item
  search for the audited public-289 cache while rejecting unknown identities.

### Selected allocation, gameplay and portability corrections

- Reuse completed dynamic sprite slots; read animation delay without cloning
  transform data; box sparse appearance packets; share private animation bases
  while keeping public lookups independently owned.
- Release script snapshot storage on explicit Stop, preserving Pause and fresh
  restart keyframes. Release completed scenario snapshots after terminal evidence.
- Share the Play navigation world with harness seeds, including failed-load
  behavior. Allocate panel CPU upload storage only when a CPU frame needs it;
  retain pixels through GPU/CPU switching.
- Preserve routing worker ownership and stale-result rejection, retry failed
  radius requests without losing the prior route, and keep bank approaches
  radius-aware. Withdraw X now waits for published inventory; loadouts and food
  resolve through Rust helpers. Existing unsupported helpers remain errors.
- Correct adjacent door crossing and baked door edges; rebake existing v8 packs.
  Retain the bounded revision-274 stun-recovery inference and focused login
  priority across reconnects.
- Restore overlay, minimap-freeze and main-modal uploads. Add native Windows
  socket wakes, HOME/USERPROFILE selection and real process resource metrics.
- Retain the opt-in fleet harness for preservation work, with explicit fixtures,
  terminal evidence, TUI/panel integration and portable basic accounting. See
  [harness usage and limits](docs/harness.md).

These are selected engineering improvements, not a combined RSS or capacity
claim. Experimental snapshot sharing, borrowed fingerprints, tiled navigation,
and advanced campaign capture/controllers are excluded.

### TS shim (`crates/script`)

- `$RS2B0T` registry parse (static scan of `src/bot/scripts/index.ts`, no
  V8): listed scripts become Browse cards; the root persists to
  `~/.274bot/rs2b0t-path` after the first successful parse.
- TS transpile at Load (CompatClass shape), rs2b0t import remap, and a
  throw-on-missing Proxy for the listed API.
- Shim Game / Inventory / EventSignal from the posted snapshot;
  `Execution.delayUntil` parks the isolate on PLAYER_INFO; Banking.open
  and bank deposit / withdraw on the BankSide container.
- Live-script gaps closed: `Inventory.first` + held-item `interact`
  (`{op:'held', name, action}`), `reader.inventorySize()` mirroring the
  posted inv-tab slot count, `LoopingBot.log` / `settings`, the
  `paintLogic` `fmtDuration` module, and `Bank.setNoteMode` /
  `withdrawOp`. The live shim inventory read now targets the side-tab-3
  backpack exactly (a first-TYPE_INV scan grabbed a bank/trade widget).
- FlatBuffers isolate IPC: delta snapshots omit unchanged tables, and a
  hold tick re-posts so `EventSignal.pending` sees the held state.
- `EventSignal.pending` and `ignoredRandoms` surface from the bot
  instance.

### Guardian solvers (complete)

- The 0.1.2 stubs are gone: evade flee, plant pick, maze / mime /
  strange-box, hazard, lamp rub, and lost-gear / lost-tool solvers land
  as the complete act set.

### Nav and banking

- Pack `274V` version **8**: content-derived bank-stand table baked by
  `nav-pack` over the content tree. v7 files are `BadVersion` — rebake.
- Banking.open and the BankBudget session: deposit-withdraw-wear with
  **any-of** `worn_req`.

### TUI and panel

- `tui-play` script chrome is wired (Browse / Start / Pause / Stop /
  Load over the same JS library); a recording script's paint shows in
  the chat pane (`p` toggles back to the game ring).
- Panel paints ScriptPaint as ImGui over the Game chatbox — never on the
  client framebuffer.

### Live BoneBurier

- `script_bone_burier` live gold: the **real rs2b0t TS BoneBurier** runs
  through the shim on the driven slot. The host starts the `$RS2B0T`
  catalog card at live boot (`scenario.start_script`); the runner seeds
  the account (tutorial skip, five Bones given before the clean relog so
  the script's `onStart` gate opens once the inv tab binds) and then only
  watches for the server's "You bury the bones." chat line. PASS with a
  unique minted account; headed `panel-play --live script_bone_burier`
  and headless `tui-play --live script_bone_burier` pass the same
  runner.

### Public world docs

- `BOT_TARGET=prod` (alias `live`) / `host-play --prod` → `w1.rs2b2t.com:43594`
  with the baked public RSA; the local engine stays the default. Cargo
  `TARGET` is the rustc triple, not a world switch. Not Jagex, not a
  hosted wall, no w1 CI.

## [0.1.2] — 2026-09-01

- Rust **1.98.0** is pinned (`rust-toolchain.toml` + CI, host and client).

### Random-event guardian

- Detect-all + owner: every random kind this rev spawns is named in
  `RandomStatus`; `ours` is a hard NPC target (`target == self`) or an
  overhead-name match, no distance-grab.
- Dialog act: Talk-to + chat continue for genie, drunken dwarf,
  mysterious old man, sandwich lady, frog — no WalkTo, no fail-teleport.
- Hold while a dialog is in flight or the slot is trapped (maze / mime /
  box): script ticks **and** the nav route freeze, then resume latched;
  `on_random` knock (`RandomClaim::Host` default, no in-tree `Handle`);
  45 s wrong-talk cooldown per NPC slot.
- `ProfileSettings.random_events` (default on; off still detects and
  publishes), `lamp_skill`, `lamp_auto` — the lamp stays inert this tag.
- Panel status row binds the same `RandomStatus`.

### TUI operator panel

- New `crates/tui`: `tui-play` (ratatui + crossterm) is the headless
  second view of `host_play::Play` — same slots, raster Off, no GPU.
- Classic collision-dot map (basemap off) with town pins, You Are Here,
  and the remaining-walk polyline; Walk-confirm routes via `arm_walk_on`
  (lifted to host-play), WASD one-tile walks, chat / NPC dialogue
  Continue/Answer, status + `RandomStatus`, inv / stats / nearby locs,
  settings popup; script chrome is shape-only.
- `tui-play --live script_*` runs the same scenarios as `panel-play`;
  e2e unchanged. TestBackend tests in CI, no GitHub TTY.
- Map pane paints the packed collision (`Play`’s nav world), not the
  live loc list — a missing copy left `map (no nav pack)` while routes
  still ran.

### After 0.1.2 (v0.1.5)

- Evade flee, plant pick, maze / mime / strange-box solvers, lamp rub
  (`lamp_auto`).
- WalkTo a live NPC tile when Talk-to is out of range.
- Hitsplat window (`combat_cycle > loop_cycle`) as an extra ours signal.
- JS / TS shim `on_random` / `EventSignal.pending()`.
- Per-name ignore list (rs2b0t `setIgnoredRandoms`).

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
- Rust **1.97.1** is pinned (`rust-toolchain.toml` + CI). Bump in the
  0.1.2 session.

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
- Native `panel-play` (ImGui, panel-owned winit + wgpu loop): profile picker, Log in / Logout,
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
