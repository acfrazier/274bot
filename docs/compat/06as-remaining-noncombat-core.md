# Remaining noncombat core gaps and finite fixtures

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 17:51 UTC. Kind: bounded read-only audit of brief 154.
Own only this report and `docs/compat/evidence/remaining-noncombat-core/`.
No product, LIVE, build, STATE, ledger, support-matrix, or queue mutation.
Root owns acceptance and any later cards. These fixture rows are
source-verified proposals, not evidence.

Read once: `AGENTS.md`, `docs/execution.md`, brief 154, queued 37–40,
153, 05w, 126/127, 06ap, 04-combat-production-design.md, remaining-production
section A for the five cards. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Concurrent working-tree edits
are not a tested binary.

## Verdict

**Eleven named solo scenarios (forty-four catalog×revision cells), gated
on 153 plus the matching 37–40 native, then the existing solo
CoreWitness pipeline.** Do not mark PASS. Do not dim. Do not copy
foreign teleport/shop/fire/leather tables. Do not invent a router.

Covered by queued natives, once 153 releases headed/host-play overlap:

| Card | Queued native | Core host ops |
|---|---|---|
| AIO Teleport | 37 | `Game.teleport` press + Magic XP **and** destination tile. Not a nav walk. Not queued if-button. |
| ShopBuyout | 38 | `Shop.open` / `buy` 10/5/1 settle / `close`. Posted stock and inv move. |
| SmithingBot | 39 | `isMainMakePanel` / `mainMakeProducts` / `makeFromPanelMax` from posted anvil rows. Not chat `make_products`. |
| Firemaker | 40 | `lightFire` XP or `CANT_LIGHT`; host next-tile in posted `FIRE_PLOTS`; tinderbox restock only. |
| LeatherCrafter | none of 37–40 | Default is needle `useOn` + frozen leather-interface `ifButton`. Fixture-gated after 153. |

**`BuyoutLogic.buyoutPlan` is ancillary, not core.** Exact call is
`ShopBuyout.BuyoutPass.plan` → `buyoutPlan(rec, stockByObj, coins, chosen)`
only when `SHOP_DB` has the keeper. Native `shopdb.js` is `export const
SHOP_DB = {}`, so `rec` is always null and the script buys all posted
stock in shop order. Do not fill `SHOP_DB` or copy `unitPrice`. `Shop.buy`
is the enabled operation (38).

BankSorter: operator decision this run keeps it
`UNAVAILABLE_BY_OPERATOR_DECISION`. Exclude from implementation and
qualification. Do not import a sorter planner.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `af9513900f7181db6dbfb878dc2837b3c2bc403e` |
| Client gitlink | `52c37f9ce50d1f184656d5b4469c007ec8a5791a` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 154 SHA-256 | `81a25d4cfddec4af865c092f4a5e4e09e96eddd65d1ad7f332685a5daa5b8105` |
| Brief 37 SHA-256 | `7d112106ede7c34aee14144df1b9abf34fe1e314a9f830ea8e62bf2cfddf7cf1` |
| Brief 38 SHA-256 | `2a9376367a1e5fc171291c1aadf543b8bc2fa5a0de71920b906e6210984a8df0` |
| Brief 39 SHA-256 | `0ba57a309eb1cd19f594255e86305a2a7e9cb9a9e17fe2ff1c820feac27ae6c1` |
| Brief 40 SHA-256 | `40f14e56318a1bc6e4e1ef9f05b9bfde576398e2150f36c47bea1569c0550775` |
| Brief 153 SHA-256 | `e50c61271ecfbce481b28fea1c0bf32ba89b17811b296ed79a6a695ec60516a2` |
| Kanban card | `t_8f13bdf1` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Generated items | `crates/api/data/game-data/{274,289}.json` schema 3, alias/id match on every row below |

Card/API SHA-256 values:
`evidence/remaining-noncombat-core/source-hashes.json`. `cmp` is 0 on
both catalogs for every five-card source and the frozen APIs they call.
37–40 reports (`04-capabilities-{teleport,shop,production,fire}.md`)
are absent. 153 report `06ar` is absent.

## Method

