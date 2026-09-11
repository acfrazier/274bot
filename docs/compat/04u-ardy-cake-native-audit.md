# Ardougne cake and pickpocket core failure ownership (frozen 4cc6cc27)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 09:24 UTC. Kind: bounded read-only architecture/fidelity
audit for brief 95. Not implementation, LIVE, fixtures, ledger, STATE, dim,
timeout, or a CakeStall controller copy. Root owns acceptance and any
authorized follow-up. No routine reviewer card.

Read once: `AGENTS.md`, `docs/execution.md`, brief 95. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report and `docs/compat/evidence/ardy-cake-native-audit/`. Frozen source is
`.superpowers/review-exports/catalog-root-4cc6cc27`, not the concurrent
working tree. Four other workers own runtime/UI/fixtures; those files were
not touched. Original six Ardy receipts were not copied or edited.

## Verdict

**The first blocking owner for ArdyCakes Flee, ArdyThiever Guard, and
ArdyThiever Knight is the shim `stealCakes` mapping, not Flee, not
`Game.inCombat`, not pickpocket NPC interact, and not a foreign-script
regression.** Do not dim either card. Do not clone `CakeStall.ts`. Do not
lengthen `SCRIPT_GOLD_DEADLINE`. Do not treat seeded Thieving/empty-pack
state as LIVE.

All six isolated old-catalog cells on frozen `4cc6cc27` died on the first
Thieving XP watch (`stat_xp_gain(17)>=1`, 150 ticks) with empty packs,
unchanged tiles, and no steal/pickpocket chat. Cakes logged the shim STAND
pin `(2661,3301,0)` then silence. Guard/Knight logged `restocking cake
(have 0)` ~149 times and never reached Pickpocket.

This is a **query/return-contract mapping defect** on existing
`Locs.query` / `Loc.interact` / `InteractReq::Loc`, plus a **shim world
pin** that replaced catalog stall/stand/flee tiles. It is not
`BLOCKED: missing loc-steal opcode`. The foreign 90s CakeStall loop,
chat classifier, and stand-swap are gameplay-controller behavior and
stay out of this slice.

## Classification against brief 95

