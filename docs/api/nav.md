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

When `BOT_NAV_ENGINE_DIR` or `ENGINE_DIR` is set, legacy positional mode
defaults to `<engine>/../content/maps`, matching doors configs, and
`<engine>/data/pack/config`. Pass all three paths when using another tree.
Output goes to `$NAV_PACK` or `~/.274bot/274bot.navpack`. `gates.loc` is
derived from the maps dir's parent (`content/scripts/general_use/configs/gates.loc`).

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
Only bundled packs are rebaked automatically: an external pack
(`--nav-pack`, `NAV_PACK`, `~/.274bot/…`) keeps the content and membership gates
it was baked with. Rebake it after content changes or a format upgrade.

The pack serializes the whole-world `WorldCollision` (four planes, packed
9-bit walk per tile: `u8` face + `SQ_BLOCKED`, row-major z-then-x) plus
the derived `TransportGraph`. Magic `b"274V"`, version byte **15**. Each
edge's reusable `item_req` remains a held-item gate; v15 adds count-prefixed
`(id, count)` `consumed_req` and `item_returns` vectors after it. Resource
counts must be positive, and a returned item requires consumed resources.
Version 15 retains v11's selected quest-family binding — its
`quest_facts_sha256` and `quest_extractor_schema` — and typed per-edge
quest-stage gates; it keeps the content-derived bank-stand table after the
edges, per-edge `members_req`, a per-edge wilderness teleport cap, and
wilderness-level formula after the banks, the content-derived zone table, and
v13's approach geometry after each edge's quest gates. Geometry is tag `0`
by `width:u8`, `length:u8`, and `blocked_sides:u8` (a rotated four-bit mask).
Only footprint-backed Ladder/Stairs/AgilityShortcut/SpiritTree edges use it;
Door, NPC and teleport admission is unchanged. Zone data includes stable
kind identities/labels, NPC and hazard rows, curated groups, carves, and
shaped masks. A shape row stores a zone index u16, north extent u8, and
row-major u64 cell mask; the shaped NPC's `r` byte stores its east extent.
Thus shaped bounds up to 8×8 remain self-describing. Decoding any v15 pack
installs `Some(ZoneTable)`, even when its row counts are zero; legacy grids
and synthetic in-memory graphs use `zones: None`. The decoder rebuilds the zone
spatial index. Raw `u32` flags are not on the pack wire. The optional
`274F` sidecar holds them for collision paint; the paint-reach bitset is a
separate `274R` sidecar bound to the pack identity.
v14 introduced bit `0x80` in the existing edge-kind byte for player-relative
Ladder/Stairs landings, including supported gangplank Cross edges encoded as
Ladder; v15 retains this encoding. The remaining kind value keeps its kind.
A flagged edge stores the canonical loc-anchor-derived `to`; decoding recovers
`player_delta = to - at`. The flag is invalid on other kinds. Absolute
landings do not set it.
`decode` accepts version 15 only — v14 and older are `BadVersion` and must be
rebaked. The `274N` grid decoder (`decode_grid`) stays for old boolean-walk
files.

### Build-time selection, reuse and overrides

| Env | Default | Meaning |
| --- | --- | --- |
| `BOT_NAV_BUILD` | `require` | `skip`: no artifact baked/staged, checked-in identities only |
| `BOT_NAV_REVISION` | `289` | selected release revision (`274` supported) |
| `BOT_NAV_ENGINE_DIR`, `ENGINE_DIR` | none; required for bundled navigation | bake input root |
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

Headerless Jagex archives are decoded without copying their compressed payload
just to restore a header. The normal decoder uses the engine's 100k block size,
avoiding a 900k libbz2 workspace for each stream. Any block-data error (which
libbz2 also uses for CRC and table corruption) retries once with the larger
hint, so larger headerless blocks remain supported; truncated or corrupt
streams still fail, at most after two attempts. This does not change decoded
content identities.

