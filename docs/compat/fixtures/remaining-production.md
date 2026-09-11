# Remaining production catalog fixtures

Architect: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: section A of brief 49. Not LIVE, not source,
not support-ledger, not capability redesign.

Branch checked first: `codex/rs2b0t-multirevision` (HEAD at write
`30193c3b5a3e`). Catalogs
`.superpowers/inputs/rs2b0t-{100adccc037d9f6898080e1cad58fcfc43364775,8e7d965be2071d6ec65c3265e12af797082d720a}`
are byte-identical for every named card below except Superheater
(both `.ts` files differ; SHA-256 prefixes in the card). Generated
facts: `crates/api/data/game-data/{274,289}.json` schema 3. Query by
`alias`. Combat/NPC type tables are **not** in those assets. Do not
invent OSRS IDs or quest gates the script does not encode.

Capability architecture stays in `04-combat-production-design.md` and
`04-provisioning-recovery-design.md`. This report only names the next
shared scenario + CoreWitness cells. Reuse `script_live_seed_steps`,
`start_catalog_step`, `drain_advancestat`, `gold_script_nav` (600 ms),
`SCRIPT_GOLD_DEADLINE` 180 s, `SCRIPT_GOLD_WATCH_TICKS` 150. Do not
raise product timeouts. Bank-travel observation may use the existing
BoneBurier 240-tick window when a script walk is in play.

## Shared contracts

| Rule | Contract |
|---|---|
| Catalogs | Superheater is the only section-A source split. Do not invent an 8e7d-only mode on any other card. |
| Five first cards | Alcher / ChickenKiller / Thiever / BoneBurier / BankFletcher already have core fixtures. Do not re-audit them. |
| Start | After `ingame && scene_state==2`, last seed wait, and `StartScript`. Baseline is the first post-seed observation **before** Start. |
| Proof | Script-caused XP / inventory / world change after Start. Seeded items, queued sends, and constructor `validate===false` are not proof. |
| Clue | No card here constructs `SolveClue`. Leave it alone. |
| PeriodicBank / DeathRecovery | Unused on these cards unless a card table says otherwise. Do not inject combat bankStrategy cells. |
| Loadout | Only HerbloreSecondaries exposes `loadout`. Blank uses `Lobster` fallback (`FOOD_DEFAULT`). Named loadout waits on the queued loadout store. |
| IDs | Witness generated `alias`/`id`, not display names alone. Rune essence is `blankrune` 1436, noted 1437. Coins 995 not 617. Dragonfire shield is `antidragonbreathshield` 1540. |
| Cheats | `give` / `givebank` / `advancestat` / `tele` / `setvar tutorial 1000` as existing catalog cells. Do not treat debug `inv_add` as normal acquisition. |
| Cartesian | One core + the distinct option cells below. Do not cross fish × location × surface, rune × mode, or every dart tier. |
| 180 rows | These cells do not drop or add enabled cards. |
| Incomplete rewrite | An existing deferred native rewrite cannot be claimed supported by a fixture that skips its required behavior. |

### Shared generated item IDs (274 = 289 for these rows)

| Display | Alias | ID | Notes |
|---|---|---:|---|
| Law / Air / Fire / Water / Earth / Nature rune | `lawrune` 563, `airrune` 556, `firerune` 554, `waterrune` 555, `earthrune` 557, `naturerune` 561 | | Exact alias, not board-game pieces |
| Staff of air / fire | `staff_of_air` 1381, `staff_of_fire` 1387 | | Unnoted. Noted `cert_*` +1 |
| Rune essence | `blankrune` | 1436 | Script hardcodes 1436. Noted `cert_blankrune` 1437 |
| Air / Earth / Nature talisman | `air_talisman` 1438, `earth_talisman` 1440, `nature_talisman` 1462 | | Noted +1 |
| Coins | `coins` | 995 | Not `fake_coins` 617 |
| Raw salmon / Salmon | `raw_salmon` 331, `salmon` 329 | | CookBot default |
| Logs / Tinderbox | `logs` 1511, `tinderbox` 590 | | Fire surface |
| Guam leaf / unidentified | `guam_leaf` 249 / unid 199 | | HerbCleaner; confirm alias in card |
| Dragonfire shield | `antidragonbreathshield` | 1540 | HerbloreSecondaries white berries |
| Lobster | `lobster` | 379 | Loadout food fallback |

### Invalid fixture vs missing capability

A **fixture** failure is: wrong tile/level, wrong NPC/loc **name**, missing skill/quest/item the script itself stops on, empty trade partner on a Master/Runner card, or a seed still in the baseline. A **capability** failure is: script starts, reaches the action, and the host throws / no-ops the mapped operation (`Game.teleport`, `Game.castOnItem`, `Trade.request`/`accept`, `Shop.buy`/`sell`, `ChatDialog.make`/`makeX`, `lightFire`, `isMainMakePanel` / `makeFromPanelMax`, `sortBank`, `Bank.withdrawLoad`, `Banking.open`). First failure in each card names which.

---

## 1. AIO Teleport

Source: `AIOTeleport/AIOTeleport.ts` (identical, sha `8b811edb6ee2`).