Enabled core is the frozen default that can finish inside the existing
180 s / 150-tick / 600 ms gold window, with bank/return/further work.
A second cell is a different control path, product flow, keeper, or
failure the caller actually requires. Numeric knobs (`lawBatchSize`,
`budgetGp`, `threadPerTrip`, log/bar tiers after one representative)
are not extra cells. Same generated action on another content row is
substitution. Unknown selected content is unavailable, not a drop.
Do not copy foreign helpers to close a gap. Selected-world buttons,
shop packs, anvil rows, and fire plots come from cache/host content.

Reuse `script_live_seed_steps` / `start_catalog_step` /
`CoreWitness` (extracted by 153). Headed `catalog_watch` must use that
same witness; scenario PASS alone is not core PASS (06ap). Original
clocks. No post-Start cheats. No manufactured PASS. No synthetic
widget trees. No content-ID universality.

## Native coverage vs 37–40

### 37 `Game.teleport` — AIO Teleport core

Shim `Game.teleport` throws. Posted `spell_buttons` are
`button_type==2` target spells only (`host-play` filter). Frozen
`Game.teleport` is `resolveTeleport` + `ifButton`; comment says
success is dispatch, not arrival. AIO Teleport then treats the boolean
as a cast and counts Law down. 37 must press the **non-target**
spellbook button from selected cache metadata and observe Magic XP
**and** tile change. A queued if-button or a seeded destination tile
fails.

Do not copy AIO `CONFIG.teleports` / `CONFIG.staffs` or frozen
`Teleport.ts` 2004 fallbacks (Varrock 1164, …). Those are script
policy and a static fallback. Host facts are selected-cache teleport
buttons and requirements.

Also unblocks FireGiant/GreenDragon tele-escape already named in 145.
Do not redo 145 here.

Staff substitution is script-local. `useStaffRunes=false` is a real
extra because the cost must come from pack runes.

AIO canvas keyboard Withdraw-X / canvas bank-close are foreign DOM.
Not host work. Restock uses existing `Bank.openNearest` +
`Bank.withdraw` All fallback (first family).

### 38 `Shop.open/buy/close` — ShopBuyout core

Shim `Shop.buy` queues one `if-button` and optional `answer-count`
without 10/5/1 looping or held-count settle; return is undefined.
Frozen `Shop.buy` returns bought count, batches 10/5/1, ≤5 user-ops
per tick, 3000 ms + 1 tick. `Interactions::shop_buy` exists but does
a single 1/5/10 guess and is unused. 38 must own the pending.

`Shop.sell` / `buyById` have no ShopBuyout caller. `buyById` stays
`not impl` (38). `Shop.sell` stays shop-owned for Nature island /
Brimhaven, not these five cards.

Hardcoded frozen `SHOP_ROOT` 3824 / stock 3900 / player 3823 are
selected-interface identities to verify, not universal IDs. Do not
copy `shopOpBatch` into JS.

### 39 makeX + smithing panel — SmithingBot core

Shim `isMainMakePanel` / `mainMakeProducts` / `makeFromPanelMax` all
throw. Isolate has chat `make_products` only. SmithingBot default is
bar `useOn` Anvil then those three. 39 posts selected-world main-modal
rows and presses the largest posted Make-N. Missing controls refuse.

`ChatDialog.makeX` (count-dialog wait, one Answer-Count) is 39 but
**not** a SmithingBot or default LeatherCrafter call. It remains the
gate for already-owned Smelter/FlaxSpinner and for FlaxRunner
conversion (05w). Do not add SmithingBot makeX cells.

### 40 `lightFire` + next-tile — Firemaker core

Shim `lightFire` is tinderbox `useOn` logs with no XP/`CANT_LIGHT`
wait and returns boolean. Frozen returns `'lit'|'blocked'|'stalled'`
with `FIRE_START_TICKS=14` and `FIRE_LIGHT_TICKS=150` from identical
`Firemaking.ts`. Read those bounds; do not invent timeouts.

