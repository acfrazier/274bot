# Bounded reachability audit after resource2d589

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: bounded architecture/fidelity investigation for
brief 138. Audit only. Not implementation, LIVE, fixtures, foreign catalog
edits, or a clone of `localReach.ts`. No routine child review.

Read once: `AGENTS.md`, `docs/execution.md`, brief 138, design 04d,
implementation 04d, resource audit 06ab, fixture note 05zb. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Work was read-only except
this report and `docs/compat/evidence/bounded-reachability-audit/`. Existing
workers own `load.rs`, scenario files, and `flax_aio_pick_failure_facts`.
Those files were not edited.

## Verdict

**The frozen Flax return stall is control-flow compatible with unbounded
native flood bits being treated as `Reachability.canReach(..., {adjacentOk:true,
maxSteps:400})`. That is a suspected cause, not a live proof that a bounded
native query would make `flax_aio_pick` pass.**

Raw 2d589 cells pick, walk to Seers, open booth 2213, deposit (FAIL inv empty),
then never `WalkTo` again. Bank stays open. In-scope field flax at Chebyshev
41–55 is posted `reachable_adj=true` from `flood_reach()` (no expansion cap).
The shim ignores `maxSteps`. Foreign `canReachLocal` caps BFS *expansions*
(not path length) at default 400. `GoToFieldTask` runs only when
`nearestFlax()===null`. `bankRun` never closes. After deposit, pick-only
`needsBank` is false and `atField` is false, so the only return path is
GoToField. Unbounded `nearestFlax` success explains a silent stall at the
open bank without changing original task policy and without injecting
`Bank.close`.

A later bounded native query *could* address that mismatch. It is subject
to implementation, review, and live qualification. This audit does not
claim a 100 percent fix from static reasoning. The FAIL snapshot did not
include collision flags, so Seers walkable density is not replayed here.

Do not dim FlaxAIO. Do not edit frozen scripts. Do not add a close-planner.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `ac3dd3ada64a5a336091dfca5de08653383a7aaf` |
| LIVE host (resource fixture 125) | `2d589c7bacfef32140060e663407b3c364000579` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 138 SHA-256 | `e0b288aafa2674b78ce02043e8625bfe0896f140bcdb49be2c961aadd74076e3` |
| Kanban card | `t_9f605084` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| picking.ts / flaxaio.ts / banking.ts / localReach.ts / Reachability.ts | byte-identical across both catalogs (hashes in evidence `refs.json`) |
| Shim `reachability.js` | `ee4127f41dc18b3a8cdc773b962733e1b41afa8eca21319d6ef6479a711b230c` |
| r289 live log | `docs/compat/evidence/catalog-harness/live/r289-flax-aio-pick-100adccc-resource2d589.log` sha256 `a705c21cce…305a43ee` |
| r274 live log | `…/r274-flax-aio-pick-100adccc-resource2d589.log` sha256 `812e3e22…25906bee` |

## Two catalogs

Named Flax / reach files are identical on both frozen inputs:

- `FlaxAIO/{picking,flaxaio,banking,walking}.ts`
- `FlaxPicker.ts`, `FlaxRunner.ts`
- `webwalk/geometry/{localReach,Reachability}.ts`
- `BrimhavenAgility.ts`

This is not a catalog-regression split. 274 vs 289 differ in how many flax
tiles are `reachable` (on-tile) versus only `reachable_adj`; the script
path is `adjacentOk: true`.

## Native ownership today

Rust already owns scene collision and two different reach primitives:

1. `SceneQuery::can_reach` — m8aq `canReachLocal`. BFS over `can_step_local`.
   `expansions += 1` after the dest/adjacent checks; abort when
   `expansions > max_steps`. Default `max_steps` is **400**.
2. `SceneQuery::flood_reach` — one scene-bounded BFS, **no 400 cap**.
   Posted isolate `reachable` / `reachable_adj` bits, and entity row bits,
   use this. Comment at `query.rs` is explicit: not N× `can_reach`.