| | |
|---|---|
| Core behavior | Default `teleportName=progressive`. At Magic 25 that is Varrock (`spellName` `'Varrock'`, cost Law 1 + Air 3 + Fire 1). `Game.teleport` then bank restock of Law runes via `Bank.openNearest('Bank booth','Use-quickly')`. Progressive stops at Camelot (lvl 45); Ardougne/Watchtower are named options only. |
| Catalog diff | None. |
| Seed | One mainland account. tutskip/relog. `advancestat magic 25`. Wield `staff_of_air` 1381 (`useStaffRunes=true` default). Bank `lawrune` 563 ×2000 (`lawBatchSize` default 1000, `minLawRunes` 100). Inv `firerune` 554 ×50 (staff does not cover Fire). `tele` Lumbridge bank (3092,3245,0) r8 so the Varrock landing is a real tile change. |
| Skills | Magic 25. No quest check in source for any destination. |
| Baseline | Tile at Lumbridge bank. Magic XP frozen. Law/Fire counts as seeded. Magic tab not required to be open. |
| Core witness | After Start: Magic XP ≥ 1 **and** tile no longer in the Lumbridge bank radius **and** `lawrune` down by ≥ 1. A queued if-button or a seeded Varrock tile fails. |
| Bank cycle | After laws in pack drop, `Bank.openNearest` at the destination bank (Varrock 3252,3420), `depositInventory`, `withdraw('Law rune','Withdraw All')`, then another teleport. Withdraw-All of seeded laws is not the cycle; a second teleport after restock is. |
| Options | `tele_falador`: `teleportName=falador`, Magic 37, Water source (wield `staff_of_water` or pack `waterrune` 555); landing ≠ Falador-already. `tele_no_staff`: `useStaffRunes=false` with Air+Fire packed; staff must not satisfy the cost. Do not also vary `lawBatchSize`. `ardougne`/`watchtower` exist as labels (lvl 51/58); source encodes **no** quest. If the selected spellbook has no button, that is selected-revision unavailable content — do not invent a 274-only drop. |
| Gate | Spellbook teleport (`Game.teleport` press + tile/XP). First-family `Bank.openNearest` for the restock cell. |
| Fixture vs capability | Magic 24 / no laws / start already at the Varrock landing with no observable move → fixture. Teleport dispatched, no Magic XP or tile change / throws → capability. |

---

## 2. RuneCrafter

Source: `RuneCrafter/RuneCrafter.ts` (identical, sha `a944049b42ee`). `ESSENCE='Rune essence'`, `ESSENCE_ID=1436`, `RUINS='Mysterious ruins'`, `ALTAR={name:'Altar', op:'Craft-rune'}`, `BOOTH={name:'Bank booth', op:'Use-quickly'}`, `TEMPLE_Z=4000`.

| | |
|---|---|
| Core behavior | Default `rune=Air runes`, `mode=Solo`. Bank Falador East (3013,3355,0) → withdraw unnoted essence 1436 → walk ruins (2988,3294,0) → talisman `useOn` Mysterious ruins → `Craft-rune` on Altar → Air rune up, RC XP → portal out → deposit. |
| Catalog diff | None. |
| Seed | One account. RC 1. Air talisman 1438 in pack or bank. Bank `blankrune` 1436 ×200 (not 1437). Start Falador East bank r8. Empty pack except talisman. |
| Skills | RC 1 for Air. Earth option needs RC 9. |
| Baseline | Overworld z<4000. 0 Air runes. Essence still in bank (or 0 in pack). |
| Core witness | After Start: temple z≥4000 observed **or** `airrune` 556 ≥ 1 and RC XP ≥ 1, then a bank deposit of those runes. Seeded Air runes fail. |
| Bank cycle | Solo `BankTrip` **is** the loop. Witness deposit of crafted `airrune` then a second essence withdraw. `givebank` essence is seed, not the cycle. |
| Options | `earth_solo`: `rune=Earth runes`, RC 9, Earth talisman 1440, ruins (3303,3477,0), Varrock East bank (3253,3420,0). `runner_mule`: two profiles, `mode=Runner` + `mode=Mule Recipient`, `partner` = recipient IGN, both Air, recipient camps altar (`ALTAR_PARK=2`), runner `TRADE_LOAD=26`. Trade: unnoted 1436 leaves runner pack and appears on recipient, then recipient crafts. Do not cross Earth × Runner. |
| Gate | Loc interact + item-on-loc (existing). Trade family for the runner cell (`Trade.request`/`offer`/`accept`; accept throws if `trade_accept_id<0`). First-family booth open. |
| Fixture vs capability | No talisman / noted essence 1437 only / wrong ruins tile → fixture. Inside temple, Altar op no-ops, or Trade window never posts → capability. |

---

## 3. NatureCrafter

Source: `NatureCrafter/{NatureCrafter,NatureRunnerLogic}.ts` (identical). Default `rune=Nature runes`, `mode=Master`, `partner=''`, `bankEvery=60`, `stayInAltar=false`. No Solo mode. `ESSENCE_ID=1436`. Nature ruins (2865,3022,0); runner bank Ardougne East (2655,3283,0); master bank Shilo (2852,2954,0) *commented* “needs the Shilo Village quest”; unnote NPC `Jiminua` (2767,3122,0). Air option: ruins (2983,3288,0), Falador East both banks, `unnote=null`.

| | |
|---|---|
| Core behavior | Two-profile Nature: Master crafts at the ruins/altar; Runner banks noted essence, un-notes at Jiminua (`Shop.sell` noted 1437 / `Shop.buy` 1436), trades unnoted 1436, Master `Craft-rune`. `bankEvery=60` min will **not** fire inside the 180 s gold window, so Shilo is not required for core craft proof. |
| Catalog diff | None. |
| Seed | Two mainland accounts. Both RC 44. Both hold Nature talisman 1462. Master: start ruins (2865,3022,0) r4, empty pack except talisman. Runner: Ardougne East (2655,3283,0) r8, bank `cert_blankrune` 1437 ×200, `coins` 995 ×10000 (`withdrawCoins` default), `partner` = master IGN. Master `partner` = runner IGN. Boat/Jiminua must exist on the selected world. |
| Skills | RC 44 Nature. Air option RC 1. Shilo quest is a source comment on the timed bank, not a Start check. |
| Baseline | Master at ruins, 0 Nature runes, 0 unnoted essence. Runner has noted essence in bank, trade closed. |
| Core witness | After both Start: Trade delivers 1436 onto the master **and** master `naturerune` 561 ≥ 1 with RC XP ≥ 1. A seeded Nature stack or `Trade.request` without `active()` fails. |
| Bank cycle | Runner restock: Ardougne withdraw noted essence + coins, Jiminua unnote, return with a second delivery. Master timed bank is out of the 180 s window; do not wait 60 min. |
| Options | `air_pair`: `rune=Air runes` Master+Runner (no shop/ship; Falador East). `stay_in_altar=true` on Nature: both trade inside the temple; runners need the talisman (already seeded). Do not also vary `withdrawEss` / `bankEvery`. |
| Gate | Trade + Shop buy/sell (Nature runner). Air option isolates Trade without Shop. |
| Fixture vs capability | Empty partner / RC 43 / noted-only on the master → fixture. Jiminua open but `Shop.sell` throws, or trade IDs missing → capability. |