New nav manifests, build stamps and compiled bundle rows carry `content_id` and
`source_sha256`. Nav manifest JSON also binds the decoded table's `zone_count`
(all zone rows) and `zone_npc_count` (NPC rows, excluding curated hazards).
Source provenance hashes the conservative content-tree closure and actual baker
config; the build stamp also binds generator source bytes. Legacy resources are
not relabeled: rebuild the application with the complete matching snapshot, or
use `nav-pack --revision 274|289 --content CONTENT_DIR --cache CACHE_DIR
--cache-manifest CACHE_MANIFEST --snapshot-root SNAPSHOT_ROOT --out NAV_PACK`.
The root contains the 16-hex version-keyed snapshot directory. Omitting
`--snapshot-root` creates an offline-only legacy pack; runtime binding rejects
it. Missing required decoded records fail rather than claiming Ready.

Local runtime nav verifies the selected content/config against the bake source
hash. Supported public profiles trust only compiled build/packager identities;
users of packaged public applications need no engine/content source checkout.
These identities attest the supported server-world assumption, not arbitrary
custom server behavior. Generated facts retain engine/content/decoder input
hashes and use decoded client identity; explicit endpoint overrides do not
inherit built-in facts. Local supported facts additionally verify their selected
source inputs. Unknown client content remains unavailable/rejected, never guessed.

The `tools/game-data` generator and verifier are offline `tsx` tools. Build
`cargo build -p nav --bin cache-content-id` first. `GAME_DATA_274_ENGINE`,
`GAME_DATA_274_CONTENT`, `GAME_DATA_289_ENGINE`, `GAME_DATA_289_CONTENT`,
`RS2B0T`, `GAME_DATA_274_SNAPSHOTS` and `GAME_DATA_289_SNAPSHOTS` are all
required and name explicit roots — an unset variable fails closed and names
itself, with no machine default. `GAME_DATA_IDENTITY_BIN` (or
`CARGO_TARGET_DIR`) still selects a non-default `cache-content-id` binary.
The Rust codec verifies
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
profile, not per bot. The bundled pack fast path keeps its cheap
revision/cache-identity check and reads + decodes the staged pack once
(`NavLoadCounters`). Both bundled and external packs expose the same verified
auxiliary sidecars when their identities are present in the selected manifest.
Paint reach is checked at preparation (magic/version, geometry, pack binding,
exact payload length and SHA-256), without retaining its world-scale words.
The first paint request decodes it once into a shared `Arc<[u64]>`, rechecking
the header and hashing the very bytes decoded before publishing them. The panel
binds only the lazy handle at profile install and requests words when the map's
reach layer or 3D collision/debug paint is enabled. The TUI requests words only
with the map open and its reach layer enabled. Routing and first walks never use
them. Closing either map releases its consumer lease; the profile caches the
process-level decode, so reopening shares it instead of adding another buffer.
`NavLoadCounters` includes both preparation and the lazy read/hash.
A failed first decode stays unavailable, not a runtime flood or an all-reached mask.

Canlight is decoded and SHA-256 verified during profile preparation, off the
client tick threads, so Fire queries can immediately crop the shared plane.
Bundled identities additionally check the pack+policy header binding; external
manifests pin the whole sidecar, including that header, to the selected pack and
cache. Legacy/minimal custom packs without sidecar identities keep reach or
Fire explicitly unavailable (legacy 3D paint alone may cache a runtime flood).
An advertised but missing, malformed or mismatched sidecar rejects preparation.
Changing a reach file between preparation and first use cannot publish new
words under the old identity. Already published words are owned and immutable,
so later file edits cannot affect them. The trust boundary remains the selected
local manifest/bundled identity, not authenticated publisher metadata.

