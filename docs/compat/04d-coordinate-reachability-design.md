# Bounded native coordinate reachability bridge

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: bounded architecture for brief 58. Not
implementation, LIVE, fixtures, or a change to frozen catalog sources.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, brief
58, and the named host files at HEAD plus client gitlink. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Work was read-only
except this report. No product edits, tests on moving WIP, LIVE, STATE,
subagents, stash, reset, checkout, merge, remotes, or release actions.
Concurrent uncommitted shim WIP on this checkout is not this product.

## Verdict

**Keep the existing host-thread flood. Post its walkable / reachable /
reachable_adj bitsets on the isolate snapshot. Map `Reachability.walkable`
and coordinate `canReach` to O(1) lookups of that posted view. Do not add
a native isolate callback, IPC RPC, JS BFS, or a world copy.**

Entity `canReach` stays on posted npc/ground row bits. Coordinate inputs
use the same flood that already stamps those bits. `maxSteps` stays unused
on the posted view (it is already unused for entity rows). Design approval
is not source acceptance and not live radius-8 success.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `43672c4d47fc5548777d32e0db34ad004b9c9f9f` |
| Isolated live host | `50f2be8a216bf96691d37454c1d70644e9ed835b` |
| Client gitlink | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 58 SHA-256 | `3bca2427a997e9e0287588e215f3246e20209a0d9b1e04a1c550b68e18132b6c` |
| Kanban card | `t_d49b087b` |
| AgilityBot.ts (both catalogs) | `dee7c8885685724b0453097a0cf6a66d36a4d081e24157f29d35c742c518028f` |
| Radius-8 289/old-catalog | `docs/compat/evidence/catalog-harness/live/r289-gnome-course-radius-100adccc-50f2be8a.*` (exit 1) |
| Radius-8 274/old-catalog | `…/r274-gnome-course-radius-100adccc-50f2be8a.*` (exit 1, same sequence) |

## Two findings, kept apart

Default `gnome_course` on host `50f2be8a` / client `56d80272` completed
laps on both frozen catalogs and both revisions. Radius-8 did not.

Live radius-8 (289 and 274, old catalog) ran a full lap from the teleported
start, then:

1. `lap 1 complete`
2. `course re-sync: step 0 (log balance) -> 6 (obstacle pipe)`
3. `Squeeze-through` at `(2484, 3435, 0)`
4. `tick 131` / `tick 130`: `not impl: Reachability.walkable`

The throw is a missing host mapping. `proxy('Reachability')` in
`crates/script/src/shim/reachability.js` has no `walkable`. Do not dim
GnomeCourse and do not change host semantics to hide the call.

The re-sync is a separate foreign-script finding, not a missing verb.
`DoObstacle.find` uses Chebyshev `l.distance() <= searchRadius`. After the
pipe, the log loc last clicked at `(2474, 3435)` is Chebyshev 10 from
`(2484, 3435)`. Radius 8 cannot see it; radius 20 can. `resyncTo` then
walks course names in declaration order and accepts the first still-visible
name, which is the pipe just finished. That is imported `AgilityBot.ts`
search/resync policy. Do not invent radius-8 success, expand the fixture
radius, or special-case gnome tiles in the host.

After `walkable` exists, the same reposition path calls
`DirectNavigator.walkTo`, which the shim still throws (`not impl:
DirectNavigator.walkTo`; only `walk` queues `walk-to`). That is the next
missing op, not this hop.

## What source does now

Rust already owns scene collision and reach:

- `SceneQuery::walkable` is `collision_at` present and
  `flags & CollisionFlag::SQ_BLOCKED == 0` (`query.rs`). Off-scene,
  other plane, or `scene.available == false` is not walkable.
- `SceneQuery::can_reach` is a capped BFS (default 400, `adjacent_ok`).
- `SceneQuery::flood_reach` is one scene-bounded BFS with no 400 cap.
  Posted isolate `reachable` / `reachable_adj` bits use this, not N×
  `can_reach` (`query.rs` comment at `flood_reach`; host-play
  `with_script_snapshot_input` builds the flood once from
  `snapshot.scene()` plus observe `here`, then samples `ReachFlood::at`
  onto npc/loc/player/ground rows).