Host-play `with_script_snapshot_input` builds that flood once from
`snapshot.scene()` plus observe `here`, packs JS-safe u32 bitsets plus
step masks into `ReachQueryView`, and posts them. Typical 104×104 view:
3 × ceil(10816/32) u32 ≈ 4 KiB bitsets plus 10816 step bytes. Encode
thread drops the flood after pack. Delta omits `reach` when origin / dims
/ availability / bytes match. Unavailable (no `here`, `scene.available
== false`, reset) must post `available: false` / empty dims so the
isolate cannot keep a previous flood. That lifecycle is already correct
for bits.

Shim `Reachability.canReach` (`crates/script/src/shim/reachability.js`):

- npc/ground row on the tile → row `reachable` / `reachable_adj`
- else O(1) bit of posted flood
- **`opts.maxSteps` is unread**

Isolate test `isolate_reachability_walkable_and_coordinate_can_reach`
locks that: `maxStepsIgnored` expects true for `{ maxSteps: 1 }` on a
reachable empty floor, with message `"maxSteps is not a per-call BFS"`.
That was the 04d design (`maxSteps` unused on the posted view). It is
the wrong contract for FlaxAIO and every other enabled caller that
passes a budget.

Loc flax does not hit the npc/ground short-circuit (`entityOnTile` is
npc/ground only). Nearest-flax uses the coordinate flood bits.

Diagnostic `flax_aio_pick_failure_facts` records
`flood_reach().at(tile)` while labeling
`adapter_query: Reachability.canReach(tile,{adjacentOk:true,maxSteps:400})`.
The label is the foreign call; the measurement is the uncapped flood.
This audit did not change that helper.

## Foreign maxSteps semantics

`localReach.ts` (both catalogs, sha `4b2884b3…af1d4`):

```
const DEFAULT_MAX_STEPS = 400;
...
if (++expansions > maxSteps) return false;
```

This is expansion count of dequeued non-matching nodes, not Chebyshev
and not route length. `Reachability.ts` in the catalog is the foreign
engine; our shim replaces that module. Do not run their BFS in JS.
Use it as the semantic spec.

Foreign `Reach.ts` documents the open-ground radius of the same budget:

> `REACH_BFS_STEPS` expansions run out at ~11 tiles of open ground

`REACH_BFS_STEPS` is 400; `PROBE_RADIUS` is 10. Offline 8-connected
open-grid BFS matching dequeue+increment order agrees: adjacent target
at Chebyshev 11 is 380 expansions (accepted at 400); Chebyshev 12 is
462 (capped). Evidence: `expansion-model.json`. That file is a
standalone bound, not a Seers replay.

Opposite bound: a 1-wide corridor finds Chebyshev 41 at 82 expansions,
inside 400. So **path length 41 does not by itself reject**. Rejection
requires enough walkable side-area that 400 dequeues are spent before
the wave reaches the field.

## Frozen Flax control flow

`nearestFlax`: Locs name Flax + Pick, loc Chebyshev to field centre
`(2741,3444,0) ≤ 12`, sort by player Chebyshev, first
`canReach(tile, {adjacentOk:true, maxSteps:400})`.

`PickTask`: picking && !full && atField && nearestFlax ≠ null.

`GoToFieldTask`: picking && !full && nearestFlax === null
(**not** `!atField`). Execute: `travelTo(fieldCentre)` via local walk
or `Traversal.walkResilient` (nav pack). That walk is not the 400-cap
probe.

`BankTask` / `needsBank` pick-only: ground floor && `Inventory.isFull()`.
`bankRun`: travel stand, `OpenBooth`, `depositInventory`, delay 1 tick.
**No `Bank.close`.**

Task order: ContinueDialog, EscapeFlaxTrap, BankTask, spin tasks,
PickTask, GoToFieldTask.

After a successful deposit at the bank: pack empty → BankTask off;
`atField` false at Chebyshev 49 from field → PickTask off; GoToField
runs only if every in-scope flax fails the 400-expansion probe.

## Live raw facts (no fresh LIVE)

Both 100adccc cells, pick-only:

| | r289 | r274 |
|---|---|---|
| Predicate | `bank_closed` 150-tick watch | `bank_closed` via 180s deadline |
| FAIL tile | `[2721,3493,0]` | same |
| FAIL inv | empty | empty |
| Bank | open, loaded, gen 1 | same |
| `at_field` | false | false |
| WalkTo after OpenBooth | none | none |
| OpenBooth | 2213 `Use-quickly` at `2721,3494` | same |
| In-scope flax locs | 48 | 46 |
| player_distance | 41–55 | 41–55 |
| `reachable_adj=true` | 48/48 | 40/46 |
| `reachable=true` | 48/48 | 0/46 |
| nearest_reachable | `[2742,3452,0]` dist 41 | same tile, adj-only |
| reach source | `flood_reach().at` | same |

r289 chat contains 112 `"You pick some flax."` lines; that is not 112
items. Empty FAIL inv plus prior WalkTo/OpenBooth is the deposit
witness. 06ab already recorded `1779×28` on the 1eba overlay; this
2d589 pair does not re-count bank slots in the FAIL evidence object.

274 on-tile `reachable=false` with adj true is consistent with
`block_walk` flax. The script asks `adjacentOk`. Enough adj-true rows
exist on both revisions for `nearestFlax` to return a loc if the shim
treats flood adj as `canReach`.

## Does this explain observed control flow?

Using only these raw facts plus frozen source:

1. Unbounded flood says in-scope flax is adj-reachable from the bank.
2. Shim `canReach` therefore returns true for the first sorted loc.
3. `nearestFlax() !== null` → GoToField does not validate.
4. Bank stays open because the script never closes it; the fixture
   waits for `bank_closed` as a proxy for having left.
5. No post-deposit WalkTo is exactly what that predicate set produces.

That is sufficient to treat **unbounded flood vs budgeted canReach as
the suspected stall mechanism**. It is not sufficient to assert that
foreign `canReachLocal` on the missing Seers collision grid would have
returned false for every loc, nor that implementing a budget would
pass LIVE. Buildings can funnel a BFS toward corridor-like expansion
counts (still inside 400 at distance 41). Seers bank plaza is not a
1-wide corridor; foreign authors treat 400 as ~11 open tiles; live
nearest flax is 41. Those three statements can stand together without
a 100 percent claim.

Rejected as this stall's owner: missing flax, missing bank-close
capability, host WalkTo refusal on an open bank (no WalkTo was even
queued), catalog drift.

## Enabled maxSteps surface (through our shim)

Our `reachability.js` is the mapped `Reachability` module. Catalog
webwalk and scripts all hit it.

| Caller | Budget | Notes |
|---|---|---|
| FlaxAIO `nearestFlax` | 400 | This stall. |
| FlaxAIO `nearestStand` | omitted → foreign default 400 | Stands are adjacent; unbounded is locally harmless. |
| FlaxPicker / FlaxRunner | 400 | Same probe. |
| BrimhavenAgility edge | 512 | Coordinate `walkable && canReach`. |
| `arrivalProbe` | 512 | Webwalk arrival. |
| `doorCrossing` | 64 / 200 / 128 | Local door/landing probes. |
| WalkExecutor | 1200 / 256 / 256 / 64 | Reach-check / trigger / stall. |
| `api/walking/Reach.ts` | 400 | Same default; comments ~11 open tiles. |
| RandomEvents flee | 1500 / 600 | Larger than default. |
| upass | 2000 / 64 / 600 | Mix of whole-scene and tiny. |
| ikov / impcatcher | 20_000 | Larger than 104×104, i.e. scene-bounded. |

Honoring `maxSteps` is not a Flax-only shim special case. The reusable
primitive is expansion metadata from the flood already computed.

## Minimal Rust-owned capability

**Keep one host flood. Record dequeue/expansion index while doing it.
Post a JS-safe u16 map next to the existing bitsets. Shim compares
index to `opts.maxSteps`, defaulting omitted to 400.**

Not: JS BFS, per-query `can_reach`, isolate RPC, collision-grid copy,
second flood, catalog/task edits, `Bank.close` injection.