Runtime pack decoding uses a bounded 64 KiB input buffer and fills the final
collision storage directly. Sidecar decoders fill final shared `Arc<[u64]>`
bitplanes in fixed-size chunks, without full-file staging or decoded-vector
copies. External pack SHA-256 covers the exact stream, including accepted
trailing bytes, before publishing the world; nav formats and routing semantics
are unchanged. The TUI `r` layer now tints walkable-but-unreached cells using
the same bound reach words as the panel and displays `reach unavailable` when
no verified words are available.

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

`TransportGraph { edges, at: HashMap<WorldTile, Vec<usize>>, approaches, teleports, … }`.
`derive_transports(content_root, loc_defs, collision)` parses the 2004 content: `doors/*.loc`
(openable doors), `gates.loc` fence/gate hops, `ladders+stairs/` and
`areas/` rs2 scripts (`p_telejump`/`p_teleport`/`~climb_ladder`,
`movecoord` landings), `skill_magic/` teleports, `skill_agility/`
shortcuts, spirit trees, Shilo↔Brimhaven cart NPC hops, Ardougne
wilderness levers, Al Kharid toll / Shantay **both ways**, essence-mine
wizard **entry and EssenceSession return**, Elkoy maze escorts, Zanaris
shed door with worn Dramen req, slashable webs (knife `oplocu` or worn
slash blade), gnome gliders, and boat NPC + gangplank. A `TransportEdge`
carries `kind` (Door/Ladder/Stairs/Boat/Teleport/AgilityShortcut/Glider/
SpiritTree/Npc), `at`/`to`, `loc_id`, the 1-based menu `option`
(`0` = use first `item_req` on the loc), `ticks`, reusable inventory
`item_req` gates, per-hop `consumed_req`, script-derived `item_returns`,
and `worn_req` (**any-of**). A held `item_req` is never budgeted as spent;
consumed counts budget supply across the full route, and returned items record
replacement after consumption (including charged-jewellery `next_obj_stage`).
Spell runes and script-deleted fares or passes are consumptive requirements.
Spell teleports have no fixed origin: they live on `TransportGraph::teleports`
unless `FindOptions::allow_teleports`. Wilderness tiles stay out unless
`FindOptions::allow_wilderness`. Both default **off**. Membership is the
packed `TransportGraph::wilderness` table derived at bake; a graph with
no zones (a legacy 274N grid) gates nothing. Packed spell and
jewellery teleports also carry a content-derived wilderness cap; `find`
will not take them from a tile whose packed `wilderness_level` exceeds
that cap. `find` also fail-closes on live `WorldState`.

The NPC Boat edge models `set_sail`; each eligible gangplank Cross remains a
separate player-relative Ladder edge. Its displacement includes the one-tile
`p_teleport` step and the two-tile `p_telejump` from the updated coordinate.
Only the two Entrana board locs are modeled in this slice. Locs 2081, 2083,
2085, and 2087 are omitted; their handler crosses before printing `board_message`.

Direct ladder/stair ops are also scanned across area and quest scripts.
Unconditional `p_telejump`, `p_teleport`, and the canonical `climb_ladder`
helper can derive fixed `movecoord(coord, dx, dlevel, dz)` landings, including
horizontal offsets such as the Mage Arena bank cellar. Presentation and
literal delays are allowed; branches, dialogs, queues, dynamic destinations,
and additional gameplay side effects are not flattened into ungated edges.
Existing specialized edges retain their requirements and measured prices.
New direct climbs price literal script/helper delays plus the interaction.

Footprint-backed loc transports use the same face/wall predicate as live
`api::query::loc_approach` interactions. Their rotated rectangle and blocked
approach sides are held in `TransportGraph::approaches`, aligned with ordinary
edges. The transport index includes each standable, operable takeoff rather
than only the loc anchor; forward routing, backward reach proofs, follower
approach selection and approach settlement share that geometry. A 2×2 stair
may legitimately be operable two tiles from its anchor, while a neighbouring
tile separated by a wall is not. This does not make occupied goals standable:
route to an operable adjacent stand for scenery such as the spinning wheel.
The shared sparse transport index reserves spare hash-table capacity when it is
rebuilt, keeping negative neighbour probes cheap without per-query allocation.

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