---

## 4. CookBot

Source: `CookBot/{CookBot,CookBotLogic}.ts` (identical). Default `fish='Raw salmon'`, `location='Catherby'`, `surface='Range'`. Catherby bank (2809,3441,0); curated range stand (2817,3443,0), loc (2817,3444,0), `locName='Range'`. `LOGS_PER_TRIP=1`. `Bank.withdrawLoad` of the matched raw name. Range path: raw `useOn` oven then `ChatDialog.make(fishName)`.

| | |
|---|---|
| Core behavior | Catherby Range: open booth → `withdrawLoad('Raw salmon')` → walk range → cook → Cooking XP / `salmon` 329 up / `raw_salmon` 331 down → deposit cooked+burnt → next withdraw. |
| Catalog diff | None. |
| Seed | Cooking 25 (salmon; source does not encode the cook level — if the range burns everything, raise or pick a lower fish; **do not guess OSRS tables beyond the setting string**). Bank `raw_salmon` 331 ×200. Start Catherby bank r8. Empty pack. Booth `'Bank booth'`. Obstacles default `'door, gate'`. |
| Skills | High enough that at least one salmon cooks (Cooking XP ≥ 1). Unresolved exact level from this script; if live burns all, that is a fixture correction, not a capability miss. |
| Baseline | At bank. 0 cooked salmon. Raw still in bank. |
| Core witness | After Start: Cooking XP ≥ 1 **or** `salmon` 329 ≥ 1 with `raw_salmon` down. Seeded cooked fish fails. |
| Bank cycle | `needsBank` when `rawLeft===0`. Witness deposit of cooked/burnt then a second `withdrawLoad` that refills raw. `withdrawLoad` no-op after open is capability (provisioning). |
| Options | `surface=Fire`: same Catherby bank, `logType='Logs'`, bank `logs` 1511 ×20 + `tinderbox` 590 ×1; `lightFire('Logs')` then cook on the Fire loc. Not Firemaker `FIRE_SPOTS`. Do not also vary fish or `location=Auto`. |
| Gate | Range: `ChatDialog.make` + loc useOn (production dialogs). Fire: `lightFire` completion (fire-completion family). `Bank.withdrawLoad` (provisioning). First-family booth. |
| Fixture vs capability | No raw salmon in bank / start at Seers / door blocks and obstacle string empty → fixture. Range used, make menu missing / `withdrawLoad` throws / Fire `CANT_LIGHT` never observed → capability. |

---

## 5. BankSorter

Source: `BankSorter/{BankSorter,BankSorterLogic}.ts` (identical). One-shot: quest tab 2 → `Banking.open` → optional junk report → optional drop → `sortBank` → close → `ScriptRunner.stop`. Shim today: `sortBank` throws `notImpl`.

| | |
|---|---|
| Core behavior | Defaults `sortBank=true`, `reportQuestJunk=true`, `dropQuestJunk=false`. Opens nearest bank, reads quest leftovers (log only), rearranges bank tabs, stops. |
| Catalog diff | None. |
| Seed | One account at Varrock East (3253,3420,0) r8. Mixed bank that is **not** already sorted (e.g. `givebank` coins 995, `logs` 1511, `lawrune` 563, `bones` 526). Empty pack. |
| Skills | None. Quest-junk report needs the quest list to publish; empty junk is OK for core. |
| Baseline | Bank closed. Pack empty. Bank contents as seeded, order not yet sorted by the host. |
| Core witness | After Start: `sortBank` completes with `moves ≥ 1` (posted bank order changed) and the script stops. A pre-sorted seed with 0 moves fails the fixture. |
| Bank cycle | None. Open/close of this one-shot is not a production restock. |
| Options | `drop_junk`: `dropQuestJunk=true` **and** a finished-quest leftover the finder actually matches (cite `bankQuestJunk` row; do not invent OSRS junk). Witness: that id leaves the bank and is dropped. `sort_off`: `sortBank=false` is a no-move report-only cell — not a substitute for core. |
| Gate | `sortBank` / `categoryOf` (not in the combat family; honest `notImpl` today) plus first-family `Banking.open` and `Quests.all` after side-tab 2. |
| Fixture vs capability | Already-sorted bank / cannot open booth → fixture. Bank opens, `sortBank` throws → capability. |

---

## 6. DartFletcher

Source: `DartFletcher/{DartFletcher,DartFletcherLogic}.ts` (identical). No bank. `useOn` feathers onto tips, `DARTS_PER_ACTION=10`, max 5 user events/tick. Default `tier=Bronze` (lvl 1, 1.8 xp, tips `'Bronze dart tip'`, product `'Bronze dart'`).

| | |
|---|---|
| Core behavior | Spam `Feather` on `Bronze dart tip` until either stack is empty. Fletching XP + bronze darts up. |
| Catalog diff | None. |
| Seed | Fletching 1. Inv `bronze_dart_tip` 819 ×100 and `feather` 314 ×100. Empty of `bronze_dart` 806. Anywhere (Lumbridge courtyard is fine). |
| Skills | Fletching 1 Bronze. Iron option 22. |
| Baseline | Tips+feathers as seeded. 0 bronze darts. 0 Fletching XP this session. |
| Core witness | After Start: Fletching XP ≥ 1 **and** `bronze_dart` 806 ≥ 10 (one action) with tips or feathers down. Seeded darts fail. |
| Bank cycle | None. Script stops on empty stacks; do not add a bank cell. |
| Options | `iron`: `tier=Iron`, Fletching 22, `Iron dart tip` (query alias at implement; do not reuse 819). Do not run every tier. |
| Gate | Item-on-item `useOn` (existing). Production dialogs not used. |
| Fixture vs capability | Missing tips/feathers / Fletching below tier → fixture. `useOn` queued with no XP → capability. |

---

## 7. HerbloreSecondaries

Source: `HerbloreSecondaries/{HerbloreSecondaries,HerbloreSecondariesLogic}.ts` (identical). Default `"Red spiders' eggs"` loot at (3120,9952,0) r14, Edgeville bank (3094,3493,0), `takeFood=true`, `FOOD_DEFAULT='Lobster'`, `FOOD_DEFAULT_COUNT=10`. Pickup via `GroundItems`; eats loadout food.

