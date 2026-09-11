# Brimhaven live failure ownership (frozen cd49e147)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 13:51 UTC. Kind: bounded read-only diagnosis for
brief 129. Not implementation, compilation, LIVE, fixtures, client,
ledger, STATE, timeout inflation, or a foreign planner/route copy.
Root owns acceptance and any later source change. Own only this
report and `docs/compat/evidence/brimhaven-failure-audit/`. No new
tasks. Do not dim the cards.

Read once: `AGENTS.md`, `docs/execution.md`, brief 129. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `c0da29753c4bbf89da67df8411c708a84bd57a4d`. LIVE cells were
recorded on frozen host `cd49e147ade94bc5b06e7798e08ed4ad05a5d245` /
client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`. Work was read-only
except this report and the unique evidence dir. Frozen source is
`.superpowers/review-exports/catalog-root-cd49e147`. Concurrent WIP
was not touched.

All four catalog×revision receipts are present. Inspected only those
completed cells. No LIVE process was started.

## Verdict

**These four FAILs are an underqualified actor, not a broken imported
script and not a native swing/plank/pillar fidelity defect.** Do not
dim. Do not raise 180s / 150. Do not copy `nextHop` / `ARENA_EDGES`.
Do not treat saws, darts, or handholds as another known hop.

design05v / fixture05y seeded coins 995×1000, lobster 379×10, tele
`(2809,3194,0)` and left Agility 1 / HP 10. That is enough to **Pay
and Climb-Down**. It is not enough for the authored core (hop XP +
first Tag + ticket + subsequent XP) against selected 274/289 content.

Native facts (both revisions; 289 diffs are parrot chat only):

- Rope swing `stat_random(agility, 100, 310)` then `You missed the rope!`
  or `stat_advance(agility, 200)` (20.0 XP). No hard level gate.
- Pillar `stat_random(agility, 120, 340)` then `You lost your balance!`
  or `stat_advance(agility, 180)`. No hard level gate.
- Plank has **no** agility roll. `loc_find(coord, agilityarena_plank_broke1)`
  always dumps with `You stepped on a broken piece of plank!`.
- Handholds refuse below 20. Zone spikes/pressure 100% fail below 20.
  Saws/darts 100% fail below 40. Those are a harder tier, not “swing
  with a different name.”

Ash `STAT_RANDOM` (289 engine `cc359656` and current 274
`PlayerOps.ts`; live 274 object `4c95f87e` is not in the current
Server git store): `floor(low*(99-lvl)/98)+floor(high*(lvl-1)/98)+1`
then `random(256)`, success if value > roll. At Agility 1, swing
101/256 ≈ 39.5%, pillar 121/256 ≈ 47.3%. At 20: swing 141/256 ≈ 55.1%,
pillar 162/256 ≈ 63.3%. At 40: 184/256 ≈ 71.9% / 208/256 ≈ 81.3%.
At 52 (already the Wildy cheat): 209/256 ≈ 81.6% / 234/256 ≈ 91.4%.

Frozen `pathingEdges` already drops 20+/40+ edges when
`Skills.effective('agility')` is below `minLevel`. From landing
platform 24 at Agility 1 the ticket-grid BFS reaches only 11 of 24
dispenser islands (3,4,7,8,9,12,13,14,18,19,23) and uses the no-roll
plank as the 24→23 link. At 20 the same frozen graph connects 0–24
without walking saws/darts. That is actor setup, not a route-policy
change.

Climb-rope `still in the pit after climbing rope` is **not** proof
they remained in the pit. The wait is
`delayUntil(!inPitNow() && !Game.animating(), 8000)`. `inArenaPit`
is `level < 3`. Every FAIL snapshot is plane 3, one tile from a
Climbing rope 3610 landing. Native `[oploc1,agilityarena_climbingrope]`
climbs `movecoord(loc_coord, 0, 3, 0)`. The interact log’s `level: 0`
is the pit loc’s plane, not the player’s. Classify as legitimate
timing/animation (and over-broad log text), not stale host publication
and not a native loc defect.

## Classification against brief 129

| | Question | Applies? |
|---|---|---|
| First owner | Underqualified Agility 1 / omitted 20–40 seed | **Yes.** All four baselines agility 1, XP 0. design05v “Agility 1 is enough; do not seed 20/40 unless running steal” mixed Thieving steal gates with Agility hard gates. |
| Pay + descend worked | | **Yes.** Coins 1000→800, Climb-Down 3617, FAIL tiles in m43_149 plane 3. |
| Hop XP watch (step 11) | Three cells never gained agility XP | **Yes.** Chat is miss/broken-plank only. |
| 289/newer hop XP | One successful swing 24→19 | **Yes as observation.** Script `settled arrived platform 19`; step 12 started. Native success has no mes, only `stat_advance(200)`. |
| First Tag watch (step 12) | 289/newer never tagged | **Yes.** Then pillar 19→14 `You lost your balance!`. No `tag the next`. |
| Climb rope still-in-pit vs plane 3 | Native defect / stale tile | **No.** Wait includes `!animating`. FAIL tiles plane 3. |
| Native swing/plank/pillar fidelity | Wrong roll, missing XP, broken loc ids | **No.** Chat matches selected rs2. 289 vs 274 arena/clerk is parrot only. |
| Harder-tier edges as “another hop” | | **No.** Handholds 20; spikes/pressure 20; saws/darts 40; darts also `stat_drain`. Script already refuses them below `minLevel`. |
| Broken script / dim | | **No.** Byte-identical catalogs. Start loaded. hintTile consumed (`hint → pillar 9/4/23`). |
| Raise 180s / 150 | | **No.** Walls 60–78s, well under 180s. Watch 150 is the limiter. |
| Foreign planner / RNG / clocks / predicates | | **No.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `c0da29753c4bbf89da67df8411c708a84bd57a4d` |
| LIVE host (frozen cd49) | `cd49e147ade94bc5b06e7798e08ed4ad05a5d245` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Isolated export | `.superpowers/review-exports/catalog-root-cd49e147` |
| Catalog-boundary binary | sha256 `b36f1ace35790395f2df6d72162a3d98a50c3cb39535569412d7936ee2cd877e` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 129 SHA-256 | `89cef2666e0de0a195e86a4e4ee28bb5e73610e300baa53bb8bc445ac5f6a15f` |
| Kanban card | `t_434c2f2b` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine (receipt) | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| BrimhavenAgility.ts (both catalogs) | `771daff07bd4b3d6f2826ab1300d4fd66bcbae0f9d7a76e4a2ad07a4d050e859` |
| BrimhavenAgilityLogic.ts (both) | `fedf5f8e05fd43642efb0370352e71e5feebf258a784bc503a2e145c33b46d4d` |
| 274 agilityarena.rs2 | `c33bf541b9e5e557dd607b27bfe7e065d5c9bfe2c949a242585a6dde787335d5` |
| 289 agilityarena.rs2 | `dadc3599740ced8b9cd9db5feb4314163c0c63fbdc300fec60f10d99bdf1a9a7` |
| agilityarena_zones.rs2 (274=289) | `459895b3961bef9f4baf9baeb2f8e7179aab4cb17474d7ca87cb2e5303267f58` |

Machine-readable copies:
`evidence/brimhaven-failure-audit/{refs,hypotheses,cells}.json`.
Log sha256 values match the per-cell `*.json` receipts.

## 1. Four completed cd49 cells

Shared baseline-after-preparation: tile `[2809,3194,0]`, scene 2,
Coins 1000, Lobster 10, ticket 2996 absent, varp 309 = 0, agility 1 /
XP 0, hitpoints 10 / XP 1154. Start loaded. Card settings
`stealRestock=false`, `bankAtTickets=1000`. Pay posted (npc 4575 on
274, 4584 on 289). Climb-Down loc 3617. StatRow in FAIL evidence is
runenergy only.

| Cell | Exit | Wall s | Runner | Predicate | Tile |
|---|---|---|---|---|---|
| r274 old `100adccc` | 1 | 77.856 | FAIL 193 ticks / 74669 ms | step 11 `stat_xp_gain(16)>=1` | `[2805,9592,3]` |
| r274 newer `8e7d965b` | 1 | 73.005 | FAIL 176 / 69736 | same | `[2806,9586,3]` |
| r289 old `100adccc` | 1 | 75.952 | FAIL 224 / 72778 | same | `[2804,9591,3]` |
| r289 newer `8e7d965b` | 1 | 60.196 | FAIL 222 / 56966 | step 12 `chat(contains "tag the next")` | `[2807,9580,3]` |

r274 old: swing 24→19 miss ×3 (`You missed the rope!`), plank 24→23
broken, eat 1 lobster, `still in the pit` then FAIL on plane 3. Inv
800 coins + 9 lobster.

r274 newer: plank 24→23 broken first, `still in the pit`, swing 24→19
miss ×2, eat, FAIL mid-swing on plane 3. Inv 800 + 9 lobster.

r289 old: plank 24→23 broken (goal 23), `still in the pit`, **no
further hop logs**, FAIL on plane 3. Inv 800 + 10 lobster. Chat is
broken-plank + fee only.

r289 newer: swing miss, climb, `hint → pillar 9`, swing **arrived
platform 19**, pillar 19→14 `You lost your balance!`, climb rope
`(2806,9579,0)`, `still in the pit`, FAIL on plane 3. Inv 800 + 10
lobster. Hop-XP watch passed; Tag watch did not.

289 `models.bin` missing is a snapshot skip, not this owner: Pay,
descend, and obstacle chat still happened.

## 2. Legal documented preparation

Smallest owned fixture change, before Start only, same cheat family
as Wildy:

`advancestat agility 20`, drain that chat, keep `give coins 1000`,
`give lobster 10`, tele `(2809,3194,0)`. Do not `give` ticket 2996.
Do not seed 99. Do not seed after Start. Do not change inject,
`SCRIPT_GOLD_DEADLINE` 180s, `SCRIPT_GOLD_WATCH_TICKS` 150, RNG,
`ARENA_EDGES`, or acceptance predicates.

Why 20, with source facts:

- Frozen `ARENA_EDGES` already tags handholds/spikes/pressure
  `minLevel: 20` and saws/darts `minLevel: 40`. `usableEdges` /
  `pathingEdges` already filter on `Skills.effective('agility')`.
- Native handholds: `if(stat(agility) < 20) mes("You need an agility
  level of at least 20…"); return;`.
- Native spikes/pressure: `stat_random(...) = false | stat(agility) < 20`
  plus the same 20 mes; 100% fail below 20.
- At 20 the frozen BFS from 24 reaches every ticket pillar without
  taking saws or darts (darts stay filtered as a drain doom loop).
- 40 is the next documented gate (`stat(agility) < 40` on saws/darts).
  It is not required for connectivity of the current pathing filter.
  Root may pick 40 or reuse Wildy’s 52 for hop-roll reliability; 20
  is the minimum that matches native hard gates the script will walk.
- Steal 20/40 is **Thieving**, not Agility. design05v’s “do not seed
  20/40 unless running steal” does not apply to this card.

HP 10 stays. Fall damage is `(stat(hitpoints)*5/100)+2` = 2. Eat-at-5
already fired on the 274 cells.

## 3. Climb rope (timing, not fidelity)

Evidence: interact `Loc { x:2805, z:9591, level:0, action:"Climb",
id:Some(3610) }` (or 2806,9579 after the 19-island fall). Native
`p_arrivedelay; ~climb_ladder(map_findsquare(movecoord(loc_coord,0,3,0),…))`.
FAIL tiles `[2805,9592,3]`, `[2806,9586,3]`, `[2804,9591,3]`,
`[2807,9580,3]`.

Frozen `ClimbOutOfPit` logs “still in the pit” when
`!inPitNow() && !Game.animating()` does not hold within 8000ms.
`inPitNow` is plane < 3. A plane-3 FAIL snapshot contradicts “still
in the pit” as a world fact. Residual climb/get-up anim, or a second
click of the plane-0 rope from plane 3, is enough to explain the
line. Host evidence plane is 3, so this is not stale FAIL publication.

r289-old has no hop log after the second climb interact. That silence
is not a loc-identity defect; do not retarget 3610. Next snapshot
should print `tile.level`, `Game.animating` if posted, and whether
`ClimbOutOfPit.validate` is still true.

## 4. One controlled future diagnostic

Unchanged 180s / 150. Same catalogs × revisions. Only add
`advancestat agility 20` (optional paired cell at 40, not a timeout
stretch). Observe, do not pass on a weaker witness:

1. Baseline `stats[16].{effective,base,xp}` after the cheat (StatRow
   is runenergy-only today).
2. First hop: loc id/name/op/tile, chat miss vs broken-plank vs
   silent success, whether `stat_xp_gain(16)>=1` lands inside 150.
3. Hint pillar index vs `pathingEdges` reachability at the seeded
   effective.
4. Climb: player `tile.level` versus rope loc level 0, and whether
   “still in the pit” fires while plane is already 3.
5. First Tag: `tag the next` / varp 309 bits / ticket 2996 still 0.

Keep every current FAIL as a FAIL. A later PASS on this diagnostic
is not retroactive acceptance of Agility 1.

## 5. What this is not

Not a native data fidelity defect on swing 3566, plank 3572, pillar
3578, ladder 3617, rope 3610, varp 309, ticket 2996, or the 98-tick
hint cycle. hintTile was consumed. Pay worked. 289 parrot chat is
idle on the Pay op3 path these cells used. Do not rewrite the
foreign controller. Do not invent a tick-end opcode.