The corrected straight-door geometry uses generator version `nav-bake-2`;
it adds no door-specific wire fields. The v15 pack retains the v12
zone table described above and carries the resource-accounting vectors
described above. Generator and producer-source digests invalidate staged
bundles and trigger a normal rebake, with refreshed pack/reach/canlight/
navpois bindings. Explicit custom packs baked with the previous generator need
to be rebaked too.

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

Strict routing and the diagnosis arms share one inlined predicate for the
fixed requirements (membership, skills, completed quests and varps).
Relaxing carry/wear or an `Unknown` stage gate never relaxes those fixed
requirements; the hot-path inlining changes neither gate order nor routes.

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
or resource-state cap). `find` is CPU-heavy; run it off-pump (a short-lived
worker) and arm the result.

Consumptive hops debit a path's running inventory and credit script-proven
replacement items. Walking nodes share interned balance IDs, without a
per-node inventory allocation. A search tracks at most 64 resource IDs and
4096 balances; exceeding either bound fails with `BudgetExhausted`. A
conservative supply proof keeps the original tile-only search when resources
cannot constrain a simple path and no returned item must unlock a held gate.
The proof excludes consumables that cannot be carried initially or acquired
through usable hops' replacement items, so absent passes and unreachable charge
families do not force resource tracking.
Backward carried-gate proofs relax only when a returning hop is initially
usable; unreachable charge families do not widen those proofs.

The two Shilo↔Brimhaven carts use the native content's live fare:
`floor(carried_coins * 5 / 100)`, clamped to 10–200 coins. Their packed
consumptive requirement is the minimum 10, not a fixed 200-coin debit.
Route labels evaluate it against the balance after earlier payments;
missing-supply diagnostics invert that same rule across the remaining route.

Danger-zone permission is scoped to the route. Named `FindOptions.zones`
exemptions (and the explicit all-zone permission) apply to the whole walk.
An origin may move continuously within and escape its active zones, but
cannot re-enter them. Safe routes are tried first. Only when none exists
may a route enter and move within an active zone containing its selected
goal; it cannot then leave that zone to use it as transit. Inactive portions
of a level-rule zone do not grant permission across its active portions.
These rules also apply to transport and teleport landings and to each
candidate's own completion in first/all-target searches.

Refusal diagnosis requires a strict `NoPath` and an all-zone-exempt route
that still satisfies the same hard gates. It reports active zone-blocked
transitions on the source-reachable frontier, including separately blocked
alternatives, rather than naming only a shortest relaxed witness. It does
not claim a minimal cut; incomplete or budget-limited frontier searches
return no diagnosis.

`Traveller::follow` walks loc hops and fires packed OP_NPC, boats,
gliders, webs, EssenceSession, Shantay, Al Kharid toll dialogue, and teles.
NPC-backed hops use the live NPC tile (search radius 8). Paid Al Kharid
toll, Boat and Npc affirmative fare choices recheck current carried supply
before clicking. Glider landings settle Chebyshev 1. Spell buttons and charged
jewellery operations recheck live skill/item requirements before sending;
an unbound control waits within the hop budget and identifies the missing
control on expiry. Adult spirit-tree gate choices follow the live
“Where can I go?” label even when only one packed destination survives.
Agility waits packed `edge.ticks` after land. A teleport hop that never
lands (a server-refused wilderness cast) stalls after the hop budget;
the spell or rub is not resent.

Absolute vertical same-coordinate Ladder/Stairs hops retain the footprint of
the observed loc actually clicked. Arrival is on the destination plane within
that footprint expanded by the hop's one-tile landing tolerance, rather
than within a symmetric radius of the nominal origin. This accepts far-edge
landings from multi-tile stairs without accepting the opposite side, a
distant tile, or the old plane.