| | |
|---|---|
| Core behavior | Dungeon loot: walk the egg field, Take eggs 223, eat if needed, bank when pack full (`Bank.openNearest` booth). |
| Catalog diff | None. |
| Seed | Tele (3120,9952,0) r8 (already inside Edgeville Dungeon — source has no trapdoor step). Bank `lobster` 379 ×50. Empty pack. Blank loadout. |
| Skills | None encoded. Food is for damage, not a skill gate. |
| Baseline | At the egg field. 0 eggs in pack. Bank still has the lobster seed. |
| Core witness | After Start: `red_spiders_eggs` 223 ≥ 1 from the **ground** (not `give`). Then a bank deposit of those eggs. |
| Bank cycle | Full pack → Edgeville booth → deposit except food/tool/shield → return to (3120,9952,0) → more Takes. |
| Options | `buy_newt`: `secondary='Eye of newt'`, start Betty shop (3012,3259,0), bank Draynor (3093,3243,0), `coins` 995 ×5000; `Shop.buy` raises 221. Distinct `mode=buy`. Do not also run white-berries wilderness on the same cell. |
| Gate | Core: ground Take + first-family bank (existing). Buy option: `Shop.buy`. Loadout food name from provisioning only if a named loadout is used. |
| Fixture vs capability | Start in Edgeville overworld / no egg spawns on this world → fixture. Eggs on ground, Take no-ops / Betty open but `Shop.buy` throws → capability. |

---

## 8. HerbCleaner

Source: `HerbCleaner/{HerbCleaner,HerbCleanerLogic}.ts` (identical). Default `herbs=[]` = every herb this Herblore level allows. Unid display name is **`Herb`**; IDs are grimy/unid. Guam: unid `unidentified_guam` 199 → `guam_leaf` 249, lvl 3. Op `'Identify'`. Deposits **everything** each bank cycle.

| | |
|---|---|
| Core behavior | Nearest bank withdraw `withdrawXById(unidId)` → Identify → Herblore XP / clean id up → deposit all → next withdraw. |
| Catalog diff | None. |
| Seed | Herblore 3. Bank 199 ×56 (not 249). Start Varrock East (3253,3420,0) r8. Empty pack. Leave `herbs` at default []. |
| Skills | Herblore 3 for Guam. Higher herbs in the table are denied live via `CANNOT_IDENTIFY` — do not pre-filter unless the option cell names them. |
| Baseline | 0 of id 249. Unids still in bank. |
| Core witness | After Start: Herblore XP ≥ 1 **and** 249 ≥ 1 with 199 down. Seeded clean guam fails. |
| Bank cycle | When eligible unids in pack hit 0: deposit all, `withdrawXById(199)` again, more Identifies. |
| Options | `named_guam`: `herbs=['Guam leaf']` so higher unids in the same bank are **not** withdrawn. Distinct from default-all. Do not also vary bank tile. |
| Gate | Inv `Identify` (existing interact). First-family `withdrawXById` / deposit. |
| Fixture vs capability | Seeded 249 as “unid” / Herblore 2 / noted 200 only → fixture. Identify clicked, no XP and no `CANNOT_IDENTIFY` → capability. |

---

## 9. PotionMaker

Source: `PotionMaker/{PotionMaker,PotionMakerLogic}.ts` (identical). Defaults `herb=Custom`/`herbCustom='Guam leaf'` (249, unf `guamvial` 91), `secondary=Custom`/`secondaryCustom='Eye of newt'` (221). `VIAL_OF_WATER_ID=227`, `BATCH=14`. Spam `useOn` herb→vial then secondary→unf. Finished product id is **not** in the table — witness XP and ingredient movement, not an invented dose id.

| | |
|---|---|
| Core behavior | Bank 14 vials + 14 guam → close → useOn → unf 91 → bank 14 newt → useOn → Herblore XP, unf consumed → deposit all. |
| Catalog diff | None. |
| Seed | Herblore high enough that guam+newt is not refused (source has no level field; if Identify-style refuse appears, raise — do not guess OSRS). Bank 249 ×42, 227 ×42, 221 ×42. Start Varrock East r8. Empty pack. |
| Skills | Unresolved exact level from this script. |
| Baseline | 0 of unf 91. Ingredients in bank only. |
| Core witness | After Start: 91 appears then falls **or** Herblore XP ≥ 1, with 249 and 227 down. Seeded unf/finished potions fail. |
| Bank cycle | RestockIngredients + FinishPotions **are** the loop. Witness a second vial/herb withdraw after deposit. |
| Options | `ranarr`: `herb='Ranarr weed'` (257, unf 99) + `secondary='Snape grass'` (231). Distinct recipe. Do not cross every herb × secondary. |
| Gate | Item-on-item `useOn` (combat design lists PotionMaker here). First-family withdrawXById. |
| Fixture vs capability | Missing vials / custom string that matches no `HERBS` row → fixture. useOn fires, no XP and unf stays → capability. |

---

## 10. SmelterBot

Source: `SmelterBot/{SmelterBot,SmelterBotLogic}.ts` (identical). Default `bar=Bronze` (Copper+Tin, lvl 1). Al-Kharid bank (3269,3167,0), furnace (3275,3185,0), loc `'Furnace'`. `ChatDialog.makeX(barKeyword, 30)`.

| | |
|---|---|
| Core behavior | Booth → `withdrawX` copper+tin per `withdrawFor` → furnace → makeX Bronze → Smithing XP / `bronze_bar` 2349 up → deposit → restock. |
| Catalog diff | None. |
| Seed | Smithing 1. Bank `copper_ore` 436 ×200 and `tin_ore` 438 ×200. Start Al-Kharid bank r8. Empty pack. Obstacles `'door, gate'`. |
| Skills | Smithing 1 Bronze. Steel option 30 + coal. |
| Baseline | At bank. 0 bronze bars. Ores in bank. |
| Core witness | After Start: Smithing XP ≥ 1 **and** 2349 ≥ 1 with copper/tin down. Seeded bars fail. |
| Bank cycle | `canSmelt()==false` trips. Witness deposit of 2349 then a second ore withdraw. |
| Options | `steel`: `bar=Steel`, Smithing 30, bank iron ore + `coal` 453 (2 coal/bar). Distinct coal ratio. Do not run every bar. |
| Gate | `ChatDialog.makeX` waiting on count-dialog (production dialogs). First-family booth + Withdraw-X. |
| Fixture vs capability | No tin / furnace name wrong / start at Varrock → fixture. Furnace used, makeX answers without `count_dialog_open` or no bars → capability. |