Firemaker still calls `findBurnLane`, `inFirePlot`, `burnLaneWant`,
`isBurnWest`, `runInDir`, `fireReactionTicks`, `tileKey`,
`NoLightTiles`, `toolRestockPlan`. 40 must map those enabled calls as
thin shims over host AABB next-tile + refused set + ordinary tinderbox
withdraw. Do **not** clone the foreign west-lane ranker. Honest
`findBurnLane` shape: one walkable posted-plot tile with no Fire loc
and not in the refused set (run may be 1). Shim `NoLightTiles`
currently extends `Error`; Firemaker constructs it as a refused set —
40 must restore that shape.

`bestPickaxe` / gatherer Tools stay `not impl`.

Posted `FIRE_PLOTS` Varrock East AABB is **not** the frozen
`FIRE_SPOTS` table. Frozen: x 3232–3284, z 3428–3430. Native
`api::content` : x 3235–3275, z 3418–3432. Runtime `FIRE_SPOTS` is
already `host().content.fire_plots`. Fixtures witness the **posted**
plot.

### LeatherCrafter — not 37–40

Default `leatherType='Leather'` `flow='interface'`: needle `useOn`
leather, wait `reader.modals().main === 2311`, `ifButton` make10 8636
(gloves). `useOn` and `ifButton` already exist. 39's anvil
`mainSkillMultiItems` is a different modal. The script does not call
`ChatDialog`.

Hardcoded 2311 / 8636 / 8638 are catalog-local. Do not claim they are
universal. Do not add a leather DSL or copy `LEATHERS`. If selected
289 modal/coms differ, that is unavailable/mismatch at LIVE, not a
new queued native. Default core is therefore **fixture + 289 identity
check**, gated on 153 only.

`flow='single'` hard-leather burst is a distinct extra (no panel).
Dragonhide `multi3` + count dialog waits 39 and is **not** in the
minimum set.

### Required-but-unassigned

None for these five cores beyond 37–40 + 153, except:

- ShopBuyout / SmithingBot are TaskBots. Brief 152 source landed
  during this audit as `3bf3644ad` (`TaskBot.loop` awaits `validate`).
  Their own `validate()` is sync. Do not re-audit 152. LIVE still
  waits 152 review/acceptance, not a fifth missing op.
- `buyoutPlan` / `SHOP_DB` / named `buyItems` filter: ancillary.
- AIO DOM Withdraw-X / canvas close: ancillary.
- Leather interface IDs: selected-world verification, not a new API.

## Finite fixture proposals (not evidence)

Gold window, clocks, and Start rules stay as remaining-production
shared contracts. Each named scenario is both catalogs × 274/289.
Root validates setup before LIVE.

Generated IDs (274 = 289): `lawrune` 563, `airrune` 556, `firerune`
554, `waterrune` 555, `staff_of_air` 1381, `staff_of_water` 1383,
`coins` 995, `vial_empty` 229, `hammer` 2347, `bronze_bar` 2349,
`bronze_dagger` 1205, `bronze_platebody` 1117, `needle` 1733,
`thread` 1734, `leather` 1741, `leather_gloves` 1059, `hard_leather`
1743, `hardleather_body` 1131, `logs` 1511, `oak_logs` 1521,
`tinderbox` 590.

### AIO Teleport — after 37 + 153

1. `aio_teleport` (core). Default `teleportName=progressive`. Magic 25
   → Varrock (`spellName` `'Varrock'`, Law 1 + Air 3 + Fire 1). Wield
   1381 (`useStaffRunes=true`). Pack 563 ×2 and 554 ×20 (staff covers
   Air, not Fire). Bank 563 ×200. Start Lumbridge bank (3092,3245,0)
   r8 so arrival is a real tile change. Empty of product XP.
   **Correction vs remaining-production:** do not start empty-pack with
   2000 banked laws and default `lawBatchSize=1000` — that withdraws
   1000 and never restocks in 180 s. Two packed laws empty the start
   batch without injecting `lawBatchSize`.
   Witness after Start: Magic XP ≥ 1 **and** tile outside Lumbridge
   bank radius **and** 563 down ≥ 1. Then Varrock booth (3252,3420)
   deposit, withdraw laws, closed bank, **second** teleport (further
   XP or second move). Seeded Varrock tile / queued if-button fail.

2. `aio_teleport_falador`. `teleportName=falador`, Magic 37, wield
   1383, Water from staff. Landing ≠ Falador-already. Distinct
   elemental cost + destination button.

