# Nav: whole-world routing, transports, and WalkTo

`crates/nav` is the single nav stack: a whole-world collision bake, a
content-derived transport graph, a Dijkstra router, and a pollable route
follower. It acts only through the kernel API (`api::interact`,
`api::settle`) and never deep-copies the world.

## Pack bake

Application builds bake the pack themselves. `crates/host-play/build.rs` runs
the shared baker (`nav::bake::bake_world`) for the selected release revision,
stages pack + flags + reach + sidecar manifest + stamp under the install resource root
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
cargo run -p nav --bin nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR] [CONFIG_JAG]
# or: nav-pack --revision 274|289 --content CONTENT_DIR --cache CACHE_DIR …
```

Legacy positional defaults are `$HOME/experiments/Server/content/maps` and
siblings. Pass the three paths if yours lives elsewhere. Output goes to
`$NAV_PACK` or `~/.274bot/274bot.navpack`. `gates.loc` is derived from the
maps dir's parent (`content/scripts/general_use/configs/gates.loc`).

Each edge's `members_req` comes from the content handler that edge's op runs,
resolved as the engine does: type `[<op>,<name>]`, then category
`[<op>,_<category>]`, then global `[<op>,_]`. The edge needs a members world
when that handler's leading path (its first statement, followed through
unconditional leading `@label` jumps) is the F2P refusal
`if (map_members = ^false …) { …; return; }`. Members spells come from
`magic_spells.dbrow` `data=members,true`. The bake fails if a free edge's
handler, or any label, choice or queue it may continue into, reads
`map_members`, unless that exact read is a listed non-gate branch. It also
fails if a members-only edge's leading path does not refuse F2P, unless its
handler carries a listed members arm (the glider, Zanaris and spirit-tree
gates).
Only bundled packs are rebaked automatically: an external v11 pack
(`--nav-pack`, `NAV_PACK`, `~/.274bot/…`) keeps the membership gates it was
baked with, so rebake one made by an earlier 274bot.

The pack serializes the whole-world `WorldCollision` (four planes, packed
9-bit walk per tile: `u8` face + `SQ_BLOCKED`, row-major z-then-x) plus
the derived `TransportGraph`. Magic `b"274V"`, version byte **11** (v11 binds
the selected quest family — its `quest_facts_sha256` and
`quest_extractor_schema` — right after the version byte and carries typed
per-edge quest-stage gates; it keeps the content-derived bank-stand table after
the edges, per-edge `members_req`, a per-edge wilderness teleport cap, and
the wilderness-level formula after the banks; raw `u32` flags are not on
the pack wire, the optional `274F` sidecar holds them for collision paint;
the paint-reach bitset is a separate `274R` sidecar bound to the pack
identity). `decode` accepts version 11 only — v10 and older are `BadVersion`
and must be rebaked. The `274N` grid
decoder (`decode_grid`) stays for old boolean-walk files.

### Build-time selection, reuse and overrides

| Env | Default | Meaning |
| --- | --- | --- |
| `BOT_NAV_BUILD` | `require` | `skip`: no artifact baked/staged, checked-in identities only |
| `BOT_NAV_REVISION` | `289` | selected release revision (`274` supported) |
| `BOT_NAV_ENGINE_DIR`, `ENGINE_DIR` | revision's canonical engine | bake input root |
| `BOT_NAV_CONTENT_DIR` | `<engine>/../content` | canonical content tree |
| `BOT_CACHE_MANIFEST` | checked-in known cache identities | verified cache manifest |
| `BOT_NAV_RESOURCE_DIR` | cargo profile dir / bundle Resources | staging root |
| `BOT_NAV_SNAPSHOT_ROOT` | `~/.274bot/unpack[-289]` | complete version-keyed decoded snapshot, read-only bake input |

### Decoded-identity migration

Normal profile preparation negotiates the selected `/crc` before freezing an
owned cache/snapshot shared by all bots using that profile. Transfer CRCs and
packed hashes remain exact; compatibility uses revision-bound `274DCI01` decoded
identity. Offline `ProfileSelection::bind()` retains legacy packed binding;
application callers use `prepare_template()` or `bind_runtime()`.

New nav manifests, build stamps and compiled bundle rows carry `content_id` and
`source_sha256`. Source provenance hashes the conservative content-tree closure
and actual baker config; the build stamp also binds generator source bytes.
Legacy resources are not relabeled: rebuild the application with the complete
matching snapshot, or use `nav-pack --revision 274|289 --content CONTENT_DIR
--cache CACHE_DIR --cache-manifest CACHE_MANIFEST --snapshot-root SNAPSHOT_ROOT
--out NAV_PACK`. The root contains the 16-hex version-keyed snapshot directory.
Omitting `--snapshot-root` creates an offline-only legacy pack; runtime binding
rejects it. Missing required decoded records fail rather than claiming Ready.

Local runtime nav verifies the selected content/config against the bake source
hash. Supported public profiles trust only compiled build/packager identities;
users of packaged public applications need no engine/content source checkout.
These identities attest the supported server-world assumption, not arbitrary
custom server behavior. Generated facts retain engine/content/decoder input
hashes and use decoded client identity; explicit endpoint overrides do not
inherit built-in facts. Local supported facts additionally verify their selected
source inputs. Unknown client content remains unavailable/rejected, never guessed.

The `tools/game-data` generator and verifier are offline `tsx` tools. Build
`cargo build -p nav --bin cache-content-id` first; set `GAME_DATA_IDENTITY_BIN`
(or `CARGO_TARGET_DIR`) and `GAME_DATA_274_SNAPSHOTS` /
`GAME_DATA_289_SNAPSHOTS` when overriding their defaults. The Rust codec verifies
actual pinned assets before generation. This does not relax e2e exact resume
provenance. Neither build nor runtime accepts a persistent decoded-identity
sidecar solely because input sizes match.

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
records the cache identity, the pack/flags/reach/canlight/navpois digests, the
generator identity (the manual id plus the bytes of `bake.rs`/`collision.rs`/
`pack.rs`/`paint.rs`/`quest_gates.rs`/`router.rs`/`transport.rs` and the
transport producers), the navpois generator
identity (`map/services.rs`/`map/poi.rs`) and a fingerprint (size + mtime) of every canonical input
(content tree, config jag, cache archives). Any change to those inputs, to the
pack format identity (`nav::pack::FORMAT_ID`), to the generator, to the cache
identity, a missing navpois sidecar, or a missing/replaced staged artifact (including the reach sidecar) rebakes.
Build preparation additionally computes source and decoded digests to detect
same-size replacement; runtime computes decoded identity once per prepared
profile, not per bot. The bundled fast path keeps its cheap
revision/cache-identity check, reads + decodes the staged pack once
(`NavLoadCounters`), and loads the bound reach sidecar with cheap geometry/binding
checks and zero `bake_reach` calls, while `--nav-pack` / `NAV_PACK` / `--nav-flags` overrides
keep the external path (including its one-time reach flood) and a differing cache identity falls back to it.
Runtime pack decoding uses a bounded 64 KiB input buffer and fills the final
collision storage directly. Bound reach and canlight sidecars decode into their
final shared `Arc<[u64]>` bitplanes in fixed-size chunks, without full-file staging
buffers or intermediate decoded-vector copies. External pack SHA-256 is computed
over the exact stream, including accepted trailing bytes, before publishing the
world; the nav formats and collision/transport semantics are unchanged.

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
`FindOptions::allow_wilderness`. Both default **off**. Membership is the
packed `TransportGraph::wilderness` table derived at bake; a graph with
no zones (a legacy 274N grid) gates nothing. Packed spell and
jewellery teleports also carry a content-derived wilderness cap; `find`
will not take them from a tile whose packed `wilderness_level` exceeds
that cap. `find` also fail-closes on live `WorldState`.

Straight wall doors join the loc's own tile (`at`) and the adjacent tile
along its placement angle. Opening removes the closed wall between those
two tiles; the reverse crossing lands back on `at`, not on a second tile
behind the loc. Both landings must be standable, so this reconnects enclosed
shop floors without jumping over scenery or snapping a standable origin.
The follower requires crossing the wall before completing a hop, including
arrival on `at` for the reverse. That holds for the exact arrival production
walks use (`close_enough` 0). With a looser `close_enough`, a corner tile on
the approach side of the loc's row or column can count as crossed before the
door opens, so callers must not relax it for door hops. Diagonal doors keep
their separate content-derived geometry.

The corrected straight-door geometry uses generator version `nav-bake-2`.
It changes no wire fields: the pack format stays `274V11`. The generator
and producer-source digests invalidate staged bundles and trigger a normal
rebake, with refreshed pack/reach/canlight/navpois bindings. Explicit custom
packs baked with the previous generator need to be rebaked too.

### Quest-stage gates (`nav::quest_gates`)

An edge that a quest stage opens carries `quest_gates: Option<QuestGates>`:
typed `api::selected::QuestGate`s — a completed quest, or a closed
`StageWindow` over a selected progress signal (`[n,n]` for one stage,
`[None,n]` for "up to n") — bound to the quest family the pack consumed
(`TransportGraph::quest_family`). A stage is never faked as a minimum varp
threshold. Only resolved progress evidence decides a gate:
`WorldState::with_quest_evidence(QuestEvidence::new(provider, family,
required_after))` attaches the caller's immutable `EvidenceProvider` snapshot,
the family it was prepared from and its causal freshness floor. Snapshot varps
and green quest rows never open a stage gate, and nav reads no journal.

`WorldState::quest_gates(edge)` is three-valued. `True` (every value still
possible lies inside every window) alone lets `find`/`find_with` take the edge.
`False` (disjoint) and `Unknown` (no, stale or other-family evidence, or values
straddling a bound) never do, however much the edge would save. After a strict
search fails, `router::find_unresolved_quest_gates` searches again crossing
only `Unknown` gates and returns the ones its route needs, as values — the
caller reads those journals, refreshes its provider and searches again. It
returns `Ok(None)` when no evidence could help (for example a disproven window)
and `Err(QuestFamilyMismatch)` for evidence from another quest family. The
routing searches evaluate gates without allocating; only the failure diagnosis
collects them.

Route selection is not permission to send later. `TravelOptions::quest_evidence`
borrows the evidence for the active route. The traveller checks a gated
transport leg at entry and again on each approach poll, including the poll
after the approach walk settles and before its interaction. Once the
interaction is sent the crossing is not rechecked. Only `True` permits
continuation; missing, disproven, stale or undecided evidence ends the leg as
`TravelOutcome::EvidenceUnproven { at, leg, verdict, unresolved }` without a
transport send. `verdict` is `False` or `Unknown`; `unresolved` names the gates
the evidence leaves `Unknown` (every gate without evidence, none for evidence
from another quest family), so a caller can acquire them and retry. Ungated
legs need no provider.

The two diagnoses see across each other. When the BankBudget diagnosis
(`find_missing_item_reqs`, which ignores only carry/wear gates) finds no route,
it searches once more also crossing `Unknown` stage gates, and
`find_unresolved_quest_gates` likewise falls back to also ignoring carry/wear
gates. Each still reports only its own kind, so a door that needs a carried
rope and an undecided stage names the rope in one and the stage window in the
other; neither hides behind the other's gate. A `False` gate closes both.

`QuestFamilyId::new(quest_facts_sha256, NonZeroU16)` is the only way to build
a family: extractor schema 0 is malformed in a pack and cannot be constructed,
so `encode` never writes a binding its decoder refuses. Runtime admission
compares the pack with the selected manifest:
`TransportGraph::admit_quest_family(&manifest_family)` refuses a pack baked
against another family digest or extractor schema, or against none. The
current bake consumes no quest family yet, so it binds none and emits no stage
gates; exact and upper-bound stage crossings stay unavailable until the
selected quest family feeds the bake.

## Router (`nav::router`)

`find(collision, graph, from, to) -> Result<Route, RouteError>` is Dijkstra
with safe defaults (no wilderness, no any-tile teleports).
`find_with(..., FindOptions { allow_teleports, allow_wilderness })` opts
those in. Tile steps use the client's directional `PL_WALK_*` masks,
**not** the blanket `walkable()`. Transport take-off is any standable
tile within **`INTERACT_RADIUS` 1** of the edge `at` (adjacent only — a
radius of 3 let cow-pen routes “use” the north-west road gate through a
fence). Any-tile teleports are refused when the takeoff tile's wilderness
level exceeds the edge's packed cap (the content
`~wilderness_level(coord) > N` gate). `Route { legs, dest, ticks }`;
`Leg::Walk { tiles }` runs collapse, `Leg::Transport { edge }` is one per
transport. `RouteError` is `NoPath` or `BudgetExhausted` (a node-expansion
cap). `find` is CPU-heavy; run it off-pump (a short-lived worker) and arm
the result.

`Traveller::follow` walks loc hops and fires packed OP_NPC, boats,
gliders, webs, EssenceSession, Shantay, Al Kharid toll dialogue, and teles.
NPC-backed hops use the live NPC tile (search radius 8). A paid Al Kharid
toll Door hop returns `Blocked` when the packed `item_req` is short at the
gate. Glider landings settle Chebyshev 1.
Agility waits packed `edge.ticks` after land. A teleport hop that never
lands (a server-refused wilderness cast) stalls after the hop budget;
the spell or rub is not resent.

Vertical same-coordinate Ladder/Stairs hops retain the footprint of the
observed loc actually clicked. Arrival is on the destination plane within
that footprint expanded by the hop's one-tile landing tolerance, rather
than within a symmetric radius of the nominal origin. This accepts far-edge
landings from multi-tile stairs without accepting the opposite side, a
distant tile, or the old plane. Other transport arrival rules are unchanged.

## Traveller (`nav::traveller`)

`Traveller::follow(client, snapshot, route, &mut options)` is **pollable**:
call it once per delivered server tick; it returns `None` while in
progress and `Some(TravelOutcome)` at a terminal state
(`Arrived`/`Stalled`/`Refused`/`Blocked`/`EvidenceUnproven`/`GaveUp`). One driver send per
call. `TravelOptions { close_enough, budget_ticks_per_hop, max_hops,
on_leg, troll_doors }`.

- **Stalled walk recovery:** five distinct game ticks without tile progress
  or actor movement reissue the current aim, even if the map flag remains.
  A delayed action or queued modal can block successive accepted clicks;
  each reissue starts a fresh idle window, not a new hop budget. Movement
  resets the idle window, duplicate observations do not advance it, and
  recovery does not consume another `max_hops` slot.
- **`Stalled { why: EndBlocked }`:** no walk of the follow was accepted,
  the player stands within one tile of the route's last tile on its level,
  and the client refused the click onto it on five distinct ticks (frozen
  `'blocked'`, `WalkExecutor.ts:1039-1094`). The player is as close as the
  live scene allows. Script walks publish it as a settled route end flagged
  blocked (`walk_outcome_blocked`), and `walkResilient` returns true on it;
  other callers end the follow as for any stall.
- **Scene settle:** a transport's approach click the client refuses
  (unreachable, off scene, scene unavailable) is retried two ticks later,
  three times per follow, before the follow ends `Refused` — a region
  rebuild briefly empties the client's local route (frozen
  `CANDIDATE_SETTLE_TRIES`, `WalkExecutor.ts:1178-1184`).
- **NPC-backed Boat/Npc/Glider recovery:** choose an operable stand by live
  scene-route cost, re-picking reachable same-type NPCs and retargeting a
  wandering NPC before the old approach settles. The shared stand predicate
  requires a cardinal shared edge without a wall and excludes the NPC's
  footprint. A diagonal stand is tried only when no cardinal stand exists; it is
  a bounded probe, not engine parity (the engine's NPC reach has no diagonal
  case), so it usually ends in the retry limit rather than a ride.
  A fresh server reach failure permits at most three actual interactions in
  one leg budget, including approach and fare dialogue. Retry watermarks
  advance after each send, failed stands/instances lose priority, and an
  open fare dialogue is never cancelled by re-clicking the NPC. Missing or
  changed tracked targets and unrecognized dialogue choices produce an
  explanatory terminal `Blocked` receipt; attempt exhaustion names its
  actual attempt count. Duplicate snapshot polls do not spend this budget.

- **Default door leg:** interact the door transport's menu option, then
  settle `arrived(to, close_enough)` — cheap, no per-tick door polling.
- **`troll_doors = true` (non-default, expensive):** per tick, read the
  door's open/closed state from the snapshot's `locs()`; when the door
  reads open, `op_loc` (re-open) and `walk` through in the **same tick**
  so a tick-perfect closer cannot slam it (the `2026-08-22-bot-nav.md`
  same-tick rule). Use only for the live door-troll fixture; ordinary
  routes pay the cheap default.

## Route inspect (preview)

Pure preview is a separate host job from walking. It never arms Traveller,
never latches a bank session, and never changes ordinary walk policies.

- **v1** `Navigator.findPath(from, to, opts)`: wilderness on, bank-fetch
  off, teleports only from explicit catalog/policy bits. Default waiter
  timeout is 20000 ms (Brimhaven passes 8000). Returned hops include
  `locName`; `expanded` is omitted.
- **v2** `api.inspectBegin({ from, to, allow_* , avoid })` returns an
  isolate token. Query `inspectSettled` / `inspectValue`, or observe
  `snapshot.route_inspect_*`. `api.request({ op: 'inspect-route', from, to,
  request_id })` with `request_id: 0` is snapshot-only: the host still
  runs a real preview into `route_inspect_*` and does not create a
  waiter. Caller-invented nonzero ids are isolate-stale and never reach
  the host job queue. Token identity is the isolate waiter; admission
  identity is host unobserved retention plus the running/pending
  reservation. Begin arguments are registered in Rust; a later
  `inspect-route` with mismatched opts is `invalid-args` and is not
  queued.
- **Bound:** isolate unsettled waiters max 3. Host storage is a 2-deep
  published ring plus one held slot (CAPACITY=3). Admission counts only
  unobserved terminals (`seq > observed_seq`, plus `held`) plus the
  executing job, a pending-replace publish, and the new request.
  Observed history is not capacity. A registered token that fails
  `can_admit` is not accepted: its identity is posted in
  `route_inspect_refused_id{,_2,_3}` (3-deep mailbox, oldest shifts)
  and the isolate settles that waiter `stale`. The mailbox does not
  overwrite unobserved ring or held terminals. Last-seen
  `route_inspect_unobserved` may local-stale begin/authorize when it
  is already 3; that count can lag the next host drain, so authorize
  is not a reservation. Snapshot-only `request_id` 0 uses the same
  admit budget, never occupies the refuse mailbox, and leaves the
  previous published latest when overload refuses a new preview.
  Continuous id0 does not disable registered traffic; ACK progress
  admits either. Held flushes only from typed isolate `inspect-ack`
  after the snapshot is applied, carrying that terminal's generation.
  Snapshot send is not observation. Old-session ACK cannot free a new
  generation. ACK is sent when the isolate applies a terminal even if
  no later inspect request occurs.
- Conditional `allow_bank_fetch` preview labels `bank_planned` only after
  a PRE-state stand proof (or wear-only). Published hops are the post-state
  from→to transports, never bank steps or Traveller actions.

## BankBudget (`nav::bank_fetch`)

WalkTo and script `walk_with` can opt in with `FindOptions::allow_bank_fetch`
(the panel/TUI **bank fetch** checkbox, default off). `find` itself stays
fail-closed: when the only missing gates are `item_req` / `worn_req`, the
host plans a fetch-and-wear session from the packed bank stand table
(`NavWorld::banks()`), walks to a standable access tile of the nearest
stand (never the booth loc or a teller spawn behind the counter), opens
it, deposits the backpack, withdraws what the route needs, closes the
bank, then wears any worn item (the client cannot wear while the bank is
open), and re-runs the strict search. Access tiles come from the one rule
the named-bank stands script bank walks use (`nav::named_banks`):
standable and never the stand tile itself. BankBudget takes the tiles in
line with the stand (north, south, east or west), not the diagonals the
named stands allow, because a teller across a booth answers only in line,
and never a tile across a wall from the stand (a banker behind the bank's
outer wall does not answer from the street).

Packed stands are content-derived booths **and** NPC tellers
(`category=bank_teller`). Open uses that packed access — a teller stand
sends the banker op (and a dialog choice when the pack carries one), and
falls back to the same bank's booths, nearest the player first, when no
teller answers. Each step then waits until the live snapshot shows it
landed: the backpack as the bank's side panel shows it while the bank is
open, the inventory tab once it is closed. A sent withdraw or wear keeps
waiting while the item has left the bank or the backpack but not yet
shown up in the backpack or the worn set. Every step has a bounded wait
(32 pumps, one per player tick) and ends the session with a logged reason
if it never lands, and a send already in flight is not repeated. The
script walk and the panel/TUI WalkTo share one step machine, one wait
budget and one in-flight latch.

**Closed bank:** the session is planned from the **open** bank's rows
(`snap.bank()`). A closed bank contributes `[]`, so BankBudget cannot prove
that a banked item exists and reports `NoPath`. That is intentional — there
is no closed-bank inventory cache. Open the bank (or keep it open) before
confirming a fetch walk if the needed item is only in the bank.

## WalkTo picker

The panel's main-chrome **WalkTo** button fills the Game pane
(`crates/panel/src/picker.rs` + `walk_map.rs`). One application-owned
renderer draws at most 24 terrain tiles (258×258, A's `select_lod`) plus
one viewport overlay for optional map-owned grid/collision/NSEW/reach/flood
layers — not a per-tile ImGui quad mesh. Until D binds an image cache the
map shows a grid and `map imagery unavailable — cache not bound` with no
POIs. Route and destination are vector markers. Reach uses bound
`.navreach` or `reach unavailable`. Wheel zooms toward the cursor; click
selects through
`host_play::walk_map::MapModel` (radius-16 walkable query, blocked clicks
stay view-only). Footer **Recentre** / **Walk** / **Send** consume that
pending destination once via `Play::map_walk` (the bot focused at confirm,
or a group of eligible wall bots); a missing origin or a running script
refuses instead of storing a later login dest. Local engines also get
**Teleport** (`Play::map_teleport`, loopback-guarded, focused-only). Close/hide
unregisters textures and drops CPU pixels. `BOT_CPU=1` still uses this
map path (panel UI GPU). `walk_status_text` mirrors the armed dest and
clears on any terminal outcome. Place names keep `/` as a stored world-map
line break; lists and canvas labels show a space. Canvas labels skip
overlapping text (markers still draw; at most 32 labels).

A present but non-standable origin refuses with `OriginNotStandable`
("observed player tile is not standable"), without queuing a route or
snapping across a wall. `NoOrigin` is reserved for a missing observed player;
a present invalid origin plane uses `InvalidCoordinates`.

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
