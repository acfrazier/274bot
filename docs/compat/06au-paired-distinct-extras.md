# Bound remaining distinct paired options

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 18:28 UTC. Kind: bounded read-only audit of brief 158.
Own only this report and `docs/compat/evidence/paired-distinct-extras/`.
No product, tests, LIVE, STATE, ledger, support-matrix, queue mutation, or
new cards. Root owns acceptance and any later fixtures after 157.

Read once: `AGENTS.md`, `docs/execution.md`, brief 158, 05w, briefs
126/127/156/157, 06as paired-seam list. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Concurrent working-tree edits
are not a tested binary. Operator mid-run: do not infer UNAVAILABLE from
a members label or a long route; keep every distinct enabled branch;
equivalent altars are not extra cells; unimplemented host travel is a
native gap, not a foreign defect.

## Verdict

**Three named extra pair scenarios (twelve catalog×revision cells), gated
after the queued default pairs and their natives.** Do not mark PASS.
Do not dim. Do not copy `shopdb` / `shopOpBatch` / boat tables. Do not
invent a market or mule scheduler.

| Named extra | Distinct because | Represented by |
|---|---|---|
| `nature_island` | Nature `unnote=Jiminua`: noted restock, Talk-to shop, `Shop.sell`/`buy`, spider-safe walk, Barnaby boat. Air `unnote=null` cannot accept this. | this extra |
| `nature_stay_in_altar` | `stayInAltar=true`: trade at `z>4000`, runner talisman, master skips portal, runner `ExitAltarForRestock`. | Air setting, not Nature×island cartesian |
| `mule_bankfill_false` | `CrafterGoBank` only resets `tradedMules`; crafter never banks essence. Mule still deposits runes and restocks. | Air + `bankFill=false` |

**Non-Air Mule is equivalent altar content, not a fourth extra.**
`MuleCrafter.ts` has no rune-name branch. `runeCraftLocations` only
substitutes rune/talisman/level/named-bank/ruins. Same Trade +
`Bank.openBooth(bankTile(name))` + talisman `useOn` + `Craft-rune` +
portal. F2P Mind/Water/Earth/Fire/Body stay represented by queued Air.
Members Cosmic/Chaos/Nature/Law stay the same loop at other tiles; host
travel for Zanaris door, Entrana boats, Karamja boats, and wilderness
walk already exists. Do not multiply those altars. Do not substitute
Mule `Nature rune` (Draynor hop) for NatureCrafter island.

`04-capabilities-shop.md` is still absent. Shim `Shop.sell` throws.
That is queued 38, the real caller being `UnNoteEssence`.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `25d4fafc6c4c7b8e717fe08bfe57d95457856955` |
| Client gitlink | `52c37f9ce50d1f184656d5b4469c007ec8a5791a` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 158 SHA-256 | `3525f7487b349d226da48f62d427b77b3b9a047584b7ee483598fc6caf63fa22` |
| Brief 126 SHA-256 | `9ce80c5e8506ddead0873017b03c878df4b288df845dd92df25278766d11ef80` |
| Brief 127 SHA-256 | `cd5ef5666607752422ba301fbce5965cbaf5c179b483cf7bd4dc37d74c8f8036` |
| Brief 156 SHA-256 | `0d8b23afdef1bb9d8850cca4932433533bfddc84cf4f2ff79fa4fcbbf430bfe7` |
| Brief 157 SHA-256 | `86eacc13bbfad3ab3f64027d7c14c49ffdb0b8d1ce88fa191ace94fc14f8c7de` |
| Brief 38 SHA-256 | `2a9376367a1e5fc171291c1aadf543b8bc2fa5a0de71920b906e6210984a8df0` |
| 05w SHA-256 | `a274f61c0ec1881963890852c92d9c10886ad514b18e9f742bc164f9aa91e83d` |
| Kanban card | `t_acea973c` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Generated items | `crates/api/data/game-data/{274,289}.json` schema 3 |

`cmp` is 0 on both catalogs for NatureCrafter, NatureRunnerLogic,
MuleCrafter, MuleCrafterLogic, runeCraftLocations, and frozen `Shop.ts`.
Card/API SHA-256 values: `evidence/paired-distinct-extras/source-hashes.json`.

Selected 274 provenance `engine_commit=4c95f87efe`,
`content_commit=000c19997e`, `npc.dat` sha256
`112b08855d7b37c9faf36d6c37f354addbdbf886fe2d54a78a7ff832c8015cf9`.
Selected 289 provenance `engine_commit=cc359656b4`,
`content_commit=92649430fc`, `npc.dat` sha256
`29b8e82b63df203e50ba376a1b9c01cbf9c734eabbfcc45444d887adf664baea`.
Generated game-data has items, not NPC rows. Jiminua is a live NPC
identity to observe, not a missing generated table.