3. `aio_teleport_no_staff`. Default progressive/Varrock, Magic 25,
   `useStaffRunes=false`, pack 556+554+563. Equipped staff must not
   satisfy Air. Distinct control path.

Named `lumbridge` at Magic 25 is the same Varrock-or-fail path as
progressive (progressive picks Varrock). Camelot/Ardougne/Watchtower
are the same `Game.teleport` on another posted button; Falador is the
non-Varrock representative. If a selected spellbook has no button,
record unavailable content. `lawBatchSize` / `minLawRunes` are not
cells. Failure paths (absent control, no laws, already at landing,
Pause/Stop during wait) are 37 composed tests, not extra LIVE
scenarios.

### ShopBuyout — after 38 + 153 (and 152 prelude)

1. `shop_buyout` (core). Default Aemad, inject `budgetGp=2000`,
   `perTripGp=2000`, `stopFloorGp=0`, `buyItems=[]`. Start (2613,3294,0)
   r6. Bank 995 ×20000. Empty pack.
   Witness: Aemad shop open, some posted stock count down **and**
   matching inv up **and** 995 down. Prefer generated `vial_empty` 229
   if that name is on posted stock; do not fail the cell solely because
   rec-null bought another posted row first. Then deposit except coins,
   withdraw coins, second buy. Queued if-button / unchanged stock fail.

2. `shop_buyout_aubury`. Preset Aubury, start (3253,3401,0), Varrock
   East bank. Same rec-null all-stock `Shop.buy`. Distinct keeper /
   shop / bank. **Not** a `buyItems` filter cell — that path needs
   `SHOP_DB` + `buyoutPlan` (ancillary).

Lundail / Wizard Guild / Mage Arena: selected-content, not core.
`recheckSeconds` is not a cell.

### SmithingBot — after 39 + 153 (and 152 prelude)

1. `smithing_bot` (core). Default `bar=Bronze`, `product=Dagger`.
   Smithing 1. Bank 2347 ×1 and 2349 ×28. Start Varrock West
   (3185,3440,0) r8. Empty pack. Anvil (3188,3425,0).
   Witness: Smithing XP ≥ 1 **and** 1205 ≥ 1 **and** 2349 down after
   the **anvil panel**, not chat make. Then deposit except hammer,
   `withdrawLoad` bars, further 1205. Seeded daggers fail.

2. `smithing_bot_platebody`. `product=Platebody` (5 bars), product
   1117. Distinct panel row and bar drain. Do not also change bar tier
   on that cell.

Other products/bar tiers are the same `makeFromPanelMax` on another
posted row.

### LeatherCrafter — after 153; 289 modal check at LIVE; not 37–40

1. `leather_crafter` (core). Default `leatherType='Leather'`. Crafting
   **1** (so `pickRecipe` stays gloves, not body). Bank 1733 ×1, 1734
   ×100, 1741 ×28. Start Al-Kharid (3269,3167,0) r8. Empty pack.
   Witness: Crafting XP ≥ 1 **and** 1059 ≥ 1 **and** 1741 down. Then
   deposit except needle/thread/leather, withdraw 1741, further gloves.
   Seeded gloves fail. Do not treat 2311/8636 as 289-universal; if the
   selected modal never matches, that is LIVE unavailable/mismatch.

2. `leather_crafter_hard_body`. `leatherType='Hard leather'`, Crafting
   28, 1743, product 1131, `flow='single'` burst. Distinct no-panel
   path. Do not run dragonhide `multi3`.

`threadPerTrip` is not a cell. Other soft-leather recipes are the same
interface flow on another frozen button.

### Firemaker — after 40 + 153

1. `firemaker` (core). Default `logType='Logs'`, `location='Varrock
   East'`. Firemaking 1. Bank 590 ×1 and 1511 ×28. Start posted plot
   bank (3253,3420,0) r8. Empty pack.
   Witness: Firemaking XP ≥ 1 **and** a Fire loc inside the **posted**
   Varrock East AABB. Walking the bank tile is not a light. Then
   deposit except tinderbox, withdraw logs, further light. useOn
   without XP/`CANT_LIGHT` fails.

2. `firemaker_oak`. `logType='Oak logs'` 1521, Firemaking 15. Distinct
   log id. Same posted plot. Do not also vary location.