---

## 11. Superheater

Source split (only section-A split):
`100adccc` Superheater.ts `9188208c0351` / Logic `41b9bf85fc0b` — wield **only** `'Staff of fire'`.
`8e7d965b` Superheater.ts `4c4ea711e2f7` / Logic `74df1dfcc8bc` — `FIRE_STAVES` from `STAFF_RUNES` fire providers, bank re-check, wait **Wield** after close.
Shared: `SUPERHEAT_ITEM='Superheat Item'`, `NATURES_DEFAULT=50`, `SUPERHEAT_SLOTS=27`, Bronze = copper+tin, `Game.castOnItem` on **primary** ore (`Copper ore`).

| | |
|---|---|
| Core behavior | Nearest bank, wear fire staff, withdraw natures + ores, `castOnItem('Superheat Item', 'Copper ore')`, Magic+Smithing XP, ore/nature down, bars up, deposit bars (keep natures), restock. |
| Catalog diff | Staff selection only. Settings schema is the same. Do not invent 8e7d bar modes. |
| Seed | Magic high enough that Superheat is on the spellbook (**level not in source**; missing button = fixture). Smithing 1. Bank `staff_of_fire` 1387 ×1, `naturerune` 561 ×200, copper 436 ×100, tin 438 ×100. Start any booth bank (Varrock East). Empty pack, staff not yet worn. |
| Skills | Unresolved Magic level from this script. Smithing 1 for Bronze. |
| Baseline | Staff in bank, 0 bars, natures in bank. |
| Core witness | After Start: Magic XP ≥ 1 **and** Smithing XP ≥ 1 **and** 2349 ≥ 1, natures down, copper down. Wield of 1387 observed after bank close. Seeded bars fail. |
| Bank cycle | Primary ore 0 with pack used → deposit except natures → withdraw ores + top up natures to 50 → more casts. |
| Options | `8e7d_other_staff` (8e7d catalog **only**): bank `fire_battlestaff` 1393 (or `mystic_fire_staff` 1401 / `lava_battlestaff` 3053) **without** 1387. `pickFireStaff` must wield that identity. Supporting only Staff of fire fails this catalog. Do not run this cell on 100adccc. `steel` bar is a recipe split, not a staff split — pick one. |
| Gate | Spell facts + `castOnItem` (and `STAFF_RUNES` export for 8e7d construct). First-family withdrawX. Equip existing. |
| Fixture vs capability | No Superheat button / no fire staff in 100adccc bank → fixture. Cast queued, no Magic XP / 8e7d cannot construct `FIRE_STAVES` → capability. |

---

## 12. SmithingBot

Source: `SmithingBot/SmithingBot.ts` (identical). Default `bar=Bronze`, `product=Dagger` (1 bar). Anvil stand (3188,3425,0), bank (3185,3440,0) Varrock West. `ANVIL='Anvil'`, `HAMMER='Hammer'` (`hammer` 2347). `bar.useOn(anvil)` then `ChatDialog.isMainMakePanel` / `makeFromPanelMax('Dagger')`. **Not** chat `make_products`.

| | |
|---|---|
| Core behavior | Keep hammer, `withdrawLoad` bronze bars, useOn anvil, main-modal Make-N, Smithing XP, bars down, `bronze_dagger` 1205 up, bank remainder, restock. |
| Catalog diff | None. |
| Seed | Smithing 1. Bank 2347 ×1 and `bronze_bar` 2349 ×28. Start Varrock West bank r8. Empty pack. |
| Skills | Smithing 1 Bronze dagger. Platebody is 5 bars — a different product cell, not core. |
| Baseline | At bank. 0 daggers. Bars in bank. |
| Core witness | After Start: Smithing XP ≥ 1 **and** 1205 ≥ 1 with 2349 down, after the **anvil panel** (not a furnace make). Seeded daggers fail. |
| Bank cycle | `barCount < barsNeeded` → deposit except hammer → `withdrawLoad` bars → next dagger. |
| Options | `platebody`: `product=Platebody` (5 bars). Distinct panel row / bar drain. Do not also vary bar tier on that cell. |
| Gate | Smithing main-panel (`isMainMakePanel` / `mainMakeProducts` / `makeFromPanelMax` all throw today). `Bank.withdrawLoad` (provisioning). |
| Fixture vs capability | No hammer / start at Al-Kharid furnace → fixture. Anvil used, isolate has only chat make_products, no 1205 → capability. |

---

## 13. FlaxPicker

Source: `FlaxPicker/FlaxPicker.ts` (identical). Default loc `'Flax'`, op `'Pick'`. Field (2741,3444,0), west gate (2736,3443,0), Seers bank entrance (2726,3487,0), stand (2725,3493,0). Full pack → `bankRun` deposit all. Combat design: loc interact, not the excluded gatherer rewrite.

| | |
|---|---|
| Core behavior | Pick Flax locs in the Seers field until pack full, walk gate→bank entrance→booth, `depositInventory`, return. |
| Catalog diff | None. |
| Seed | Tele field (2741,3444,0) r6. Empty pack and bank of `flax` 1779. |
| Skills | None. |
| Baseline | At field. 0 flax. |
| Core witness | After Start: 1779 ≥ 1 from a **Pick** on a Flax loc (not `give`). Then a deposit that clears the pack at Seers booth. |
| Bank cycle | `Inventory.isFull()` → deposit all → return to field → more Picks. |
| Options | None that change product. Do not retarget `fieldTile` unless a selected world has no Flax loc at the default (then record unavailable content). |
| Gate | Loc interact (existing). First-family booth. |
| Fixture vs capability | Start at the bank / no Flax loc in leash → fixture. Pick sent, flax count unchanged → capability. |

---

## 14. FlaxSpinner