Content-derived player-relative Ladder/Stairs edges, including supported
gangplank Cross edges encoded as Ladder, carry `player_delta: Option<WorldTile>`.
Forward routing translates the actual admissible takeoff tile by that delta;
backward reachability proves the same takeoff-to-landing relation, rather
than using the loc anchor's nominal landing. A route leg's `edge.to` is its
planned landing, while `player_delta` is retained for live settlement.
The follower translates the exact tile from which it sent the op and uses the
caller's unchanged `close_enough`; reaching an unrelated nominal/planned
landing is not arrival. Absolute horizontal edges continue to settle at their
absolute `to`. No arrival tolerance is enlarged.

Radius walk goals use a Chebyshev margin. Plain tile goals keep the margin
centered on the requested tile, even when a loc or decoration occupies it.
A goal is loc-backed only when the caller explicitly supplies loc identity
(native `WalkRequest.loc_id`, as used by Gatherer and loc interaction recovery).
Compat `walkTo(x,z,r)`, Quester stand tiles and bank stands are plain tile goals.
Their packed endpoint filter and observed arrival share one reach rule: both
exact reach and open-wall adjacency to a solid target use a reach-work budget
of `(2 * radius + 1)^2`, the size of the requested goal region, rather than a
fixed step limit. Live probes read the current player's cached flood ranks;
packed probes borrow collision data with bounded, call-local scratch. This
adds no retained per-bot state and does not widen the Chebyshev margin or
weaken wall checks where collision is observable.
The planner clamps both the goal box and its reach-work radius to 104.
It classifies packed candidates in batches, sharing directed collision steps
and reach-rank bounds; shortest path distance alone is not the arrival rule.
Only undecided ranks need an ordered forward probe, preserving the shared
predicate even for asymmetric walls. Floods leaving the initial collision
window continue through lazily populated 32-by-32 dense pages shared by the
landmark bounds, batched wavefronts and ordered probes. Bounds also read
directed steps from unprepared window-boundary tiles, so another candidate
cannot hide an arrival route that hugs the boundary before leaving it.
Epoch-stamped marks reuse the exact-probe scratch; a cache edge is never
treated as collision. Page buffers and per-cell scratch reserve a shared
budget-derived allowance once per call, avoiding incremental reallocation
chains while keeping page population lazy and the cache extensible.
These structures avoid repeated per-tile world lookups during a flood and
add no retained per-bot state.
Failure diagnostics reuse the same computed goal list rather than filtering
it a second time.
For an identified loc in scene with a known footprint, distance is measured
to its full rotated footprint rectangle, not its south-west anchor. A legal
stand must fit that margin and pass the shared live wall/force-approach rule.
Off-scene or unknown loc footprints use the plain anchor-radius estimate;
they never flood connected solids. The same owned walk is re-planned when
its target enters the scene, its footprint becomes known, or a known
footprint vanishes before arrival—not on every tick at an unchanged
estimate. Live operability proves loc arrival while
the identified loc remains present. If it is absent or replaced in a ready,
loc-observed scene, the walk instead settles through the plain tile predicate
so its caller can reselect a target. Off-scene targets still defer.
Exact tile walks are unchanged.

Native callers accept only an `Arrived` receipt as arrival proof. A
correlated successful terminal route receipt is held while authoritative
local-actor motion is still moving; failed, blocked, evidence-required and
cancelled receipts surface promptly. Motion is carried explicitly through
the isolate snapshot; old snapshots report it unavailable, not stationary.
The existing active walk deadline bounds the wait, without relaxing reach.
`NeedsEvidence` preserves its typed quest gates through native walk adapters.
Quester Stops with a `needs-evidence` failure and retains those gates for its
caller through `unresolved_walk_gates()`.