Other log names / other `FIRE_PLOTS` names are AABB substitution after
one plot is witnessed.

## Differences vs remaining-production (explicit)

| Item | remaining-production | This audit |
|---|---|---|
| AIO restock in 180 s | Empty pack, 2000 banked laws, default batch 1000 | Pack 2 laws so the start batch empties; do not inject `lawBatchSize` as a cell |
| ShopBuyout witness | Always `vial_empty` 229 | 229 if posted; rec-null may buy another posted row first |
| ShopBuyout `aubury` | `buyItems` one rune name | Keeper/shop/bank only; named filter is ancillary |
| Firemaker AABB | Frozen `FIRE_SPOTS` 3232–3284 / 3428–3430 | Posted native `FIRE_PLOTS` 3235–3275 / 3418–3432 |
| LeatherCrafter gate | “production interface buttons” as if 39 | Not 39; 153 + 289 identity |
| BankSorter | Fixture still described | Operator UNAVAILABLE; out of this set |

Source hashes for the five cards match remaining-production prefixes
(`AIOTeleport` `8b811edb6ee2…`, others identical across catalogs).

## Queue so root can serialize

Do not merge 38 with 39 or 39 with 40 (04 design). 06ap: do not start
37 (or other host-play natives) until 153 extracts shared CoreWitness.

1. **153** shared headed witness. Reuse for every cell below.
2. **152** TaskBot prelude source is `3bf3644ad`; ShopBuyout /
   SmithingBot LIVE waits its review, independent of 37–40.
3. **37** then AIO Teleport fixtures. Also releases 145 tele extras.
4. **38** then ShopBuyout fixtures. Also releases Nature island
   `Shop.sell` (paired extra, not these cells).
5. **39** then SmithingBot fixtures. Also releases already-owned
   Make-X LIVE and FlaxRunner conversion.
6. **40** then Firemaker fixtures. Also releases CookBot `surface=Fire`
   (already-owned CookBot extra; not in this eleven).
7. **LeatherCrafter fixtures** after 153; no 37–40 wait. LIVE still
   checks 289 leather modal.

Mule 127 may proceed in parallel (paired files only). Option 147 waits
153 (already stated). No new generic router.

## Paired seams (list only; 05w / 126 / 127; not re-audited)

Outstanding from 05w + those briefs, not this eleven:

- NatureCrafter Air pair: existing cell; headed `Paint.buttons` was the
  108 stall; full cycle still needs observed trade + restock + second
  transfer. Island / Jiminua / `Shop.sell` extra after 38. `stayInAltar`
  extra.
- MuleCrafter Air pair: 127 bootstrap fixture + 126 native trade.
  `bankFill=false` and non-Air runes extra. Solo Mule does not qualify.
- FlaxRunner pair: missing `driveActivePartnerTrade` + 39 Make-X for
  conversion. First transfer without spin is partial only.
- 153 known limit: paired full-core needs paired-slot orchestration.
  Do not claim paired acceptance from solo observation.
- Default vs extra: Air/Falador and Seers flax remain the pair
  representatives; island/shop/stayInAltar/bankFill=false are extra.

## Exclusions

- BankSorter: operator `UNAVAILABLE_BY_OPERATOR_DECISION`. No substitute
  value/ID ordering. No foreign planner import.
- Combat / resource already in 139 / 147–150 / 153 / 145. CookBot Range
  is 05q. CookBot Fire is 40 on that owned card, not a sixth base card.
- No LIVE. No product edits. No STATE/ledger/matrix writes.

## Falsifiers

Wrong if a later worker: copies `buyoutPlan` / `SHOP_DB` / `TELEPORTS` /
`findBurnLane` / `LEATHERS`; treats queued if-button as teleport arrival;
uses chat `make_products` for SmithingBot; witnesses frozen fire AABB
instead of posted `FIRE_PLOTS`; multiplies lawBatchSize/budget/log-tier
cells; claims LeatherCrafter needs 39; claims `buyoutPlan` is core while
`SHOP_DB` is empty; runs BankSorter; redoes 145; labels these proposals
as LIVE evidence; manufactures PASS with post-Start tele/`give`.