| | Question | Applies? |
|---|---|---|
| First owner, Cakes Flee | Shim `stealCakes` | **Yes.** TaskBot validate-first reaches `StealCakes` (`nearMarket` vs pinned STAND r40). Flee never ran. |
| First owner, Guard/Knight | Same `stealCakes` via `RestockCakes` | **Yes.** `foodCount()<=restockAtFood=0` wins before Pickpocket. |
| Flee / `Game.inCombat` stuck | | **No.** No kite log. Cakes FAIL tile still `[2668,3312,0]`. Thiever restock requires `!inRealCombat()`. |
| Pickpocket NPC mapping | | **Not first.** `Npcs.query().name().action('Pickpocket').interact` already queues `InteractReq::Npc`. It never ran. |
| Foreign catalog regression | | **No.** Both catalogs are byte-identical (`cmp` 0) on ArdyCakes, ArdyThiever, CakeStall, cakeStallData. |
| Missing loc-steal opcode | | **No.** `Loc.interact` + host-play loc dispatch already exist (04i selected id). |
| Missing CakeStall controller | | Do not implement. Spell `classifySteal` as **BLOCKED: missing cakeStallData.classifySteal** and leave the honest stub. |
| Dim | | **No.** Missing host mapping must not hide the cards. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen host | `4cc6cc27d7048c4c60e6b0393ae92132d9b88a1f` |
| Frozen client | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/catalog-root-4cc6cc27` |
| Catalog-boundary binary | sha256 `0fb28e5ceafe9a3168a152ce67e695dd216f6f6a7c4b0e8850c063ceb0027f1a` |
| Campaign HEAD at write-up | `8bd34675b61c94707e6c5ed07a943102bf06cac5` (working tree; not the tested source) |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 95 SHA-256 | `50d9bbe85699442303c2323e95c7949900700f82e4c43d2aae736eb2c19ccd73` |
| Kanban card | `t_fa68337f` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| ArdyCakes.ts (both) | sha256 `f19ce19422c16e337fdc30678b93772054ee907a35297a1cd4a8f6fb062943d7` |
| ArdyThiever.ts (both) | sha256 `3cee6ff99d8f24532a5564fc7215faba19ca78f0156386913497bf266fca8aa5` |
| CakeStall.ts (both) | sha256 `156cf7143f42b5fbfb1ec580723f5d07928e4a174b3f65bdda91ba4b2ac384ef` |
| cakeStallData.ts (both) | sha256 `331570423b3c3730afa404b73909373b5150e2c85c799fe78bbf66b282516ccd` |
| Frozen `cake_stall.js` | sha256 `1342758c0fc8243ecb1e56a04f076054bb8d253665d56eced0457bbb2dea46be` |
| Frozen `cake_stall_data.js` | sha256 `8fe2b9d8092d81bbe31c46b3bb537bd667d0e584d5321b0a3b5cc86adcb08de5` |

Line hashes match `docs/compat/evidence/catalog-headed/source-4cc6cc27.json`.
Extra hashes: `docs/compat/evidence/ardy-cake-native-audit/hashes.json`.

## 1. Six isolated cells (immutable receipts)

Old catalog only, as retained. Newer-catalog Ardy cells were not in this
batch.

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv |
|---|---|---|---|---|---|---|
| r274 cakes | 1 | 114.429 | FAIL 165 ticks / 111307 ms | `stat_xp_gain(17)>=1` | `[2668,3312,0]` | empty |
| r289 cakes | 1 | 114.440 | FAIL 166 / 111285 | same, step 11 Baker's stall XP | `[2668,3312,0]` | empty |
| r274 Guard | 1 | 115.667 | FAIL 165 / 112495 | `stat_xp_gain(17)>=1` step 9 | `[2661,3306,0]` | empty |
| r289 Guard | 1 | 115.046 | FAIL 165 / 111824 | same | `[2661,3306,0]` | empty |
| r274 Knight | 1 | 116.234 | FAIL 165 / 113025 | same | `[2661,3306,0]` | empty |
| r289 Knight | 1 | 115.612 | FAIL 165 / 112385 | same | `[2661,3306,0]` | empty |

Chat at FAIL is the seed set (`Inventory wiped.` / `set tutorial: to 1000` /
`Welcome to RuneScape.`). No attempt/lockout/stun/pickpocket line. No
`not impl` on the steal path. Scene 2. CoreWitness: cakes
`ardy_cakes core post-Start delta incomplete`. These receipts are not LIVE
acceptance of a correction.

Cakes start log (both revisions): `stand (2661, 3301, 0)` — the shim pin,
not fixture STAND `(2668,3312,0)`. Guard/Knight start logs use
`targetSpot` `(2661, 3306, 0)` r19/r29 from host `PICKPOCKET_SPOTS`, then
`restocking cake (have 0)` 148–149 times.

## 2. TaskBot validate-first (exact)

Frozen prelude (`shim/mod.rs` 91–104):

```
async loop() {
    for (const task of this._tasks) {
        if (task.validate()) {
            await task.execute();
            return;
        }
    }
}
```

One validating task per 600 ms `loopDelay`. Later tasks do not run that
tick.

### ArdyCakes Flee (`ArdyCakes.ts` 179–191, 122–135)

Order: ContinueDialog, DeathRecovery, **Flee** (`Game.inCombat()`),
EatCake, SolveClue stub, BankRun, **StealCakes**, ReturnToStall.

Flee execute logs `combat — kiting the guard to ${FLEE_TILE}`. That line
is absent. FAIL tile is still the fixture stand. `Game.inCombat()` reads
`snap().in_combat === true` (`client_adapter.js` 90–92 / `game.js` 57–58).
If combat were stuck true, Flee would own the loop and the tile would
move toward the pin `(2661,3301,0)`. It did not.

StealCakes validate (`287–288`): `nearMarket() && !Game.inCombat() &&
!Inventory.isFull() && !died`. `nearMarket` is `STAND.distanceTo(here)
<= 40`. With shim STAND `(2661,3301,0)` and player `(2668,3312,0)`
Chebyshev 11, that is true. Execute calls `stealCakes({ fillTo: 28,
abort, shouldEat, lockedOutUntil, setStatus, log, onSteal, onReset })`
and only branches on `'combat'` / `'no-progress'`.

### ArdyThiever Guard / Knight (`ArdyThiever.ts` 177–206, 434–450)

Order: ContinueDialog, DeathRecovery, Flee (`inRealCombat()` =
`Game.inCombat() && !stun`), EatFood, PanicRetreat, SolveClue stub,
PeriodicBank Off, BankRun, **RestockCakes**, **Pickpocket**, LootDrops,
ReturnToAnchor.

RestockCakes validate: `nearMarket() && !inRealCombat() && !full &&
foodCount() <= RESTOCK_AT`. Fixture injects `restockAtFood=0`,
`foodTarget=1`. Empty pack → RestockCakes wins every loop. Execute logs
`restocking cake (have 0)` then `stealCakes({ fillTo: FOOD_TARGET, … })`.
Pickpocket validate also requires `foodCount() > RESTOCK_AT`, so even a
later mapping that only steals cannot pickpocket until one stall food
lands.

## 3. Shim stealCakes vs catalog call shape

Frozen `cake_stall.js` 13–16:

```
export async function stealCakes() {
    const loc = Locs.query().nearest();
    if (!loc || String(loc.name).toLowerCase() !== STALL_NAME.toLowerCase()) return false;
    return loc.interact(STALL_OP);
}
```

Catalog `CakeStall.ts` 22–34 / 55: `stealCakes(opts: StealCakesOptions):
Promise<'stocked'|'combat'|'aborted'|'no-progress'>` with `fillTo`,
`abort`, `shouldEat`, `lockedOutUntil`, `setStatus`, `log`, `onSteal`,
`onReset`. Foreign `stockedStall()` already filters
`.name(STALL_NAME).action(STALL_OP).where(l => l.tile().distanceTo(STALL_TILE) <= 3).nearest()`.

The shim:

1. Drops every option.
2. Uses unfiltered `Locs.query().nearest()` (any loc).
3. Returns a boolean. Catalog execute ignores it.
4. Does not walk, wait for item/XP, or classify chat.

That is sufficient to produce the cakes silence and the Thiever restock
spam. It is **not** permission to paste the foreign 90s loop
(`DEADLINE_MS`, `CLAIM_TIMEOUT_MS`, `classifySteal`, stand swap).

`Locs.query` (`query.js` 23–77) already supports `name` / `action` /
`where` / `nearest`. `Loc.interact` (`locs.js` 30–41) already queues
`op:'loc'` with selected `id`. Host-play dispatch (`host-play/src/lib.rs`
1864–1885) already matches tile + optional id and sends
`ActionSpec::Label`. Those ops are present. The shim does not use them
correctly.

## 4. cake_stall_data pin vs selected world

| Constant | Catalog `cakeStallData.ts` | Shim `cake_stall_data.js` |
|---|---|---|
| `STALL_TILE` | `(2667,3310,0)` | `(2661,3301,0)` |
| `STAND` | `(2668,3312,0)` | `(2661,3301,0)` |
| `STAND_ALT` | `(2669,3310,0)` | `(2661,3301,0)` |
| `FLEE_TILE` | `(2655,3298,0)` | `(2661,3301,0)` |
| `STALL_NAME` / `STALL_OP` | Baker's stall / Steal from | same |
| `classifySteal` | success/caught/lockout/refused/timeout | **BLOCKED: missing cakeStallData.classifySteal** |

`2661,3301` is not the Guard stand (`2661,3306`) and not the cakes
stand. Cakes start log proves the pin is live. Fixture seed is still
`(2668,3312,0)` / `(2661,3306,0)` (`scenario/src/lib.rs` 6008–6011,
`ARDOUGNE_GUARD`). Do not copy the catalog TypeScript table into the
shim as a foreign world file. Host already posts `PICKPOCKET_SPOTS`
(`api/src/content.rs` 135–149: Guard/Knight `(2661,3306,0)` leash
19/29). Stall/stand/flee belong on that same host-content seam, sourced
from selected engine/cache, not CakeStall.ts.

Selected engine loc (274 `Server` and 289 `lostcity-289`,
`[bakers_stall_stealing]`): `name=Baker's stall`, `op2=Steal from`,
2×2. Distinct `[bakerymarket]` is `name=Bakery stall` without steal.
Prior headed paint (not this LIVE binary) posted loc type **2561** at
`(2667,3310,0)` and a second Baker's stall at `(2655,3311,0)`. Item id
2561 in game-data is Ring of dueling; loc type ids are a different
namespace.