## Method

A second pair cell is a different control path, product flow, or
required failure — not another altar tile. Numeric knobs
(`bankEvery`, `withdrawEss`, `withdrawCoins`, `minFlaxCapacity`) are
not cells. Same generated Trade/bank/craft on another rune row is
substitution. Members labels and long walks are not UNAVAILABLE.
Unavailable requires selected-world missing content or a missing
operation at a real caller. Unimplemented host travel is a native gap.
Do not copy foreign helpers to close a gap.

Reuse the existing paired harness, 180 s prep / 180 s gold / 150 watch
ticks, shared Start, no post-Start cheats. Seeded first packs are not
restock. First transfer is not full. Harness must not Trade or click
`gobank`.

## 1. Nature island / Jiminua (distinct, required)

Default `rune='Nature runes'`. `RUNES['Nature runes']`: ruins
`(2865,3022,0)`, runner bank Ardougne East `(2655,3283,0)`, master bank
Shilo `(2852,2954,0)` quest-gated, `unnote={npc:'Jiminua',
tile:(2767,3122,0)}`. Air is `unnote=null` and Falador East both banks.

Runner tasks that Air never adds: `PickupNotedEssence`, `UnNoteEssence`.
`BankRestock` on the noting route: `Bank.setNoteMode(true)`, withdraw
the full noted stack (or `withdrawEss`), coins to
`max(3000, withdrawCoins)`, `LOW_COINS=1000` or stop. `walkTo` inserts
`SPIDER_SAFE (2790,3094,0)` between store and ruins. `openUnnoteShop`
is `Talk-to` + `ChatDialog.chooseOption()` / `continue()`, **not**
`Shop.open('Trade')`. Then `planStoreStep` → `Shop.sell('Rune essence',
n, id!==1436)` and/or `Shop.buy('Rune essence', n)` for at most
`STORE_PASSES=6`, then `Shop.close`. Ladder Climb-up/down at the store
is existing loc interact (foreign `jiminuaStoreLadder` tests; do not
copy).

There is no Barnaby API in the script. The ship is
`Traversal.walkResilient` over host boat edges. Nav already has
Captain Barnaby npc 381 Ardougne `(2679,3275,0)` → Brimhaven ship,
fare `(995, 30)`, 7 ticks; return Customs officer 380. That travel is
implemented, not a native gap and not a dim.

`bankEvery=60` will not fire in 180 s. Shilo is not required. Master
`BankEverything` already fails that walk in 30 s and continues.

Shim `Shop.sell` throws (`0c532288…`, unchanged vs 05w). Frozen
`Shop.sell` uses player pack `SHOP_PLAYER_COM=3823`, 10/5/1
`shopOpBatch`, 3000 ms + 1 tick. 38 must own sell **and** buy settle.
Do not copy `shopOpBatch` into JS. Do not fill native `SHOP_DB={}`.
Frozen catalog `shopdb.junglestore` is foreign: keeper Jiminua,
`allstock=true`, baseline without rune essence. The live unnote is
sell-into-allstock then buy posted essence, not a copied item list.

Generated IDs (274 = 289): unnoted essence `blankrune` 1436
(`certificate_link=1437`), noted `cert_blankrune` 1437, Nature rune
561, Nature talisman 1462 (`members=true`, not a disposition), coins
995.

## 2. stayInAltar (distinct, required; Air representative)

Setting default false. Runner `onStart` waits 3 s for
`conf.talisman` and stops if missing. Master `CraftNatures` skips
`portalOut` when true. `masterShouldEnterAltar` enters with empty
essence to wait. `WaitForRunner` idles in-temple. Runner
`DeliverEssence` calls `enterAltar` when not in-temple.
`ExitAltarForRestock` portals out only when unnoted==0 and trade
closed. Help text says “nature talisman”; code uses `this.conf.talisman`.

Air + `stayInAltar=true` is the enabled branch without also running
island shop. Nature + stayInAltar is the same altar control on the
island route — do not cartesian.

Temple test is `Game.tile().z > 4000`. Seeded temple occupancy is not
a trade.

## 3. Mule bankFill=false (distinct, required)

Help: “When disabled, mules bring all essence (blank rune mode). Only
applies to Crafter mode.” Solo forces `bankFill=true`. Mule mode
ignores the flag.