Source: `FlaxSpinner/FlaxSpinner.ts` (identical). Default `product='Flax'` (menu match; output is Bow string). Bank (2722,3493,0), ladder (2714,3471,0) `'Ladder'` Climb-up/down, wheel (2711,3471,1) `'Spinning wheel'` Spin. `ChatDialog.makeX(product, fibreCount)`. Obstacle `'door'`.

| | |
|---|---|
| Core behavior | Floor 0, fibre 0 → booth withdraw-all Flax → climb to z=1 → Spin → makeX Flax → Crafting XP, 1779 down, `bow_string` 1777 up → climb down → deposit → restock. |
| Catalog diff | None. |
| Seed | Crafting 1. Bank `flax` 1779 ×56. Start Seers bank r8. Empty pack. |
| Skills | Crafting 1 flax. Wool option is a different fibre. |
| Baseline | Floor 0 at bank. 0 bow strings. Flax in bank. |
| Core witness | After Start: Crafting XP ≥ 1 **and** 1777 ≥ 1 with 1779 down, after the wheel make menu. Seeded strings fail. |
| Bank cycle | Fibre 0 on floor 0 → deposit strings → withdraw flax → next climb. |
| Options | `wool`: `product='Wool'`, bank `wool` 1737, product `ball_of_wool` 1759. Distinct menu row. |
| Gate | `ChatDialog.makeX` + count-dialog (production dialogs). Ladder loc existing. |
| Fixture vs capability | No flax / start upstairs with empty pack → fixture. Wheel opens, makeX no-ops → capability. |

---

## 15. FlaxAIO

Source: `FlaxAIO/{flaxaio,picking,spinning,banking,walking}.ts` (identical). Defaults `picking=true`, `spinning=true`. Same Seers tiles as picker+spinner. `SPUN_NAME.Flax='Bow string'`. `needsBank` only on level 0, when bowstrings > 0 or (spin-only and fibre 0).

| | |
|---|---|
| Core behavior | Pick field → (full or spin path) → wheel makeX → bank strings. Not a concatenation of the two other cards' settings; both flags default on. |
| Catalog diff | None. |
| Seed | Tele field (2741,3444,0) r6. Empty pack. Crafting 1. No banked flax required for pick+spin. |
| Skills | Crafting 1 to spin. |
| Baseline | At field. 0 flax, 0 strings. |
| Core witness | After Start: 1779 from Pick **then** 1777 from the wheel **then** a Seers deposit of 1777. Pick-only or spin-only is a different cell. |
| Bank cycle | Bowstrings on floor 0 → deposit → if spinning&&!picking, withdraw flax; else return to field. |
| Options | `pick_only`: `spinning=false` (picker loop, no makeX). `spin_only`: `picking=false`, bank 1779 ×56 (spinner loop). Do not treat those as covering core. |
| Gate | Loc Pick + `ChatDialog.makeX`. First-family booth. |
| Fixture vs capability | Spinning true but Crafting blocked / start at wheel with empty pack and picking true → fixture. Pick works, makeX throws → capability. |

---

## 16. GemCutter

Source: `GemCutter/{GemCutter,GemCutterLogic}.ts` (identical). Default `gems=[]` = every gem this Crafting level allows. `CHISEL_ID=1755`. Chisel `useOn` uncut. Opal/jade/topaz `canCrush` → `CRUSHED_GEMSTONE_ID=1633`. Sapphire 1623→1607 lvl 20 does **not** crush.

| | |
|---|---|
| Core behavior | Nearest bank, keep chisel, `withdrawXById(uncut)`, spam chisel-on-uncut, Crafting XP / cut id up, deposit except chisel. |
| Catalog diff | None. |
| Seed | Crafting 20. Bank `chisel` 1755 ×1 and `uncut_sapphire` 1623 ×28 only (so default-all cannot grab crushables). Start Varrock East r8. Empty pack. |
| Skills | Crafting 20 Sapphire. Opal is lvl 1 but crush makes the witness ambiguous — do not use it for core. |
| Baseline | 0 `sapphire` 1607. Uncuts in bank. |
| Core witness | After Start: Crafting XP ≥ 1 **and** 1607 ≥ 1 with 1623 down. Seeded cut sapphires fail. 1633 must stay 0. |
| Bank cycle | Eligible uncuts in pack 0 → deposit except 1755 → withdraw 1623 again. |
| Options | `named_sapphire`: `gems=['Sapphire']` with mixed uncuts in bank; only 1623 is withdrawn. Distinct from default-all. |
| Gate | Item-on-item `useOn` (combat design). First-family withdrawXById. |
| Fixture vs capability | No chisel / Crafting 19 / noted 1624 only → fixture. useOn fires, no XP → capability. |

---

## 17. MuleCrafter

Source: `MuleCrafter/{MuleCrafter,MuleCrafterLogic}.ts` (identical) + `data/runeCraftLocations.ts`. Default `rune='Air rune'` (singular), `mode='Crafter'`, `partner=''` (**solo** when blank), `bankFill=true`. Air ruins (2983,3288,0), Falador East bank (3013,3355,0). Mule mode **requires** partner or throws at start.

| | |
|---|---|
| Core behavior | Solo Crafter: bank essence 1436, talisman on Mysterious ruins, Craft-rune, Air rune up, RC XP, deposit. Same family as RuneCrafter Solo but this card's ruins tile and setting names differ — do not reuse the RuneCrafter scenario verbatim. |
| Catalog diff | None. |
| Seed | RC 1. Air talisman 1438. Bank `blankrune` 1436 ×200. Start Falador East r8. `partner` left blank. |
| Skills | RC 1 Air. Mind option lvl 2, Edgeville, ruins (2980,3511,0). |
| Baseline | Overworld. 0 Air runes. Essence in bank. |
| Core witness | After Start: `airrune` 556 ≥ 1 and RC XP ≥ 1, then a bank deposit. |
| Bank cycle | Solo `bankFill` withdraw is the loop. |
| Options | `mule_pair`: two profiles, Crafter `partner`=mule IGN + Mule `partner`=crafter IGN. Trade 1436 at the ruins, crafter crafts, mule banks runes. Distinct from solo. Do not also switch to Nature (that card owns the ship/Jiminua route). |
| Gate | Loc + Trade (pair cell). First-family booth. |
| Fixture vs capability | `mode=Mule` with empty partner (throws before loop) → fixture. Inside altar, craft op no-ops / Trade IDs missing → capability. |