FAIL `loc_facts` list only doors. That is `keep_bounded_loc`
(`catalog_boundary_live.rs` 602–609: door/gate ids and names, distance
≤ 8, take 16). It does **not** prove the stall was absent from
`snap().locs`. This audit does not claim a 4cc6cc27 loc-observation
hole. If a later filtered interact still produces no OP_LOC, that is a
new snapshot/dispatch question, not the first owner.

## 5. Native pickpocket / combat facts (not first)

Generated `274.json` / `289.json` pickpocket groups (schema 3):

- Guard (`guard1` 9, `guard2` 10, `ardougne_guard` 32): level 40, 468 XP
  tenths, stun 8, coins 995 ×30.
- Knight of Ardougne (ids 23/26): level 55, 843 XP tenths, stun 8, coins
  995 ×50.

`npcs.js` 69–78 already queues `op:'npc'` with name, action, index.
`STUN_COMBAT_TICKS = 9` in `steal_rules.js` is a shim constant; generated
stun is 8. That mismatch is not this FAIL. `isHostileAttacker` /
Fight stay pending as 05m said. SolveClue remains the honest stub
(`validate() => false`).

## 6. Bounded implementation slice

Compatible with the shared queue: named-bank aliases → special →
teleport → shop → Make-X → fire. Cake steal is **loc mapping**, not
those cards. It may follow named-bank because both use existing walk/loc.
It must not insert a JS task-loop or cut the special/teleport/shop/Make-X/fire
line. `crates/host-play/src/lib.rs` is a hotspot; this slice should not
edit it unless a later filtered interact proves dispatch refusal.