`CrafterGoBank.execute`: if `!fillBank()`, `resetTradedMules()` and
return — no walk, no deposit, no essence withdraw. `CrafterAtBank`
still withdraws if the crafter is standing at the bank; prep must
start the crafter at the ruins so that path does not fire.

Mule `MuleGoBank` still deposits received runes and withdraws 27
unnoted 1436. Full cycle is mule bank/return/second transfer, with
the crafter staying at the ruins.

05x deadlock still applies if the first window is essence-only (mule
left at ruins with 0 ess 0 runes; `MuleGoBank` does not validate).
Prep the 127 first unnoted load on the crafter so the first craft
produces runes **before** the mule offers essence. That seed craft is
not the pair cycle. Do not seed 556.

## 4. Non-Air Mule (equivalent content, not an extra)

No `if (rune === …)` in `MuleCrafter.ts`. Table only:

| Setting | bank | ruins | level | talisman id | members talisman |
|---|---|---|---|---|---|
| Air rune (queued) | Falador East (3013,3355,0) | 2983,3288,0 | 1 | 1438 | false |
| Mind rune | Edgeville | 2980,3511,0 | 2 | 1448 | false |
| Water rune | Draynor | 3182,3162,0 | 5 | 1444 | false |
| Earth rune | Varrock East | 3303,3477,0 | 9 | 1440 | false |
| Fire rune | Al Kharid | 3310,3252,0 | 14 | 1442 | false |
| Body rune | Edgeville | 3050,3442,0 | 20 | 1446 | false |
| Cosmic rune | Draynor | 3173,9501,0 | 27 | 1454 | true |
| Chaos rune | Edgeville | 3060,3585,0 | 35 | 1452 | true |
| Nature rune | Draynor | 2865,3022,0 | 44 | 1462 | true |
| Law rune | Catherby (catalog BANK_LOCATIONS; not host `named_banks` CANDIDATES) | 2858,3378,0 | 54 | 1458 | true |
| Death rune | Edgeville | 3221,3218,0 | 65 | 1456 | true |

Host `named_banks::CANDIDATES` are the five F2P Mule banks. Mule
resolves `bankTile` from frozen `BANK_LOCATIONS`, not posted host
aliases. Catherby exists on that catalog table. Cosmic travel: nav
Zanaris shed door (Lost City + worn Dramen) is implemented. Law:
Entrana boats npc 657/658 implemented. Nature ruins from Draynor uses
existing Karamja boats, not Jiminua. Chaos is a wilderness tile, not a
new op.

Death's table tile `(3221,3218,0)` is host Lumbridge
(`walk_destinations` test). That is not a members exclusion and not a
foreign defect. Do not use Death as a non-Air representative. Missing
`Mysterious ruins` at that tile would be a LIVE content mismatch, not
a dropped Mule card.

## Native coverage vs queued work