---

## 18. ShopBuyout

Source: `ShopBuyout/{ShopBuyout,shopPresets}.ts` (identical). Default shop = first preset `Aemad's vials — East Ardougne` (keeper `Aemad`, shop (2613,3294,0), bank (2655,3283,0)). `buyItems=[]` = all stock. `Shop.buy`. Budget 250k / per-trip 100k / floor 5k — **too fat for a 180 s cell**; core must lower `budgetGp`/`perTripGp` without changing shop. Mage Arena / Wizard Guild are selected-content risks — not core.

| | |
|---|---|
| Core behavior | Ardougne East bank withdraw coins → Aemad → Buy 10/5/1 until pack or budget → deposit except coins → repeat. |
| Catalog diff | None. |
| Seed | Tele Aemad (2613,3294,0) r6. Bank `coins` 995 ×20000. Settings: `budgetGp=2000`, `perTripGp=2000`, `stopFloorGp=0` so the gold window can finish. Empty pack. |
| Skills | None. |
| Baseline | Shop closed. 0 vials in pack. Coins in bank. |
| Core witness | After Start: Aemad stock of `vial_empty` 229 (display `Vial`) down **and** inv 229 up by the bought qty, coins down. Queued if-button fails. |
| Bank cycle | Pack full or trip gp spent → deposit vials, withdraw coins, second buy. |
| Options | `aubury`: preset `Aubury's runes — Varrock`, start (3253,3401,0), bank Varrock East; buy a named rune (`buyItems` one name). Distinct keeper. Do not also run Lundail/Wizard Guild on the same cell; if those keepers/tiles are missing on a revision, record unavailable content. |
| Gate | `Shop.buy` pending 10/5/1 (shop-transfers family). First-family bank. |
| Fixture vs capability | No Aemad / 0 coins / budget already 0 → fixture. Shop open, `Shop.buy` throws or stock unchanged → capability. |

---

## 19. DoorOpener

Source: `DoorOpener/{DoorOpener,DoorOpenerLogic}.ts` (identical). Default stand (3215,3212,0) Lumbridge, `obstacle='door, gate'`, leash 8. Opens nearest loc that still offers **Open**. No bank, no XP.

| | |
|---|---|
| Core behavior | Walk stand → Open nearest shut door/gate → count `opens`. Loops. |
| Catalog diff | None. |
| Seed | Tele (3215,3212,0) r4. Confirm a shut door/gate with Open is in leash **before** Start (if the courtyard is already open, pick a known shut door on this world or the cell is a fixture miss). |
| Skills | None. |
| Baseline | At least one matching loc still shut. `opens=0`. |
| Core witness | After Start: that loc no longer offers Open (or a world-change through it). A queued Open without a loc state change fails. |
| Bank cycle | None. |
| Options | `gate`: `obstacle='gate'` at a tile whose nearest shut loc is a gate, not a door. Distinct name match. |
| Gate | Loc interact (existing). Not this family's missing API. |
| Fixture vs capability | No shut matching loc in leash → fixture. Open sent, loc still shut → capability. |

---

## 20. TannerBot

Source: `TannerBot/TannerBot.ts` (identical). Default `'Soft leather'`: hideId **1739**, productId **1741**, tanAllComId **8686**, NPC `'Tanner'`, stand (3277,3191,0), Al-Kharid bank (3269,3167,0). `buyThread=true` every 5 trips at Dommik (3316,3192,0) — **will not fire** inside 180 s if a trip is one inventory. Coins float 2000. Hide display: 274 `'Cow hide'`, 289 `'Cowhide'` — **seed and witness by id 1739**, not the label.

| | |
|---|---|
| Core behavior | Withdraw 1739 + coins → Tanner → tan-all widget 8686 (verify 289 archive; post selected id) → 1741 up, 1739 down, coins down → bank leather. |
| Catalog diff | None. |
| Seed | Tele Al-Kharid bank r8. Bank 1739 ×28 and `coins` 995 ×5000. Empty pack. `buyThread=false` on core so Dommik is not required. |
| Skills | None encoded. |
| Baseline | 0 leather 1741. Hides in bank. |
| Core witness | After Start: 1741 ≥ 1 with 1739 down after the Tanner widget, not a shop. Seeded leather fails. |
| Bank cycle | Deposit leather (keep coins) → withdraw 1739 → second tan. |
| Options | `hard`: `hideType='Hard leather'` product 1743. Distinct widget 8690. `thread`: `buyThread=true` **and** force a shop trip (`threadEveryRuns=1`); `Shop.buy('Thread')` 1734. Do not also run all four dragonhides (same-name IDs 1753/1751/1749/1747). |
| Gate | Tanner widget press (catalog-local coms; verify 289) + first-family bank. Thread option: `Shop.buy`. |
| Fixture vs capability | Seeded by name `'Cow hide'` on 289 / no Tanner NPC → fixture. Tanner open, 8686 missing or no 1741 → capability. |

---

## 21. VialFiller

Source: `VialFiller/{VialFiller,VialFillerLogic}.ts` (identical). Default bank Falador West (2946,3369,0), fountain (2949,3381,0). `EMPTY_VIAL='Vial'` id `vial_empty` **229**. `buyVials=false`. Fill: empty `useOn` fountain loc → 227 up.

| | |
|---|---|
| Core behavior | Withdraw empty 229 → fountain useOn → `vial_water` 227 up, 229 down → deposit filled (keep coins) → restock empties. |
| Catalog diff | None. |
| Seed | Tele Falador West r8. Bank 229 ×56. Empty pack. `buyVials=false`. |
| Skills | None. |
| Baseline | 0 of 227. Empties in bank. |
| Core witness | After Start: 227 ≥ 1 with 229 down at the fountain. Seeded filled vials fail. |
| Bank cycle | Pack of 227 → deposit → withdraw 229 → second fill. |
| Options | `jatix`: `buyVials=true`, `buyEveryRuns=1`, coins 995 ×2000, start with **no** 229 in bank; `Shop.buy('Vial')` at Jatix (2899,3427,0) then fill. Members/Taverley: if the selected world cannot reach Jatix, record unavailable content. |
| Gate | Item-on-loc (existing). Buy option: `Shop.buy`. |
| Fixture vs capability | No empties and buyVials false / fountain missing → fixture. useOn fires, 229 unchanged → capability. |