Manual movement takes ownership before frontend or script follow. A matching
native walk completes once with the normal `WalkEnd::UserInput` receipt
(`blocked=None`), ahead of arrival, deadline and evidence checks. The slot
detail says “cancelled by user input” (not an action error) and clears when a
later walk is armed or another walk outcome is published. Cancellation
invalidates the old route generation and clears pending route/bank work and
cancelled carry.
Requests queued from an older observed walk-outcome sequence cannot restart that
owner, and live route-less walking composers stop through the shared intent
sequence. Already operator-paused or reconnect-carried script work keeps its
existing Resume behavior.

The shared navigation preference `nav.pause_script_on_manual_walk_abort`,
displayed as “Pause script on manual movement,” defaults to ON. It gates only
the pause of the script that owns the cancelled walk; it never gates intent
detection, cancellation, or the posted outcome. With it OFF, cancellation
still occurs and scripts remain able to observe their normal end state,
including compat `false` and the v2 `user-input` reason. A native script may
make a new decision after an explicit Resume, but the cancelled request, host
carry, queued pre-takeover walking work, and watchdog recovery are not replayed.
For compat walks, watchdog-owned replacement routes retain the original
request, key and generation for this terminal; their internal zero id does not
replace the caller's receipt. Native-owned recovery ends its typed walk to its
owner instead of carrying a receipt-only compat identity. Takeover also
restamps gameplay progress, so leaving pause OFF does not immediately trigger
a stale WedgeWalk recovery.

The detector classifies movement intent, not successful displacement. Accepted
conservative positives include a world-menu Examine/Cancel selection and
unwalkable or same-path clicks, even when no movement follows. Inventory,
chat, main-modal widgets, and unrelated keyboard input are excluded. The
documented residue remains: a rare pointer jump from a hovered world-menu
action to side chrome can invoke the last-drawn default action without passing
the world-pointer classifier; spellbook/jewellery teleports and dialogue travel
are also outside this takeover rule, and standalone client-window input is not
claimed.

Quester treats a refused native walk as the owning step's terminal. A
`Failed`, `Blocked`, or `Refused` walk receipt sets that step to `Blocked`;
missing quest evidence blocks it with the required gates in the reason. This
applies to explicit walk steps and approach walks inside talk, interact,
use-on, bank, buy, and make steps. The step does not silently queue the same
route again or advance to its interaction. A terminal blocked Quester run
requires an explicit Start; **Read Journal** cannot revive the old run. The
host and native action still publish one correlated receipt for the original
request.

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
  Escalating a slammed door to the troll strategy retains the elapsed hop
  budget; an approach escalation retains its elapsed wait too. Losing the
  connection reports `Dropped` only if the attempt made no tile progress;
  otherwise the exhausted hop is `Expired`.
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
fail-closed: when only held, consumed, or worn item requirements are missing,
the host plans a fetch-and-wear session from the packed bank stand table
(`NavWorld::banks()`), walks to a standable access tile of the nearest
stand (never the booth loc or a teller spawn behind the counter), opens
it, withdraws only shortages, closes the bank, then wears any missing worn
item (the client cannot wear while the bank is open), and re-runs the strict
search. It never deposits backpack items: tools, quest items, food, and existing
route stacks remain carried, including for compatibility-v1 walks, which enable
fetch by default. Consumed resources use the full route's running budget
(two 30-coin fares need 60); reusable held gates use their peak requirement.
An item needed both carried and worn reserves a separate copy for wearing.
Exact withdrawals use fixed bank amounts or Withdraw-X when needed.
Access tiles come from the one rule
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

## Manual-click LIVE regression harness