### Own

- `crates/script/src/shim/cake_stall.js`
- `crates/script/src/shim/cake_stall_data.js`
- Optional host content row(s) for Baker stall/stand/flee posted like
  `PICKPOCKET_SPOTS` (selected cache/engine: name Baker's stall, op Steal
  from, stall tile `(2667,3310,0)`, stand `(2668,3312,0)`, flee
  `(2655,3298,0)`). Reject a second Baker's stall at `(2655,3311,0)` for
  this card unless a later fixture needs it.
- Unique isolate/unit tests only.

### Do not own

Catalog scripts, scenario fixtures, `SCRIPT_GOLD_*`, host-play dispatch,
Fight/`isHostileAttacker`, clue solving, timeout, dim, CakeStall.ts,
`classifySteal` policy tables.

### Mapping contract (preserve)

`stealCakes(opts)` must accept the catalog options object and return one
of `'stocked' | 'combat' | 'aborted' | 'no-progress'`.

- `aborted`: `opts.abort()` or `opts.shouldEat?.()`
- `combat`: `Game.inCombat()` after or instead of the interact
- `stocked`: `Inventory.isFull()` or `carriedCakes() >= (opts.fillTo ?? inf)`
- `no-progress`: no matching loc, interact refused, or one attempt that
  did not gain food and did not enter combat

One walk-then-interact per call is mapping onto existing
`Traversal.walkTo` / `Loc.interact`. TaskBot re-enters next loop. Do not
inner-loop 90s. Do not subscribe to chat. Do not swap STAND/STAND_ALT.
Do not invent success/refusal counters. `onSteal` / `onReset` /
`setStatus` / `log` may be called only for events this mapping actually
observes (food gain; not fabricated resets).

Cakes first LIVE steal: player is already on catalog STAND adjacent to
`(2667,3310,0)`. Filtered interact is the minimum. Thiever restock from
`(2661,3306,0)` is Chebyshev 6 from that stall; a query-only interact
without approach may still fail after this mapping. Approach via existing
walk to the **host-posted** stand is in scope; a new steal state-machine
is not.

### Unsupported / honest stubs

- **BLOCKED: missing cakeStallData.classifySteal** — keep `notImpl`.
- Fight / `isHostileAttacker` — pending, not this slice.
- SolveClue — stub, `solveClues=false` in these cells.
- PeriodicBank Off — not the bank proof; ArdyThiever BankRun at
  `bankAtLootSlots=1` remains the bank cycle after pickpocket works.

## 7. Verification (for the later implementer; not run here)

No compilation, LIVE, or source edits in this audit.

Meaningful isolate regressions (behavior, not source snapshots):

1. Unfiltered nearest that is not Baker's stall must not queue a loc op
   and must return `'no-progress'`.
2. Name+action(+host stall tile) match queues `InteractReq::Loc` with
   action `Steal from` and selected loc id when posted.
3. Options object is accepted; boolean `true`/`false` is not a legal
   result.
4. `classifySteal` is not invoked on the mapping path.
5. `carriedCakes` / `needsCakeRestock` still count cake/bread/chocolate
   slice only (not chocolate cake 1897).

Root's unchanged-window LIVE falsifier (original `SCRIPT_GOLD_DEADLINE`
180s / watch 150):

- Cakes: stall steal + Thieving XP, deposit acquired stock, pack empty of
  cake 1891, bank closed, return to STAND, further steal.
- Guard/Knight: actual coins 995 + Thieving XP (stall-only XP/cakes do
  not satisfy), deposit coins, empty pack of coins, return, further
  pickpocket.

Do not claim this source audit or the six FAIL receipts as that LIVE.

## Limits

This audit did not dump `snap().locs` on the FAIL ticks. Door-only
`loc_facts` cannot settle stall observation. A later filtered interact
that still sends nothing is a new card. Newer-catalog Ardy LIVE was not
in the six receipts; catalog sources are identical, so the same shim
owner applies, but those cells still need their own raw logs before a
pass.