---

## 22. LeatherCrafter

Source: `LeatherCrafter/{LeatherCrafter,LeatherCrafterLogic}.ts` (identical). Default `leatherType='Leather'` id 1741. Crafting 1 recipe `'Leather gloves'` (make1 8638 / make10 8636). Needle **1733**, thread **1734**. Al-Kharid bank (3269,3167,0). Needle `useOn` leather; interface flow for soft leather.

| | |
|---|---|
| Core behavior | Keep needle+thread, withdraw 1741, needle-on-leather, Crafting XP, 1741 down, `leather_gloves` 1059 up, bank products. |
| Catalog diff | None. |
| Seed | Crafting 1. Bank 1733 ×1, 1734 ×100, 1741 ×28. Start Al-Kharid bank r8. Empty pack. |
| Skills | Crafting 1 gloves. Hard leather body is lvl 28 / `flow='single'` — a different cell. |
| Baseline | 0 gloves. Leather in bank. |
| Core witness | After Start: Crafting XP ≥ 1 **and** 1059 ≥ 1 with 1741 down. Seeded gloves fail. |
| Bank cycle | Leather below recipe qty → deposit except needle/thread/leather → withdraw 1741 → next craft. |
| Options | `hard_body`: `leatherType='Hard leather'`, Crafting 28, leatherId 1743, product `'Hardleather body'`. Distinct `flow='single'` burst. Do not also run dragon leather `multi3`. |
| Gate | Item-on-item + leather interface buttons (production; widget ids are catalog-local, verify 289). First-family bank. |
| Fixture vs capability | No needle / Crafting 0 / 1743 seeded as 1741 → fixture. Needle used, interface ids missing, no 1059 → capability. |

---

## 23. Firemaker

Source: `Firemaker/Firemaker.ts` (identical). Default `logType='Logs'`, `location='Varrock East'`. `FIRE_SPOTS['Varrock East']`: bank (3253,3420,0), AABB x 3232–3284, z 3428–3430. Tools tinderbox. `lightFire(logName)` then next tile in the plot.

| | |
|---|---|
| Core behavior | Booth, keep tinderbox 590, withdraw logs 1511, walk the west-running plot, `lightFire` until Firemaking XP + a Fire loc, bank leftover, restock. |
| Catalog diff | None. |
| Seed | Firemaking 1. Bank 590 ×1 and 1511 ×28. Start Varrock East bank r8. Empty pack. |
| Skills | Firemaking 1 Logs. Oak is 15 — a different log cell. |
| Baseline | At bank. 0 Firemaking XP this session. No Fire loc on the plot yet (or ignore pre-existing fires). |
| Core witness | After Start: Firemaking XP ≥ 1 **and** a Fire loc inside the Varrock East AABB. Walking the bank tile is not a light. |
| Bank cycle | Logs 0 → deposit except tinderbox → withdraw logs → next light. |
| Options | `oak`: `logType='Oak logs'`, Firemaking 15. Distinct log id (query `oak_logs` at implement). Do not also vary location on that cell. |
| Gate | `lightFire` completion + next-burn-tile (fire-completion family). First-family bank. |
| Fixture vs capability | No tinderbox / start at Seers / plot blocked → fixture. useOn logs, no XP and no `CANT_LIGHT` → capability. |

---

## 24. FlaxRunner

Source: `FlaxRunner/{FlaxRunner,FlaxRunnerLogic}.ts` (identical). Default `mode='Runner'`, `partner=''`, `minFlaxCapacity` default 24. Same Seers field/wheel/bank/meet (2719,3471,0) tiles as FlaxAIO. **Two profiles required** (Runner without partner cannot deliver). Spinner spins and banks strings.

| | |
|---|---|
| Core behavior | Runner picks 1779, walks `MEET_TILE`, `Trade.request` spinner, offers flax. Spinner accepts, climbs, makeX, banks 1777. |
| Catalog diff | None. |
| Seed | Two mainland accounts. Runner: field (2741,3444,0) r6, empty pack, `mode=Runner`, `partner`=spinner IGN. Spinner: wheel house (2714,3471,0) r4, empty pack, Crafting 1, `mode=Spinner`, `partner`=runner IGN. |
| Skills | Spinner Crafting 1. Runner none. |
| Baseline | Trade closed. 0 flax on runner. 0 strings on spinner. |
| Core witness | After both Start: Trade delivers 1779 onto the spinner **and** spinner 1777 ≥ 1 with Crafting XP ≥ 1. `Trade.request` without `active()` fails. |
| Bank cycle | Spinner: strings on floor 0 → Seers deposit → return to meet. Runner: after delivery, pick again (no bank unless pack jammed). |
| Options | None that drop Trade. Do not run this as a solo FlaxAIO substitute. |
| Gate | Trade + `ChatDialog.makeX` on the spinner. Loc Pick on the runner. |
| Fixture vs capability | Empty partner / both started as Runner → fixture. Meet adjacent, trade IDs missing / spinner makeX throws → capability. |

---

## Implementer notes

1. Reuse the existing catalog scenario runner and CoreWitness. Do not add a controller.
2. Superheater is the only catalog-source split; 8e7d fire-staff alternatives are a **proof** on spell facts, not a new API.
3. Two-profile cells: NatureCrafter, RuneCrafter runner_mule, MuleCrafter mule_pair, FlaxRunner. Seed `seed.profiles` length 2 like Duel Arena.
4. Name collisions: unid herbs display `Herb`; rune essence 1436 vs noted 1437; cow hide 1739 display differs 274/289; dragonhide 1747/1749/1751/1753; Vial 229 vs Vial of water 227.
5. Widget literals (Tanner 8686…, leather 8638…) are cache evidence to verify on 289, same rule as duel modals.
6. Preserve 45 enabled cards and 180 rows. Do not claim a deferred rewrite supported because a fixture skipped it.