The isolate never sees that flood. It receives a FlatBuffer snapshot
decoded on the `js-isolate` thread into `__rs2b0t_host.snapshot`. Host
does not block on JS. JS→host is the interact queue after the tick. There
is no synchronous native query callback.

Current `Reachability.canReach` resolves a tile, finds a matching posted
npc/ground row, and returns `reachable` / `reachable_adj`. No row →
`false`. Empty-floor gnome faces therefore never match. Isolate tests
already lock that: `isolate_reachability_can_reach_reads_posted_flags`
expects a bare `{x,z,level}` with no row to be false, and
`isolate_reachability_can_reach_is_not_chebyshev` forbids a distance fake.

Deltas omit unchanged tables; the isolate keeps the last JS value. Reset
clears the snapshot fingerprint so the next post is a keyframe
(`02b-host-snapshot-reset.md`). Flood is skipped when `here` is missing or
`scene.available` is false; entity bits then post `(false, false)`.

Root correction after inspecting immutable 50f2be8a source: `GameSnapshot::tile`
and `host_play::player_here_tile` both publish `client.minusedlevel`, not a
hardcoded body level 0. The sampled Gnome milestones happen on the ground;
that does not establish a constant-plane defect. Live loc dispatches include
levels 1 and 2. Flood requires `here.level == scene.level`; coordinate queries
use this actual posted scene plane, and a destination on another plane is false.
No new player-plane field is required by this design.

## Enabled callers of these two ops

Frozen enabled set only. Catalog AgilityBot hashes match.

| Caller | Ops | Notes |
|---|---|---|
| GnomeCourse `repositionForRetry` | `walkable(f) && canReach(f)` on four orthogonal faces of the loc tile, `level` copied from the loc | Exact live throw. Faces omit player tile. Then `DirectNavigator.walkTo` (out of scope). |
| BrimhavenAgility edge staging | `walkable(t) && canReach(t, { maxSteps: 512 })` | Same two ops. `maxSteps` is already ignored for entity rows; keep that on the posted view. |
| CookBot / Firemaker | `walkable` plus `canStep` | `canStep` stays `not impl` on the same proxy. Mapping walkable does not finish those cards. |

Webwalk `WalkExecutor` / `doorCrossing` / `arrivalProbe` also call
`canReach`/`walkable`/`canStep`/`probeable`. They are not these two
mapped verbs and are not this hop. Do not clone foreign `localReach.ts`.

Coordinate contract for the shim:

- Missing / non-numeric `x` or `z` → `false` (`resolveTile` already).
- Missing `level` → `0`.
- Entity with `tile()` keeps npc/ground row bits when a row exists
  (existing tests).
- Else look up posted bits. `adjacentOk` selects `reachable_adj`.
- `maxSteps` is not a per-call BFS.

## Rejected shapes

| Shape | Why not |
|---|---|
| V8 native callback into `SceneQuery` | New isolate-thread seam. Host already computed the flood at encode. Sharing `Arc<SceneView>` with JS would be a world handle, not a query view. |
| Generic IPC / RPC during the tick | Brief forbids it. Isolate commands are already one-way snapshot then tick. |
| JS port of `canReachLocal` / collision flags | Foreign navigation clone. Fail-closed. |
| Per-call `SceneQuery::can_reach` | Re-floods on every face. Diverges from posted entity bits (400-cap vs uncapped flood). |
| Deep-copy `collision_flags` (~104×104 i32) into the isolate | World copy. Bitsets are the cached derived view. |

## Recommended bridge

Reuse the flood `with_script_snapshot_input` already builds. Next to it,
pack a walkable bitset from the same `SceneView` (`SQ_BLOCKED == 0`, same
`lx * height + lz` index as `ReachFlood`). Put three bitsets plus origin
on the snapshot:

```
reach: { available, base_x, base_z, level, width, height,
         walkable[], reachable[], reachable_adj[] }
```

Encode words as **u32 or bytes**, not JS `number` ulong. A u64 word
through `f64` drops bits. `ReachFlood` can keep u64 internally; the
packer emits JS-safe words.

Ownership:

- Host observe/encode thread: borrow `GameSnapshot::scene()`, compute
  flood + walkable pack, drop both after encode. No extra scene retain.
- FlatBuffer owns the copy in the post bytes.
- Isolate thread decodes into `__rs2b0t_host.snapshot.reach` before the
  tick. JS reads are synchronous and local.
- Per isolate: about 3 × ceil(104×104/32) u32 ≈ 4 KiB last view, not a
  collision grid and not a second BFS.

Invalidation (stale-session):

- Keyframe always includes `reach`.
- Delta omits `reach` only when origin/level/dims/available and bit
  bytes equal the last post.
- Available → unavailable (no `here`, `scene.available == false`,
  reset): **must post `available: false` / empty dims**. Omission would
  keep the previous course flood. Reset already forces a keyframe.
- Selected scene/revision identity is whatever observe already bound;
  do not add a second scene handle.

Shim (`reachability.js` only for these two members):

```
walkable(target) -> resolveTile, then reach.walkable bit, else false
canReach(target, opts) ->
  existing npc/ground row path when a row exists;
  else reach.reachable / reachable_adj
```

No interact queue, no hardcoded gnome/face coordinates, no change to
`DirectNavigator`, nav, or catalog sources.

## Minimal source seams (implementation card)

1. `api`: pack walkable bits; expose flood words/origin without cloning
   `SceneView`. Existing `query.rs` walkable/flood tests stay the oracle.
2. `host-play` `with_script_snapshot_input`: attach the pack to
   `SnapshotInput`; keep one flood per encode.
3. `isolate.fbs` + `isolate_fb` encode/decode/fingerprint. Old buffers
   without `reach` remain readable; JS treats missing as unavailable.
4. `load.rs` snapshot apply: present `reach` overwrites; omitted keeps
   last; keyframe unavailable clears.
5. `reachability.js` as above.

Do not edit fixtures, LIVE harnesses, or foreign inputs.

## Falsifiable experiment (not LIVE)

Isolate + encode tests, meaningful behavior:

- Open in-scene floor: `walkable` true; `SQ_BLOCKED` tile false;
  off-scene / other level / missing `reach` / `available: false` false.
- Coordinate `canReach` true on empty floor the flood can reach; false
  on an isolated open pocket; false on a wall; `adjacentOk` uses adj
  bits only.
- Entity row path unchanged (`exact`/`adj`/`tile`/`missing` in
  `isolate_reachability_can_reach_reads_posted_flags`).
- Far empty tile still not Chebyshev-true.
- Delta: unchanged bits omitted; scene/player-tile change reposts;
  unavailable post clears last bits (no stale flood after reset).
- Probe is a sync JS read (no `host.interact` row).

Host-play may keep sampling entity bits from the same flood so row bits
and coordinate bits cannot disagree on one encode.

A source review of that card is not live acceptance. Radius-8 LIVE stays
root-owned and can still fail on foreign re-sync and
`DirectNavigator.walkTo`.

## Stopping rule

This design is done. Implementation is a later bounded card with
same-card `reviewer`. Stop that card when the two ops no longer throw
and the tests above pass on the posted view.

Do not, in that card: clone foreign reach, honor `maxSteps` with a new
BFS, map `canStep`/`probeable`/`walkTo`, rewrite AgilityBot, dim
GnomeCourse, or call radius-8 LIVE green because `walkable` exists.

Out of scope here: product edits, LIVE, packaging, nav identity, tile
distance (`43672c4d`) and settings Tile shape. Native level publication already
uses the selected current scene level; do not add a speculative decoding fix.