| Caller | Op | Status |
|---|---|---|
| `UnNoteEssence` | `Shop.sell(name, n, pick id!==1436)` | missing; shim throws; **38** |
| `UnNoteEssence` | `Shop.buy(name, n)` 10/5/1 settle | **38** (shim queues one if-button) |
| `openUnnoteShop` | `npc.interact('Talk-to')` + `ChatDialog.chooseOption/continue` | mapped; island does not call `Shop.open` |
| `BankRestock` noting | `Bank.setNoteMode` | mapped when note buttons posted |
| Nature/Mule Trade family | request/offerAll/accept/decline | **126** |
| Nature runner `onPaint` | `Paint.buttons` gobank | headed **157**; do not auto-click |
| Flax `driveActivePartnerTrade` | unused by these extras | **156**, not this set |
| Island ship | `walkResilient` + nav Barnaby 381 | implemented host travel |
| stayInAltar enter/portal | talisman `useOn` / Portal `Use` | mapped |
| bankFill=false | no new op; crafter skips `CrafterGoBank` walk | fixture after **127**/**157** |

No uncovered missing Rust operation beyond 38's sell/buy. Do not
propose a second shop stack or a boat capability card.

## Finite fixture proposals (not evidence)

Each named scenario is both catalogs × 274/289. Gold/watch/Start
unchanged. Root validates setup before LIVE.

### `nature_island` — after 38 + 126 + 157

Inject: `rune='Nature runes'`, Master + Runner, exact minted names,
`stayInAltar=false`. Leave `bankEvery=60`, `withdrawEss=0`,
`withdrawCoins=10000`.

Master: RC 44, pack Nature talisman 1462 only, start ruins
`(2865,3022,0)` r4. No 561, no 1436.

Runner: Ardougne East booth ack `givebank cert_blankrune 1437 ×200`
and coins 995 ×10000, open+loaded+close+generation, then place at
Jiminua `(2767,3122,0)` r3 holding noted 1437 (TRADE_CAP 25) and coins
≥3000, **no unnoted 1436**. Empty of natures.

Witness full (`IslandShopBankSecondCycle`): after Start, Jiminua shop
open via Talk-to (not Trade), noted 1437 down **and** unnoted 1436 up
from `Shop.sell`/`buy` (queued if-button / unchanged inv fail);
bilateral offer+confirm with the minted counterpart; 1436 left the
runner; master 561 ≥ 1 and RC XP ≥ 1; then Ardougne East note-mode
withdraw of a **new** 1437 load (first 25 is seed), coins topped,
closed bank, second Jiminua unnote, second transfer/craft. Seeded 561
or pre-Start unnoted 1436 fails. Shilo / `bankEvery` must not be
waited.

Timing: first Barnaby hop is pre-Start by placing the runner at
Jiminua. Restock still needs one boat round trip + second shop inside
180 s. If gold expires after shop+first craft, print explicit partial
`IslandFirstUnnoteCraft` — that is not full and not acceptance.
Bounded decomposition if the return cannot fit: serialize a second
cell that starts the runner empty at Ardougne East with banked 1437
(same clocks, same no-post-Start rule) whose witness is note-withdraw
+ boat + second unnote + second transfer. Neither cell alone
qualifies the branch.

### `nature_stay_in_altar` — after 157 Air pair + 126

Inject: `rune='Air runes'`, Master + Runner, `stayInAltar=true`.

Master: RC 1, Air talisman 1438, ruins `(2983,3288,0)`, no essence.
Runner: Air talisman 1438 **required**, Falador East ack `givebank
blankrune 1436 ×200`, firstload 25 unnoted at ruins. Do not seed 556.

Witness full: both offer+confirm **inside** `z>4000`; 1436 left the
runner; master 556 ≥ 1 and RC XP ≥ 1 **without** master portal-out
before that craft; runner portals out after empty; Falador East
restock of a new 1436 load; runner re-enters altar; second inside
trade/craft. Overworld-only Air pair fails this cell. Missing runner
talisman is a fixture stop, not a dim.

### `mule_bankfill_false` — after 126 + 127 + 157

Inject: `rune='Air rune'`, Crafter + Mule, `bankFill=false`.

Crafter: ruins, talisman 1438, 127 first unnoted 1436 ×27, no 556, no
RC XP. Mule: Falador East ack 1436 ×200, firstload 27 unnoted at
ruins, partner=crafter IGN.

Witness full: bilateral rune-for-essence (crafter already holds
script-crafted 556 when the mule offers 1436); conservation; crafter
does **not** open Falador East after Start; mule deposits received
556, withdraws a new 1436 load, second transfer, further crafter
craft. Crafter bank open, seeded 556, or first-transfer-only fail.
Solo `bankFill` force-true is a fixture miss.

## Queue so root can serialize

Do not merge these extras into 157's three minimum cells.

1. **157** headed default pairs (Nature Air, Mule Air `bankFill=true`,
   Flax after 156). 126/127 remain their owners.
2. **38** then `nature_island`. Also the ShopBuyout core from 154.
3. **`nature_stay_in_altar`** after Air full-cycle (or in parallel as
   offline witnesses). Same Trade family.
4. **`mule_bankfill_false`** after 127 bootstrap + Air mule pair.
5. Non-Air Mule: no extra fixture card. Air remains the altar
   representative.

156 is unused by these three. 37/39/40 unused.

## Exclusions

- Flax `minFlaxCapacity`, Nature `bankEvery`/`withdrawEss`/`withdrawCoins`.
- Mule Nature as a stand-in for Jiminua.
- Death/Cosmic/Law/Chaos extra pair cells (equivalent loop).
- Copying `shopdb.junglestore`, `shopOpBatch`, or boat NPC tables.
- Dimming NatureCrafter or MuleCrafter. No source-proven foreign defect.
- Treating remaining-production's Nature-as-core as still binding; 05w/158
  made Air the pair representative.

## Falsifiers

Wrong if a later worker: accepts Air as island; substitutes Mule Nature
for Jiminua; cartesian stayInAltar×island or bankFill×Mind; counts
seeded 1436/1437/556 as shop unnote, restock, or conversion; prints
first-transfer as full; auto-clicks `gobank`; copies `drivePartnerTrade`
or `shopOpBatch`; implements a second boat API while Barnaby 381
already exists; marks Cosmic/Law/Nature Mule UNAVAILABLE because
`members=true`; dims for `Shop.sell` throw; claims these proposals are
LIVE evidence; post-Start tele/`give`.
