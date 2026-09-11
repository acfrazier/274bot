# Tanner arrival ownership against preserved native WalkNear

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 09:50 UTC. Kind: bounded read-only corrective design audit
of 04v for brief 100. Not implementation, LIVE, fixtures, ledger, STATE,
dim, timeout, or a foreign-router copy. Root owns acceptance, any LIVE
snapshot, and commit of this evidence. No routine reviewer card.

Read once: `AGENTS.md`, `docs/execution.md`, brief 100, 04v. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Frozen host is exact
export `.superpowers/review-exports/catalog-root-78cc99d0` (commit
`78cc99d07046a781fe8cd5ebf66bb63e663d23ea`), not current WIP. Concurrent
named-bank / resource / paired / UI workers and root `28d97f38` binaries
are unrelated. Work was read-only except this report and
`docs/compat/evidence/tanner-arrival-contract-audit/`. Raw 78 cells stay
in `docs/compat/evidence/catalog-harness/` and
`docs/compat/evidence/catalog-headed/`.

## Verdict

**04v's claim that host WalkNear must force indoor adjacency to fit the
catalog 5000ms modal wait is rejected.** A Chebyshev-3 stand for a
radius-3 request is the existing native contract. 04v's "radius 0"
is the traveller's last-hop exact stand on the *selected approach tile*
`3274,3188`, not WalkNear radius 0 and not a door-tile failure.

Keep 04v's raw finding: both soft 274/289 cells observed real IF 679
(tan-all 8686, 27 hides, 2000 coins) after Trade, at indoor
`3276,3193`. Re-evaluate ownership separately from that symptom. The
timeout text is not "IF never opened". A later bank-leg snapshot cannot
answer arrival collision.

Do not tighten WalkNear. Do not copy foreign `isArrived`. Do not extend
5000ms. Do not delay `Npc.interact`. Do not issue a categorical dim or
native-fix from timeout duration alone. The exact loaded-scene
connectivity of requested `3277,3191` from proven terminal `3274,3188`
at the Arrived+Trade isolate post is still absent.

| | Hypothesis | Status |
|---|---|---|
| (1) | Shim misrepresents native WalkNear (should require dest-reachable / indoor) | **Falsified.** Native `approach_tiles` + `calculate` pick the first packed-*standable* tile within Chebyshev radius that has a packed path, nearest-to-player first. Shim `walkResilient` completion is that same Chebyshev. |
| (2) | Native preservation regression vs 78 | **Falsified** for this mapping. 78 export `approach_tiles`/`calculate` match the live 78 binary that aimed `3274,3188`. Traversal.js byte-identical 78 ↔ current. |
| (3) | Host must copy foreign `isArrived` (reach after radius) | **Rejected as policy.** Posted ReachFlood already exists; Traversal must not grow a foreign router. |
| (4) | Catalog accepts a legal radius-3 arrival, then 5000ms is its own IF_OPEN budget | **Open**, split by the missing collision snapshot. Not a WalkNear defect either way. |
| (5) | `3274,3188` connected to `3277,3191` under selected local collision at Trade | **Absent.** Widget witness and fail `latest` are later tiles. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen host (tested binary) | `78cc99d07046a781fe8cd5ebf66bb63e663d23ea` |
| Isolated export | `.superpowers/review-exports/catalog-root-78cc99d0` |
| Campaign HEAD at write-up | `7c2eef6eb8db6b01933c333aa64cf5f68169d8d2` (WIP; not the audit source) |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 100 SHA-256 | `24f5a04d15b38644ab41061e09594537c7bfc2c19e98b5c7830374d2fcebc7bf` |
| 04v SHA-256 | `d43b1477b6b42364ef7920e1449699726dbbaed831a2d0a80ff77a4d235c212a` |
| Kanban card | `t_bcf07b82` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| TannerBot.ts (both; sha256 equal) | `b8038b17ac72eec3cf7b2e84f8323e4cd898f1975de7d210cfba333e552b8d7f` |
| Foreign Traversal.ts (both equal) | `984fda1becb12276c46e61e8588f4bd5d51085f968849ff202034b49c44f5d6d` |
| Foreign arrival.ts (both equal) | `f65ded68a962c2a62c772b6c6817827e6174ef05b9127c931025d519d59e8540` |
| Foreign Reachability.ts (both equal) | `4388d15d919249dd4468aa740345168fcf0d7e2068b13a08a38c87cc8a3b36ec` |
| Frozen Npc.ts (both equal) | `241f23f2fb83edf27f1166c40dd23cfd5d6c34d72301abefe356993d2151c2d3` |
| Frozen Input.ts (both equal) | `90d83a09b0469e02c3b351b60b9577349164a3605d7d638ee6edc42af8e55bff` |
| 78 + current traversal.js | `5b272833520f908b02e1aae3c75577829ab1c8ad843082659aca0b6148980274` |
| 78 host-play `lib.rs` | `826a920437f1ae5953755f15601fbeb48c3f61773174bfd3018a8642435a2612` |
| 78 `interact.rs` | `7e2e23fabdcf29d82181c14e0b3f9399e21f0f16ae2c3126b0749ca9141deb06` |
| 78 `query.rs` | `84158080adc1a00d4ba710083f27ad43d746b1a817cdc4c0257491baf9f7f0b9` |
| Headless binary | `c334aae122661a235af1fd13c8028819a42e8e479f7dd4604fe160f399ee0816` |
| 274 nav pack | `05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4` |