Suggested shape (implementation card, not this run):

- During `flood_reach`, store u16 dequeue index on each reachable tile
  (0 = start). Unreachable sentinel `0xFFFF`. 104×104 × 2 ≈ 21 KiB.
  Fits; scene max index 10816 < 65535.
- `canReach` exact: posted `reachable` bit **and** `index <= maxSteps`
  (match foreign: dest found at dequeue before increment, so 0-based
  index `k` succeeds iff `k <= maxSteps`).
- `adjacentOk`: dest may be blocked and have no index. True if some
  orthogonal neighbor has `index <= maxSteps` **and** the existing adj
  wall bit / `reachable_adj` allows that approach. Do not treat
  full-flood `reachable_adj` alone as budgeted adj.
- Omitted `maxSteps` → 400, matching `DEFAULT_MAX_STEPS` and
  `can_reach` `unwrap_or(400)`.
- Entity npc/ground short-circuit is a host extra. Foreign has no row
  path. Leave rows on unbounded bits or sample the same index map;
  Flax locs do not use it. State that as a semantic limit.
- Lifecycle: same keyframe/delta/unavailable rules as current `reach`.
  Standing at the bank, flood is stable and deltas omit. Region/reset
  still posts empty.
- Memory: +21 KiB per posted view versus ~15 KiB bitsets+steps today.
  No extra `SceneView` retain. No per-query deep copy.

`SceneQuery::can_reach` already implements the budgeted probe in Rust.
Posting its N-times cost per flax loc would be the rejected shape.
The flood+index map is the cached projection.

04d forbade honoring `maxSteps` with a **new BFS**. This recommendation
does not add a BFS. It reuses the flood 04d already approved.

## Semantic compatibility limits

Do not demand bit-for-bit `localReach.ts`:

- Expansion index matches foreign only if DIRS and enqueue order match.
  Native `DIRS` already match W E N S then diagonals; prove it with
  isolate tests, do not assume.
- Open-grid vs corridor bounds differ; live Seers density was not in
  the log.
- Entity row short-circuit can disagree with budgeted coordinate
  lookup on an npc/ground tile.
- `flood_reach` adj is computed after the full flood; budgeted adj is
  a neighbor-index check, not the uncapped adj bit.
- Webwalk `pathExpand` has its own `maxSteps=800` BFS; that is a
  different function and stays out of this hop.
- A correct budget can still leave Flax FAIL for other reasons
  (identity/colocation 05k, nav, clocks). Bounded reach is not
  acceptance.

## Proof design (not LIVE)

Implementation card, empty-target isolate + `api` flood tests. Do not
launch LIVE from that card. Do not retarget `flax_aio_pick_failure_facts`
from this audit.

1. Nearby accept: open floor, dest Chebyshev 1–8, `maxSteps: 400` true;
   `{ maxSteps: 1 }` true only for start/adjacent as foreign does.
2. Distant reject: dest whose flood bit is true at Chebyshev ≥ 12 on
   an open packed scene, `{ maxSteps: 400 }` false; uncapped / huge
   budget still true.
3. Adjacency: blocked dest, open orthogonal neighbor within budget →
   `adjacentOk` true; wall-separated neighbor → false.
4. Collision pocket: isolated open tiles remain false at any budget.
5. Region reset: `available: false` / empty dims clears last index map
   (no stale course flood).
6. Default: omitted `maxSteps` equals 400, not unbounded.
7. Sync JS read, no `host.interact` row.

The existing `maxStepsIgnored` isolate assertion must change on that
card (owned by whoever implements; this audit did not touch it).

Offline evidence already in this tree: `expansion-model.json` for
distances 1,2,8,10,11,12,20,41,49,55; `flax-loc-facts.json` from the
frozen logs.

Root LIVE remains the only acceptance of `flax_aio_pick`.

## Stopping rule

This audit is done. Next hop is a bounded implementation card with
same-card `reviewer`, then root LIVE. Do not treat this document as
source acceptance or as a passed flax cell.
