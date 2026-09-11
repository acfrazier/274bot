# Trained Brimhaven route failure ownership (native942 / Agility 52)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 15:20 UTC. Kind: bounded read-only diagnosis for
brief 141. Not implementation, compilation, LIVE, fixtures, client,
ledger, STATE, timeout inflation, or a foreign planner/route copy.
Root owns acceptance, display, and any later source change. Own only
this report and `docs/compat/evidence/trained-brimhaven-route-audit/`.
No new tasks. Do not implement dim.

Read once: `AGENTS.md`, `docs/execution.md`, brief 141. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `8c56eb85a846162cf2e13e8ce00c49ca372071b1`. LIVE cells were
recorded on frozen host `9429357d1124a9a3451c33e2fae1725fd5296be0` /
client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`. Work was read-only
except this report and the unique evidence dir. Frozen catalogs are
`.superpowers/inputs/rs2b0t-{100adccc037d9f6898080e1cad58fcfc43364775,8e7d965be2071d6ec65c3265e12af797082d720a}`.
Concurrent WIP was not touched.

Inspected only the two completed old-catalog native942 cells named by
the brief. No LIVE process was started. 06ad remains the Agility-1
snapshot; training did not retire these FAILs.

## Verdict

**289 no-first-hop-XP is a foreign route/avoid policy defect, not a
host loc-selection bug and not an underqualified actor.** At Agility
52, northern first-hints make `pathPlatforms(24, goal, 52)` uniquely
choose plank 24→23. `nextHop(..., avoid=23)` has no equal remaining
hop-count alternative, so it returns 23 anyway. `CrossObstacle` then
clears `avoidHop` because the chosen hop equals the avoid list, and
retries the same Walk-on. Three native no-roll dumps, zero agility XP,
FAIL still in the pit.

**274 hop-XP then no-first-Tag is not a missing Tag primitive and not
a WalkTo/tile identity defect.** The same plank policy wasted two
falls. The cell escaped landing only because the hint rotated to
pillar 1, where swing 24→19 ties hop-count with plank and wins on
`hopKindCost` (3 < 5). Silent native swing XP passed step 12. The
script then chased 1 via pillar/spikes/wall/saws, missed two 98-tick
hints, and expired the 150-tick Tag watch on plane 3 while staging
12→13. Spinning-blade chat is the native saws fail line
(`You were hit by the spinning blades!`), and the WalkTo intent
`2778,9557` is exactly `walkTrapStand(7,6)`.

Do not copy `nextHop` / `ARENA_EDGES` into Rust. Do not teleport
between hops, seed ticket 2996, force the arrow/timer, or raise 180s /
150. Do not treat 52 as having solved Brimhaven. Root may dim the
imported `BrimhavenAgility` card for the 24→23 avoid-retry identity
below; this audit does not dim.

## Classification against brief 141

| | Question | Applies? |
|---|---|---|
| Actor now 52/52 | Reviewed 131 seed | **Yes.** Both baselines agility base 52, effective 52, XP 123660. |
| Frozen catalog identity | Both catalogs | **Yes.** `BrimhavenAgility.ts` / `Logic.ts` sha256 match across `100adccc` and `8e7d965b`. Native942 LIVE is old-catalog only. |
| 289 no hop XP | Plank 24→23 loop | **Yes.** Goals 17 then 20. Loc 3572 @ 2802,9590 ×3. Chat is broken-plank ×3. Predicate `stat_xp_gain(16)>=1` at 228 ticks / 47697 ms. Tile `[2802,9590,0]`. |
| Foreign nextHop vs longer swing | equal-rem only | **Yes.** For goals 17/20/22, rem[23] = rem[24]-1 and rem[19] is +1 hop. `avoid=23` cannot take 19. Script resets avoid when hop==avoid. |
| Same-tile host loc defect | 3572 vs other planks | **No.** 3572 is `agilityarena_plank3` (Walk-on). `plank_broke1` is 3576 and has no op, so `findEdgeLoc` cannot click it. Dump is native `loc_find(coord, broke1)` after a 2-tile forcemove. Host clicked the script's unique from-side Walk-on. |
| 274 hop XP | swing 24→19 | **Yes.** Arrived platform 19, anim=true, pit=false. Native success has no mes. Step 13 is first Tag. |
| 274 first Tag | `tag the next` | **No Tag.** Hints 22→1→13, two `missed pillar` lines, never `here === target`. FAIL tile `[2783,9564,3]` during 12→13 stage WalkTo `2784,9568`. |
| Spinning blades | host path into blade | **No.** Native saws timer chat. Intent matches `walkTrapStand(7,6)`. At 52, `stat_random(30,210)` is 124/256 ≈ 48.4% success. `pathingEdges` keeps saws; `hopKindCost` prefers them (0). |
| Unavailable host primitive | Tag / WalkTo | **No.** TagPillar never validated. Stage misses recovered with click-from-here; hops still arrived. |
| Ordinary timer/RNG miss | 150 ticks / 98-tick hint | **Partial for 274 Tag only.** Chase after two plank falls cannot tag within one 98-tick window. Not the 289 owner. |
| Budgeted reach 140 | flood / maxSteps | **Not this owner.** Approach uses `maxSteps: 512`. 289 never left the landing plank. A 140 claim would need a flood-index rerun on the two 274 stage-miss tiles, not a 100-percent statement. |
| Raise 180s / 150 | | **No.** 289 wall 50.9s / 228 ticks. 274 131.7s / 303 ticks, Tag watch is the limiter. |
| Training solves new FAILs | | **No.** Preserve 06ad as the Agility-1 snapshot. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `8c56eb85a846162cf2e13e8ce00c49ca372071b1` |
| LIVE host (native942) | `9429357d1124a9a3451c33e2fae1725fd5296be0` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Catalog-boundary binary | sha256 `51c4e645ba7d7f0c1b0ab1df35e624358d2a794a18748f2b16d77f9448a624d6` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 141 SHA-256 | `ec616de4e955cbfca0bc82a8b09df9a01991135979d51f8fed00e8b083173774` |
| Kanban card | `t_1739ef62` |
| Old catalog (LIVE) | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog (source only) | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine (receipt) | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| BrimhavenAgility.ts (both catalogs) | `771daff07bd4b3d6f2826ab1300d4fd66bcbae0f9d7a76e4a2ad07a4d050e859` |
| BrimhavenAgilityLogic.ts (both) | `fedf5f8e05fd43642efb0370352e71e5feebf258a784bc503a2e145c33b46d4d` |
| 274 agilityarena.rs2 | `c33bf541b9e5e557dd607b27bfe7e065d5c9bfe2c949a242585a6dde787335d5` |
| 289 agilityarena.rs2 | `dadc3599740ced8b9cd9db5feb4314163c0c63fbdc300fec60f10d99bdf1a9a7` |
| agilityarena_zones.rs2 (274=289) | `459895b3961bef9f4baf9baeb2f8e7179aab4cb17474d7ca87cb2e5303267f58` |
| r274 log SHA-256 | `4a880ee71f1f48542bcf2f2821fa153bad1d4a3714c05e2056102ea780372332` |
| r289 log SHA-256 | `6a7d56fb69deb7d74f813d25fd774edd22b511656d5666d7c6c1916d97b2c7a1` |

Machine-readable copies:
`evidence/trained-brimhaven-route-audit/{refs,hypotheses,cells,nexthop}.json`.
Log sha256 values match the per-cell `*.json` receipts.

## 1. Two completed native942 old-catalog cells

Shared baseline-after-preparation: tile `[2809,3194,0]`, scene 2,
Coins 1000, Lobster 10, ticket 2996 absent, varp 309 = 0, agility
base 52 / effective 52 / XP 123660, hitpoints 10 / XP 1154. Start
loaded. Card settings `stealRestock=false`, `bankAtTickets=1000`.
Pay posted (npc 4575 on 274, 4584 on 289). Climb-Down loc 3617.

| Cell | Exit | Wall s | Runner | Predicate | Tile |
|---|---|---|---|---|---|
| r274 old `100adccc` | 1 | 131.705 | FAIL 303 ticks / 128462 ms | step 13 `chat(contains "tag the next")` | `[2783,9564,3]` |
| r289 old `100adccc` | 1 | 50.899 | FAIL 228 / 47697 | step 12 `stat_xp_gain(16)>=1` | `[2802,9590,0]` |

r289: plank 24→23 goal 17, fall, avoid 23, climb rope 3610 @
2805,9591, `still in the pit`, hint 17, **same plank 24→23**, fall,
avoid 23, climb, plank 24→23 goal 20, fall 3, FAIL in pit. Inv 800
coins + 10 lobster. Chat: broken-plank ×3 + fee. No swing log.

r274: plank 24→23 goal 22 ×2 with the same loc/falls/avoid reset,
then **swing 24→19 goal 1 arrived**, `missed pillar 22`, hint 1,
pillar 19→14 arrived, spikes 14→13 stall then arrived, pillar 13→12
arrived (stage 2793,9568 missed, click-from-here), wall 12→7 arrived
(stage 2785,9564 missed), saws 7→6 WalkTo 2778,9557 stall ×2
(`distStart=4` then `3`), wall 7→12 goal 13 arrived, `missed pillar 1`,
hint 13, pillar 12→13 stage WalkTo 2784,9568, FAIL. Inv 800 + 10
lobster. Chat newest: spinning blades, broken-plank ×2, fee.
Runenergy 92.

289 `models.bin` missing is a snapshot skip, not this owner: Pay,
descend, and plank dumps still happened.

## 2. Frozen nextHop / pathingEdges at 52 (not a new controller)

`pathingEdges` keeps every edge with an endpoint 24, including plank,
and drops other planks only when agility ≥ 20. Blades and darts are
dropped. Hop-count BFS therefore still uses 24→23.

Evaluated on the original pure functions (agility 52):

| from | goal | avoid | rem[24], rem[23], rem[19] | first hop | nextHop |
|---|---|---|---|---|---|
| 24 | 17 | 23 | 3, 2, 4 | 23 plank | **23** (no equal-rem alt) |
| 24 | 20 | 23 | 4, 3, 5 | 23 plank | **23** |
| 24 | 22 | -1/23 | 2, 1, 3 | 23 plank | **23** |
| 24 | 1 | 23 | 7, 6, 6 | 19 swing (geo 66=66, cost 3<5) | **19** |

`CrossObstacle.validate` calls `nextHop(here, target, agility)`
**without** `avoidHop`. `execute` passes avoid, then:

```
if (this.bot.avoidHop === hop) this.bot.avoidHop = -1;
```

So a known-broken 24→23 is retried and the avoid bit is discarded.
That is the 289 loop. 274 left the landing because the hint became 1,
not because avoid took the longer swing.

Foreign tests already skip ticket-grid planks 5↔6 and blades; they do
not cover landing 24→23 with `avoid=23`.

## 3. Loc 3572 @ 2802,9590 is not a host same-tile mispick

Both engines' `loc.pack`: `3572=agilityarena_plank3` (name Plank,
op Walk-on). `3576=agilityarena_plank_broke1` has no op.
`findEdgeLoc` queries name `Plank` with `actions().length > 0`, then
scores mid-point + 0.25×from-side. The live interact is always
`Walk-on Plank @ 2802,9590` id 3572, staged from WalkTo `2804,9590,3`
(platform 24 is 2805,9590; 23 is 2794,9590).

Native `[label,agilityarena_plank]` forcemoves 2 tiles then
`loc_find(coord, agilityarena_plank_broke1)` with **no agility roll**.
Clerk `agilityarena_change_planks` keeps outer tiles of two of the
three rows always broke1. Selecting the from-side Walk-on loc walks
the player onto that overlay. Other inner/middle planks either have
no Walk-on or lose the from-side score. This is foreign loc-choice
plus selected-content overlay, not the host clicking broke1 instead
of a good loc on the same tile.

06ad's "always broken" fact is the no-roll dump **when** broke1 is
under the player, which these five native942 attempts all hit.

## 4. 274 first Tag path (WalkTo vs policy)

Hop source/target after the two plank falls: 24→19 swing loc 3566 @
2806,9585, stage WalkTo `2806,9587,3`, arrived 19. That is the
authored rope-swing landing hop, and it is enough for hop XP.

Later WalkTo intents match frozen helpers, not a rewritten route:

- spikes 14→13: `2800,9568` (`walkTrapStand` 5 tiles west of 2805,9568)
- saws 7→6: `2778,9557` (`walkTrapStand(7,6)` and the frozen unit test)
- wall/pillar stages are `edgeApproachCandidates` tiles; two missed
  inside 4000 ms, then click-from-here, and those hops still arrived

Tag never ran: `TagPillar.validate` requires `here === target`.
Targets were 22, then 1, then 13. The FAIL tile `[2783,9564,3]` is
on the 12↔7 wall footprint (wall loc 3565 @ 2783,9562), one hop
short of dispenser 13 (2794,9568), during a WalkTo toward 2784,9568.

Spinning blades fired on the saws attempt. That is not timed-blade
chat (`You were hit by the saw blade!`). Saws stay in `pathingEdges`
at 52 and are the unique shortest hop 7→1.

Climb-rope `still in the pit` is the same 06ad 8000 ms
`!inPit && !animating` wait. 274 subsequently hops on plane 3. 289
FAIL plane 0 is the third dump, not stale publication.

## 5. Bounded recommendation (root decides)

1. **Owner r289:** imported `nextHop` / `pathingEdges` 24-plank
   exemption / equal-rem `avoidHop` / reset-on-same-hop. Failure
   identity: `crossing plank 24→23` + loc 3572 @ 2802,9590 +
   `avoiding hop to 23` + immediate retry + `stat_xp_gain(16)>=1`
   FAIL. Root may dim `BrimhavenAgility` for that identity. Do not
   dim as a host loc or Agility-52 fixture miss.
2. **Owner r274 Tag:** same landing policy, then an ordinary 98-tick
   chase miss after hop XP, plus foreign saws preference. Not a Tag
   host gap. Keep the FAIL. Do not pass on hop XP alone.
3. Do not implement a Rust arena planner, do not seed tickets, do not
   force hint/timer, do not inflate 180s / 150.
4. If root wants a later diagnostic instead of dim: drop 24-plank from
   `pathingEdges` the same way other planks drop at 20+, **or** let
   avoid accept a +1-hop swing — in the frozen script, out of this
   audit. A 140-reach rerun is only for the two 274 stage-miss tiles
   with flood dequeue-index posted; it is not required to own these
   FAILs.
5. Keep 06ad as the Agility-1 snapshot. These native942 FAILs stay
   FAILs.

## 6. What this is not

Not a native data fidelity defect on swing 3566, plank3 3572, pillar
3578, wall 3565, ladder 3617, rope 3610, varp 309, or ticket 2996.
Not an unavailable WalkTo/Tag primitive. Not clock inflation. Not a
partial-progress PASS. Not compiled/startup accept. Do not rewrite
the foreign controller in this task. Do not invent a tick-end opcode.