Machine-readable copies: `evidence/tanner-arrival-contract-audit/{refs,hypotheses}.json`.

## 1. Actual native WalkNear contract

78 export `ScriptWalkArm::route_with_radius` (`host-play` 3286 / 5262):

- Radius ≤ 0: exact packed route to `to`.
- Radius > 0: `approach_tiles` enumerates packed `collision.standable` tiles
  with `|dx|<=r` and `|dz|<=r` (Chebyshev square, clamped 0..=104), sorts by
  `(chebyshev(from, t), chebyshev(t, dest), x, z)`, then `calculate` returns
  the first candidate with a packed path.

Traveller last hop onto that selected tile is exact (`radius = 0` when
`last`). Mid-hops click-ahead at Chebyshev 2. That is why the live log
prints `aim=3274,3188 radius=0` after `WalkNear { x: 3277, z: 3191, radius: 3 }`.

From bank `3269,3166` (the armed `from`):

- `3274,3188` → Chebyshev 22 from player, 3 from dest.
- `3277,3191` → Chebyshev 25 from player, 0 from dest.

Nearest-to-player therefore prefers the SW radius-3 ring over dest whenever
the ring tile is standable and pathable. Indoor dest is not selected even if
a later packed door hop to it would also exist. That is preserved host
semantics, not a missing indoor remainder.

`max(|3274-3277|,|3188-3191|) = 3`. Distance 3 for radius 3 is valid.

## 2. Actual shim and foreign contracts

Shim `traversal.js` (78 and current, byte-identical): `walkResilient` with
`radius > 0` queues native `walk-near` and completes when
`chebyshev(here, dest) <= radius`. It does not read posted Reach.

Posted Reach already exists: isolate `Reach` table, `SceneQuery::flood_reach`
in 78 `query.rs`, entity `reachable` / `reachable_adj`, shim
`reachability.js`. WalkNear does not consume those bits. Using them to keep
`walkResilient` open after a legal packed arrival would copy foreign
`isArrived` into the shim. Brief 100 forbids that.

Frozen `TannerBot.walkTo`: skip if Chebyshev ≤ 4, else
`Traversal.walkResilient(dest, { radius: 3, attempts: 2, timeoutMs: 45_000 })`.
Then `Npcs.query().name('Tanner').nearest()`, `interact('Trade')`,
`delayUntil(modals().main === 679, 5000)`.

Frozen `Npc.interact` → `Input.interactNpc` → `actions.menuAction(...)`
returns immediately. Host `input.js` / `npcs.js` queue `InteractReq::Npc` and
return true immediately. `check_target` does not require NPC adjacency
(scene presence only). Soft 679 after Trade already falsifies wrong-op /
never-sent as first owner.