The ignored TUI tests exercise the production `TuiApp::on_key` → `dispatch`
path at 120×40 and 80×24, including the reachable persisted settings row.
The panel tests inject through production `Session::capture_tx`, preserving
the client's normal input path and CPU game-pane capture. The matrix covers
pause OFF and ON, watchdog-owned recovery followed by manual takeover and
Resume, a v2 cancellation consumer, group-member isolation, and route-less
resilient walking with pause OFF and with pause ON followed by Resume. Fresh
NPC and loc trials use the client's projected actor
bounds and actual opened world-menu rows, not fabricated picks or stale hover
rows. World-menu Cancel and Examine rows conservatively cancel too. Harmless
tab and right-click controls must not cancel. The group trial records a
no-click control before takeover and requires the companion's active arm,
generation, destination and aim to remain unchanged, with no cancellation
receipt; this isolation check does not infer companion displacement.

The F2 regression separately pauses an armed compat walk, keeps pumping
until the paused manual click is observed, then resumes and verifies that
the uncancelled carry completes. The panel fixture chooses a pure walk leg
of at most 48 tiles so nearby destinations with long castle detours cannot
consume the unchanged 60-second script deadline. Its second walk clicks
immediately after the production Resume call and requires exactly one
correlated `UserInput` receipt for the carried request. These tests reject
missing LIVE configuration rather than silently passing.

Supply a disposable `HOME` (you own throwaway isolation) holding a writable
copy of a decoded R289 cache snapshot at `$HOME/.274bot/unpack-289`.
`BOT_MANUAL_CLICK_CACHE_SOURCE` must point to a read-only source snapshot,
not to an operator-writable application directory. The tests require that
copied cache, plus the evidence, engine and nav-pack paths below, before
opening a vault. Run from the repository root with the local
R289 engine and an already-built R289 nav pack:

```bash
set -euo pipefail
: "${MANUAL_LIVE_EVIDENCE:?set to a writable evidence directory outside the source tree}"
mkdir -p "$MANUAL_LIVE_EVIDENCE"
LIVE_HOME="$(mktemp -d)"
trap 'rm -rf "$LIVE_HOME"' EXIT
mkdir -p "$LIVE_HOME/.274bot/unpack-289" "$LIVE_HOME/tmp"
: "${BOT_MANUAL_CLICK_CACHE_SOURCE:?set this to a read-only decoded R289 cache snapshot}"
cp -R "$BOT_MANUAL_CLICK_CACHE_SOURCE"/. "$LIVE_HOME/.274bot/unpack-289/"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME RUSTUP_HOME
export TMPDIR="$LIVE_HOME/tmp"
export LIVE=1
export MANUAL_LIVE_EVIDENCE
export WORLD_ENGINE_DIR="${WORLD_ENGINE_DIR:?set to the local R289 engine directory}"
export WORLD_NAV_PACK="${WORLD_NAV_PACK:?set to the built R289 nav pack}"
export BOT_NAV_BUILD=skip
export BOT_CPU=1
export BOT_LIVE_NAME_PREFIX="${BOT_LIVE_NAME_PREFIX:-mc}"

HOME="$LIVE_HOME" cargo test -p tui --lib manual_click_live_tests -- --ignored --nocapture --test-threads=1
HOME="$LIVE_HOME" cargo test -p panel --lib manual_click_live_tests -- --ignored --nocapture --test-threads=1
```

Each case writes a directory under `MANUAL_LIVE_EVIDENCE` containing the
surface capture (`.txt` for TUI, raw CPU `.argb` for panel) and a matching
JSON receipt with the host probe state. The TUI receipt also stores the
rendered cell buffer. Keep the evidence outside the source tree; remove the
throwaway HOME only after preserving the captures.

Movement-packet proof is separate from the live takeover receipts. Run
`cargo test -p client --test ground_input_composed cpu_ground_input_composed -- --nocapture`
for the CPU draw → human down → deferred terrain pick → `MOVE_GAMECLICK`
path, including menu selection and blocked terrain; run
`cargo test -p client --test walk -- --nocapture` for minimap packets and
open-world-menu dismissal followed by a minimap walk. The panel harness
captures the live CPU game pane, not native window chrome.

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
