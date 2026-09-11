# ArdyCakes 274 guard / return failure ownership (native942)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 15:55 UTC. Kind: bounded read-only diagnosis for
brief 142. Not implementation, compilation, LIVE, fixtures, client,
ledger, STATE, timeout inflation, food/XP seeding, forced guard/RNG,
or a CakeStall controller copy. Root owns acceptance and any later
source change. Own only this report and
`docs/compat/evidence/ardy-guard-return-audit/`. No new tasks. Do not
implement dim. Do not edit 139.

Read once: `AGENTS.md`, `docs/execution.md`, brief 142. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `6dd6f9102b1111fdcadf551ccfd3a24658d81661`. LIVE cells were
recorded on frozen host `9429357d1124a9a3451c33e2fae1725fd5296be0` /
client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`. Host cake mapping
(`crates/{api,script}/src/cake_stall.rs`, shim `cake_stall.js`) is
unchanged versus that host. Work was read-only except this report and
the unique evidence dir. Frozen catalogs are
`.superpowers/inputs/rs2b0t-{100adccc037d9f6898080e1cad58fcfc43364775,8e7d965be2071d6ec65c3265e12af797082d720a}`.
Concurrent WIP was not touched. No LIVE process was started.

## Verdict

**274 no-first-Thieving-XP is a native Baker-stall guard catch, not a
host loc / WalkTo / observation-identity defect and not a foreign
catalog regression.** The first `Steal from` loc 2561 at `(2667,3310,0)`
from stand `(2668,3312,0)` returns `combat`. Engine
`~stealing_check_for_guard` hunts `ardougne_guard` within 5 tiles + LOS
and retaliates **before** reward or `loc_change`. There is no
`stat_random` on this stall. HP 40→37, empty pack, thieving XP stuck at
388. Flee `Walk` to host `FLEE_TILE` `(2655,3298,0)` arrives. Post-combat
lockout then `WalkTo` stand, then the same loc, then catch/kite again.
FAIL mid-return at `[2657,3300,0]` when the 150-tick first-XP watch
expires. That is interrupted return of a working walk, not a stuck
WalkTo.

**289 is not the same failure.** Same host sequencer and byte-identical
`ArdyCakes.ts` steal bread at tick 47 (`thieving` 388→404) and finish
with cake + 2 chocolate slice (`thieving` 452 = +64 = four 16-XP
steals). It FAILs later on `arrived_near(2655,3286,0,6)` with a 3-item
pack. That is the empty-28-slot fill/bank watch, which 139 ballast
prepares and does not claim to fix 274.

Do not dim ArdyCakes. Do not clone `CakeStall.ts`. Do not raise 150.
Do not treat 214 isolate termination as fixed.

## Classification against brief 142

| | Question | Applies? |
|---|---|---|
| 274 first XP | `stat_xp_gain(17)>=1` | **No XP.** 165 ticks / 91610 ms. Tile `[2657,3300,0]`. Inv empty. Chat seed only. |
| Guard caught → flee 2655,3298 | native catch + Flee | **Yes.** 3 kites. Each `Walk` arrives. |
| Lockout / return loops | 10-tick lastcombat + host lockout | **Yes.** After each arrive, ~9× `no-progress` then `WalkTo` 2668,3312. |
| Host loc identity | 2561 vs other stall 2655,3311 | **No.** Every steal is loc 2561 @ 2667,3310, op `Steal from`. |
| WalkTo / walkOpening identity | return vs flee | **No.** Flee is `Walk` (nav-follow, arrived). Return is stealCakes `WalkTo` → `ix.walk`. 289 returns to stand and steals. 274 FAIL is on that path, not at the flee tile. |
| ReturnToStall task | `STAND.distance > 40` | **No.** Flee cheb 13. Task never validates. |
| Stale / missing baker facts | `facts_valid` / depleted op | **No.** Loc commands fire. Catch is not empty-stall: engine returns before `loc_change(market)`. |
| Native observation 134 | in_combat / inv / here | **Combat observed.** First and third cycles return `'combat'`. Inv/XP never move. 274 baseline `npc_facts=[]` at tick 18 is a snapshot hole, not the steal click. |
| Actor Thieving 5 / HP 40 / Flee | fixture minimum | **Qualified for the stall.** Level 5 is the dbrow requirement. Stall success is LOS, not a thieving roll. HP 37/40 is one guard hit, not death. |
| Ordinary guard / world | hunt r5 + LOS | **Yes, this owner.** Deterministic catch when a guard sees the click. |
| Proven foreign defect | imported policy | **No.** Both catalogs `cmp` 0 on ArdyCakes / CakeStall / cakeStallData. 289 first-XP passes with the same script. Caught does not stand-swap in the foreign loop either. |
| 289 slow fill ≡ 274 no XP | | **No.** 289 has food/XP. Different predicate. |
| 139 Knife ballast | 6 food slots | **Not this fix.** Do not edit 139. |
| 140 budgeted reach | flood / maxSteps | **Not this owner.** Known tiles, Walk/WalkTo, both arrives. |
| 214 isolate termination | | **Not reproduced.** Do not claim fixed. |
| Raise 150 / seed / force RNG | | **No.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `6dd6f9102b1111fdcadf551ccfd3a24658d81661` |
| LIVE host (native942) | `9429357d1124a9a3451c33e2fae1725fd5296be0` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Catalog-boundary binary | sha256 `51c4e645ba7d7f0c1b0ab1df35e624358d2a794a18748f2b16d77f9448a624d6` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 142 SHA-256 | `e329545b5329d7c4a8e0325900e8d98cea1e056047a9df8d64ed8747b35f762d` |
| Kanban card | `t_ad570a8a` |
| Old catalog (LIVE) | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog (source only) | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine (receipt) | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 274 content | `000c19997e07206131bcb3c884265840efce416d` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| 289 content | `92649430fcbc83538d8c4367ecb96cee1a67a944` |
| ArdyCakes.ts (both) | `f19ce19422c16e337fdc30678b93772054ee907a35297a1cd4a8f6fb062943d7` |
| CakeStall.ts (both) | `156cf7143f42b5fbfb1ec580723f5d07928e4a174b3f65bdda91ba4b2ac384ef` |
| cakeStallData.ts (both) | `331570423b3c3730afa404b73909373b5150e2c85c799fe78bbf66b282516ccd` |
| r274 log SHA-256 | `c61486cae3603f8fae65b2062341beb4e7606e1207995ad47ff52ff9f04d6402` |
| r289 log SHA-256 | `0672490b67cc743ea5118b7da4c54cce51fee4d7f681b2855a65248fc3327ecd` |

Machine-readable copies:
`evidence/ardy-guard-return-audit/{refs,hypotheses,cells,hashes}.json`.
Log sha256 values match the per-cell `*.json` receipts.

## 1. Two completed native942 old-catalog cells

Shared baseline-after-preparation: tile `[2668,3312,0]`, scene 2,
empty pack, Thieving base/effective 5, XP 388, Hitpoints 40 / XP 37224,
varp 309 = 0, `guardResponse=Flee`, `solveClues=false`. Start loaded.
Host pins match catalog `STAND` / `STALL_TILE` / `FLEE_TILE`.

| Cell | Exit | Wall s | Runner | Predicate | Tile |
|---|---|---|---|---|---|
| r274 old `100adccc` | 1 | 94.801 | FAIL 165 ticks / 91610 ms | step 11 `stat_xp_gain(17)>=1` | `[2657,3300,0]` |
| r289 old `100adccc` | 1 | 78.749 | FAIL 251 / 75557 | step 13 `arrived_near(2655,3286,0,6)` | `[2668,3312,0]` |

r274 events: loc×4, `guard caught`×2, kite×3, flee `Walk`×3 (all
Arrived), stand `WalkTo`×3, `no-progress`×29, eat×0. Latest: thieving
388, items `{}`, `local_health` 37, `in_combat` false, `ardy_cakes_cycle.stolen`
null. Guards at FAIL (player 2657,3300): 2660,3308 cheb 8; **2659,3303
cheb 3**; 2664,3301 cheb 7. Those are return-path tiles, not a stall-time
proof for click 1.

r289 events: loc×14, `guard caught`×2, kite×2, flee Arrived×2, stand
`WalkTo`×2, eat Bread, four steal chat lines. First food at tick 47 on
STAND (`Bread` 2309, thieving 404). Latest: thieving 452, items Cake 1 +
Chocolate slice 2, `local_health` 36, still on STAND. Guards at latest
all cheb 6–7 from STAND (outside hunt 5). Bank never approached.

Elapsed times are not performance evidence.

## 2. Causal chain (274)

TaskBot order: ContinueDialog, DeathRecovery, **Flee** (`Game.inCombat()`),
EatCake, SolveClue stub, BankRun (`Inventory.isFull()`), **StealCakes**,
ReturnToStall (`distance>40`).

1. Start on STAND. StealCakes validate true. Host one-attempt
   `stealCakes` is already on stand → loc 2561.
2. 274 engine `~steal_from_stall`: guard hunt hits → `npc_say` +
   `~npc_retaliate(0)` → return. No XP, no item, stall not emptied.
3. Host `WaitSteal` sees `in_combat` within 2400 ms → `'combat'`. Script
   logs `guard caught the steal`. Flee walks to 2655,3298, waits
   `!inCombat` (15 s bound), `markCombatEnd`.
4. Engine also blocks steal while `lastcombat+10 > map_clock`. Host
   `lockedOutUntil = combatEndTick + 10` returns `'no-progress'`
   immediately (does not wait). That is the post-arrive spam.
5. Lockout expires. Not on stand → `WalkTo` STAND. Then loc again.
   Cycle 2: loc, `no-progress`, loc, `no-progress`, then Flee without a
   second `guard caught` line (combat rose after the 2400 ms bound).
   Cycle 3: loc → `guard caught` → kite. Watch expires during the third
   return at 2657,3300.

Host mapping matches the posted one-attempt contract (04w): lockout and
combat skip dispatch; one food below fillTo 28 is honest `no-progress`;
`onReset` is never called. It is not 90 s CakeStall equivalence and does
not need to be.

## 3. Engine stall (274 selected content)

`stealing.dbrow` `[stealing_bakery_stall]`: loc `bakers_stall_stealing`
= pack id **2561**, owner `baker_merchant`, guard **`ardougne_guard` only**,
level 5, experience 160 (16.0 XP), respawn 8 ticks, loot cake 13 / bread 5
/ chocolate_slice 2 over 20. Stall string field count is 1, so there is
no `You attempt to steal` mes. Catch is overhead `Hey! Get your hands
off there!`.

`~steal_from_stall` does not roll thieving. Owner check can also abort
without combat; 274 click 1 returned combat and lost 3 HP, so it is the
guard path.

289 baker dbrow header is the same hunt rule. 289 `stealing.rs2` /
`thieving.rs2` differ later (extra stalls, login bits); the baker catch
predicate is still hunt r5 + LOS + not `opplayer2`.

## 4. What 289 shows, and what it does not

289 first loc clicks also `no-progress` then a catch, then flee/return,
then **successful** steals from the same STAND loc. Guards at the stolen
snapshot are cheb 5–7 from STAND. The same one-attempt sequencer can
clear first XP when the hunt misses. 274 never got that window inside
150 ticks.

289 FAIL with 3 foods is the remaining 28-slot fill against a 150-tick
bank-arrival watch. Not proof 274 would fill. Not a reason to dim.

## 5. Recommendation

One bounded correction: **none in host steal mapping and none in the
imported script.** Keep the honest 274 FAIL. Do not dim. Do not clone
the foreign restock/stand-swap loop. Do not raise the first-XP watch,
seed cake, suppress guards, or inflate Thieving.

If root requalifies 274, keep Thieving 5 / HP 40 / Flee / real food/XP.
A later cell can still steal when r5+LOS is clear; this cell did not.
139 ballast remains a 289 pack-cycle preparation, not a 274 no-XP fix.

Optional later evidence, not this task: npc_facts at steal dispatch so
click-1 hunt tiles are posted. 274 baseline `npc_facts=[]` at tick 18
is insufficient to reconstruct that hunt, not insufficient to classify
the catch.

140 cannot be blamed from these walks. 214 isolate termination is
absent; do not close it from these cells.