Foreign `isArrived` (arrival.ts 18–36): same-level Chebyshev ≤ radius, then
*additionally* `probe.canReach(dest)`; if dest is walkable but not reachable,
return false; else `!probeable || canReachAdjacent`. Foreign
`walkResilient` feeds that probe every ladder pass and may scene-walk /
unstick / `tryNearbyDoor`. That is foreign router policy. Do not copy it.

## 3. What the 78 cells actually prove

Soft 274 old-catalog (`live4dullf_0`, log sha
`6e0b0d644e61fff5fd9547e69fedf0bc0786ad121e9f6f146c0d960b89e797cb`):

1. `WalkNear { 3277, 3191, radius 3 }` from `3269,3166`.
2. Last hop exact `aim=3274,3188 radius=0`.
3. `Arrived { at: 3274, 3188 }`.
4. Immediate `interact Npc Trade` index 225.
5. No `[nav-walk]` during the catalog 5000ms wait.
6. `tanner interface did not open — retrying`, then `WalkNear` bank.
7. First host nav after timeout already at indoor `3278,3192` (client OPNPC
   pathing, not host nav).
8. Soft witness tick 115 / tile `3276,3193` / modal 679 / 8686 present.
   Fail `latest` tick 150 / `3273,3169` / modal -1 is the next bank leg.

289 soft: same packed terminal, Trade 229, first indoor after timeout
`3277,3191`, witness tick 120 / `3276,3193` / 679. Hard bothrevs: same
Trade/timeout/bank order, no 679 witness hit. Headed 274/newer: Trade→timeout
5.398s / 5.363s; indoor logged only after the wait; terminal snapshot is
another bank tile with no Tanner NPC.

Packed route fact is proven. Loaded-scene reachability of dest at step 3–4
is not in any retained snapshot.

## 4. Ownership

- **Not a shim misrepresentation of WalkNear.** Chebyshev completion matches
  native radius selection. 04v treated foreign indoor adjacency as a host
  guarantee because 5000ms elapsed. A fixed NPC modal timeout does not
  create that guarantee.
- **Not a native preservation regression** in the 78 WalkNear path.
- **Not authorized:** Tanner-specific WalkNear tightening, generic radius
  change, delayed `Npc.interact`, host timer expansion, forced NPC
  relocation, cloned door opener, foreign `isArrived` in `traversal.js`.
- **Catalog caller:** `walkTo` skip-at-4 plus radius 3 plus immediate Trade
  plus 5000ms. On the foreign runtime that arrival also demanded dest
  reachability; on this host it does not. Repairing that foreign assumption
  is outside host policy. Dim only with the missing collision fact, not from
  timeout duration.

If the next snapshot shows dest **reachable** from `3274,3188` at Trade,
foreign `isArrived` would also have accepted the same tile, and the remaining
defect is the catalog 5000ms IF_OPEN budget after a legal radius-3 arrival
(dim candidate under the existing host-semantics / foreign-dim policy;
keep 5000ms). If dest is **walkable but not reachable**, the remaining defect
is the catalog assuming foreign `isArrived`; still do not copy that policy
into WalkNear. If dest is **not walkable**, report `reachable_adj` of dest
and of the Tanner entity at that same post — Chebyshev 3 is not adjacency.

## 5. Single next observation (root LIVE only)

One isolate/debug dump at the Arrived+Trade post, player still `3274,3188`,
before the 5000ms cond settles false. Reuse already-posted ReachFlood; no
new harness.

Required fields:

- `here` = `3274,3188,0`
- `Reach.available` plus walkable / reachable / reachable_adj bits for
  `3277,3191` and `3274,3188`
- Tanner `SceneEntity` tile, distance, `reachable`, `reachable_adj`
- `main_modal_id` (expect -1 at queue time)

Do not substitute the tick-115 indoor witness or fail-terminal bank tile.
No worker may launch this. Root owns controlled LIVE.

No implementation slice. No dim. No routine review hop.
