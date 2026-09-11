# Remaining combat and world catalog fixtures

Architect: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: section B of brief 49. Not LIVE, not source,
not support-ledger, not capability redesign.

Branch checked first: `codex/rs2b0t-multirevision` (HEAD at write
`1a2f2dcf`). Catalogs
`.superpowers/inputs/rs2b0t-{100adccc037d9f6898080e1cad58fcfc43364775,8e7d965be2071d6ec65c3265e12af797082d720a}`
are byte-identical for every named card below (SHA-256 prefixes in the
file list). Generated facts: `crates/api/data/game-data/{274,289}.json`
schema 3. Combat-NPC type tables are **not** in those assets; combat
targets are script display names unless a pickpocket group already
joins an ID. Do not invent OSRS IDs.

Capability architecture stays in `04-combat-production-design.md` and
`04-provisioning-recovery-design.md`. This report only names the next
shared scenario + CoreWitness cells. Reuse `script_live_seed_steps`,
`start_catalog_step`, `drain_advancestat`, `gold_script_nav` (600 ms),
`SCRIPT_GOLD_DEADLINE` 180 s, `SCRIPT_GOLD_WATCH_TICKS` 150. Bank-travel
observation may use the existing BoneBurier 240-tick window when the
script's own 120 s walk is in play; do not raise product timeouts.

## Shared contracts

| Rule | Contract |
|---|---|
| Catalogs | No card here differs across frozen roots. Do not invent an 8e7d-only mode. |
| Five first cards | Alcher / ChickenKiller / Thiever / BoneBurier / BankFletcher already have core fixtures. Do not re-audit them. |
| Start | After `ingame && scene_state==2`, last seed wait, and `StartScript`. Baseline is the first post-seed observation **before** Start. |
| Proof | Script-caused XP / inventory / world change after Start. Seeded items, queued sends, and constructor `validate===false` are not proof. |
| Clue | `SolveClue` stays pre-Start refusal. Core cells inject `solveClues=false`. A `true` cell must refuse Start with `BLOCKED: missing native clue solving`, not silently no-op. |
| PeriodicBank | Default source label is `Off`. Core cells keep Off. Bank-cycle cells use `Loot count` (host must parse that **label**, not the shim token `loot`). |
| DeathRecovery | Idle on core cells (no induced death). A death cell is a separate option and needs the queued DeathRecovery capability. |
| Loadout | Blank uses each card's script fallback (`Lobster` or `Trout` as cited). Named loadout/gear/weapon cells wait on the queued loadout store. |
| IDs | Witness generated `alias`/`id`, not display names alone. Same-name rows: Dragonhide 1747/1749/1751/1753, Magic shortbow unstrung 72 vs strung 861, Bones 526 vs sheep/newbie, Coins 995 not 617. |
| Cheats | `give` / `givebank` / `advancestat` / `tele` / `setvar tutorial 1000` as existing catalog cells. Do not treat debug `inv_add` as normal acquisition. |
| Cartesian | One core + the distinct option cells below. Do not cross meleeStyle × combatStyle × loot lists. |
| 180 rows | These cells do not drop or add enabled cards. |

### Shared generated item IDs (274 = 289 for these rows)

| Display | Alias | ID | Notes |
|---|---|---:|---|
| Bones | `bones` | 526 | Regular bury; not 280–283 / 2530 |
| Big bones | `big_bones` | 532 | |
| Dragon bones | `dragon_bones` | 536 | |
| Dragonhide (green) | `dragonhide_green` | 1753 | GreenDragon table says `"Dragonhide"`; do not accept 1747/1749/1751 |
| Dragonfire shield | `antidragonbreathshield` | 1540 | Selected display name; this **is** the anti-dragon shield row |
| Glarial's amulet | `glarials_amulet_waterfall_quest` | 295 | FireGiant keep-item |
| Rope | `rope` | 954 | |
| Brass key | `edgevilledungeonkey` | 983 | |
| Limpwurt root | `limpwurt_root` | 225 | |
| Coal | `coal` | 453 | |
| Cake | `cake` | 1891 | Ardy food |
| Bread | `bread` | 2309 | CAKE_ITEMS |
| Chocolate slice | `chocolate_slice` | 1901 | CAKE_ITEMS; not chocolate cake 1897 |
| Coins | `coins` | 995 | |
| Nature / Law / Air / Fire / Water / Chaos / Death / Blood rune | `naturerune` 561, `lawrune` 563, `airrune` 556, `firerune` 554, `waterrune` 555, `chaosrune` 562, `deathrune` 560, `bloodrune` 565 | | |
| Lobster / Trout / Shark | `lobster` 379, `trout` 333, `shark` 385 | | Generated fixed heals: lobster 12, bread 4 |
| Staff of air / fire | `staff_of_air` 1381, `staff_of_fire` 1387 | | |
| Maple shortbow (strung) | `maple_shortbow` | 853 | Not unstrung 64 |
| Bronze / Iron arrow | `bronze_arrow` 882, `iron_arrow` 884 | | |
| Rune scimitar / Dragon dagger | `rune_scimitar` 1333, `dragon_dagger` 1215 | | |
| Knife | `knife` | 946 | |
| Steel axe | `steel_axe` | 1353 | |
| Magic logs | `magic_logs` | 1513 | |
| Magic shortbow unstrung / strung | `unstrung_magic_shortbow` 72 / `magic_shortbow` 861 | | Same display name |
| Magic longbow unstrung / strung | `unstrung_magic_longbow` 70 / `magic_longbow` 859 | | Same display name |
| Agility arena ticket | `agilityarena_ticket` | 2996 | |
| Casket | `casket` | 405 | `RANDOM_EVENT_CASKET_ID` |
| Uncut sapphire | `uncut_sapphire` | 1623 | |
| Super attack(3) / Super strength(3) | `3dose2attack` 145, `3dose2strength` 157 | | |
| Half of a key | `keyhalf1` 985 / `keyhalf2` 987 | | Same display name; loot matcher is name-contains |

Pickpocket groups (274 = 289 IDs): Guard `guard1/2` 9/10 + `ardougne_guard` 32, lv 40, 468 XP, 30 coins; Knight 23/26 lv 55 843 XP 50 coins; Paladin 20 lv 70 1518 XP; Hero 21 lv 80 2733 XP.

### Invalid fixture vs missing capability

A **fixture** failure is: wrong tile/level, wrong NPC/loc **name**, missing skill/quest/item the script itself stops on, clue left `true`, or a seed still in the baseline. A **capability** failure is: script starts, reaches the action, and the host throws / no-ops the mapped operation (`Autocast.arm`, `PeriodicBank.execute`, `DeathRecovery.execute`, `Game.teleport`, `isHostileAttacker`, `Shop.buy`/`sell`, `reader.ifText`, `weaponOf`/`gearOf`/`suppliesOf`, duel partner text). First failure in each card names which.

---

## 1. Duel Arena Combat Trainer

Source: `DuelArena/{DuelArena,DuelArenaLogic,DuelInterface}.ts` (identical).
Melee only. Challenges every 5 s at `DUEL_CHALLENGE_ANCHOR` (3368,3274,0).
Modal literals 6575 / 6412 / 6733 and accept 6674 / 6520 are catalog-local;
combat design already requires posting the selected archive if 289 differs.

| | |
|---|---|
| Core behavior | Travel to challenge lobby → Challenge op 1 → accept select/confirm → Fight op 2 in a pen → melee XP. Stops when Attack/Strength/Defence all meet targets. |
| Catalog diff | None. |
| Seed | Two mainland accounts (`seed.profiles` length 2). Both: tutskip/relog, wield a 1-handed melee weapon that offers exact Attack + Strength styles (and Defence if that target > 1), tele (3368,3274,0) radius 8. Companion **also Starts this card** (not a closer). No `give` XP. |
| Skills | Fresh melee is enough for core (Defence target default 1). |
| Baseline | Both tiles in the challenge rect; no Attack/Strength XP yet; no duel modal. |
| Core witness | After Start: Challenge dispatched, modal observed, then **Attack or Strength XP ≥ 1** and `duels` progress (pen occupancy then return). A seeded modal or a queued Challenge without XP fails. |
| Bank cycle | None. |
| Options | `duel_defence`: `targetDefence=2` (or 5) with a weapon that has Defensive; Defence XP ≥ 1. `duel_complete_targets`: low targets (e.g. Attack=2, Strength=2, Defence=1) then script stop after those levels. Do not also vary Challenge interval. |
| Gate | Hostile-duel facts (`Input.interactPlayer` already maps; `reader.ifText` / posted modal IDs still required). |
| Fixture vs capability | No second player in the lobby / wrong tile → fixture. Challenge sent, modal IDs missing or `ifText` throws, no Fight XP → capability. |

---

## 2. ChaosDruidKiller

Source: `ChaosDruidKiller/{ChaosDruidKiller,ChaosDruidLogic}.ts`. Own bank/death
logic; does **not** construct PeriodicBank / DeathRecovery / SolveClue.

| Location | NPC | Field | Bank | Extra |
|---|---|---|---|---|
| Edgeville Dungeon (default) | Chaos druid | (3110,9936,0) r14 | (3094,3491,0) | Trapdoor (3096,3468) / ladder (3096,9868) Climb-up; wilderness gate (3130,9914) |
| Chaos Druid Tower | Chaos druid | (2562,3356,0) r4 | Ardougne West (2616,3332,0) | Door Pick Lock (2565,3356,0); Thieving 46; Swarm id 411 flee (2576,3356,0) |
| Yanille Dungeon | Chaos druid warrior | (2580,9501,0) r8 | (2612,3092,0) | Agility 40 Balancing ledge; slash weapon or knife for the entrance web |

Loot matcher is exact `herb` / `law rune` / `nature rune` (unidentified herbs
display `Herb`, ids 199…). Food fallback `Lobster` ×12. Combat style is a
0–3 **button index**, default `1`.

| | |
|---|---|
| Core | `location=Edgeville Dungeon`, default food/style. Tele field after tutskip. Seed `give lobster 12` (and `givebank lobster 200` for the bank cell). |
| Baseline | In field, 12 lobster, 0 herb/law/nature, no combat XP. |
| Core witness | Combat XP (the resolved style) **and** at least one wanted loot pickup (`Herb` id 199-family or Law 563 or Nature 561) after Start. |
| Bank cycle | Pack full of loot (or food=0 and HP ≤ 35%): open Edgeville booth, deposit, withdraw 12 lobster, return through trapdoor, another kill/loot. 240-tick travel window allowed. |
| Options | `chaos_tower`: Thieving 46, tele door stand, Pick Lock, fight inside the 5×9 room. `chaos_yanille`: Agility 40, knife or slash weapon, ledge Walk-across, Chaos druid warrior XP. One non-default `combatStyleIndex` (e.g. `0`) on Edgeville only. |
| Gate | Loc interact + NPC Attack already mapped; bank family for the cycle; loadout blank uses lobster (`scriptFood`). Tower Swarm is script-owned walk-out, not DeathRecovery. |
| Fixture vs capability | Wrong dungeon / missing 46 Thieving (script throws) → fixture. Booth open/withdraw fails after a real full pack → banking capability. |

---

## 3. RockCrab

Source: `RockCrab/{RockCrab,RockCrabSpots,AmmoLogic}.ts`. Constructs
PeriodicBank, DeathRecovery (r4), SolveClue.

Targets: dormant `Rocks` then active `Rock Crab` (lines 531–545). Default
spots `(2704,3726,0)` … `(2713,3727,0)`; reset `(2712,3688,0)`; Seers bank
`(2725,3491,0)`. Default melee / strength / lobster ×20 / `bankStrategy=Off`
/ `solveClues=true` (inject false).

| | |
|---|---|
| Core | Tele default spot 1, `combatStyle=melee`, `solveClues=false`, PeriodicBank Off. Seed lobster 8 in pack (enough to start; not a bank proof). |
| Baseline | On a DEFAULT_SPOTS tile, melee style, no strength XP, no Rock Crab kill loot. |
| Core witness | Strength XP ≥ 1 after Start **and** a dormant `Rocks` → `Rock Crab` fight (stack/clear). Seeded XP fails. |
| Bank cycle | `bankStrategy=Loot count`, `bankEveryItems=1`, seed `givebank lobster 40` plus one listed loot name in the pack after a kill (uncut sapphire 1623 or casket 405 — **not** `small oyster pearls`; that string is not a generated display name). Walk to `bankTile` (2725,3491,0), booth open, deposit, restock food, return, further XP. |
| Options | `rockcrab_mage`: Staff of air 1381, Wind Strike, `Autocast.arm`, combat Magic XP with **no** inventory elemental runes. `rockcrab_range`: Maple shortbow 853 + Bronze arrow 882, Rapid, arrow consume or ground sweep. `rockcrab_junk_off`: `bankCommonJunk=false` does not deposit gem/casket unless in `loot`. Clue-true → pre-Start refusal. |
| Gate | Autocast family for mage; PeriodicBank execute for loot-count; DeathRecovery only on a death cell. |
| Fixture vs capability | Tele not in field / still tutorial → fixture. Mage starts, varp 108 never 3, elemental runes consumed → autocast capability. Loot-count never opens Seers booth → PeriodicBank capability. |

---

## 4. MossGiant

Source: `MossGiant/MossGiant.ts`. TARGET `'Moss giant'`. Safespot
`(2553,3406,0)`, Ardougne North bank `(2615,3332,0)`, r10. DeathRecovery.
Default melee/strength, lobster ×20, `buryBones=false`, `bankCommonJunk=true`.
DROP_DB includes Big bones, herbs, gems, spinach roll/coal/arrows stripped
from default loot.

| | |
|---|---|
| Core | Tele safespot, melee, solve/clue N/A (no SolveClue). Seed lobster 10. |
| Baseline | At safespot, 10 lobster, 0 big bones, no strength XP. |
| Core witness | Strength XP ≥ 1 and a Moss giant leave-scene. Optional loot of Big bones 532 (banked, not buried). |
| Bank cycle | Fill loot slots (default loot), walk `(2615,3332,0)`, deposit except food/runes/ammo/weapon, withdraw lobster, return, further XP. |
| Options | `moss_mage` / `moss_range` as RockCrab (Wind Strike + staff air; Maple shortbow + Iron arrow 884). `moss_bury`: `buryBones=true` → Prayer XP and Big bones consumed, not banked. `moss_junk_off`: `bankCommonJunk=false` does not pick uncut gems. |
| Gate | Autocast for mage; common-loot matcher for junk-on pickup; DeathRecovery only if death induced. |
| Fixture vs capability | No Moss giant at that tile → fixture. Matcher throws on `bankCommonJunk=true` → common-loot capability. |

---

## 5. GreenDragon

Source: `GreenDragon/{GreenDragon,GreenDragonLogic}.ts`. TARGET
`'Green dragon'`. Anchor `(3096,3814,0)` r22, Edgeville bank `(3094,3493,0)`.
Range **unavailable** (shield slot). Default melee, Rune scimitar 1333,
Dragonfire shield 1540, special on, potions on, lobster ×20, escape
`Flee to bank`, `solveClues=true` (inject false). DeathRecovery **anchors at
BANK_TILE**, not the field. DROP_DB `"Dragonhide"` → witness **1753 only**.

| | |
|---|---|
| Core | Wilderness field, `solveClues=false`, `useSpecial=false`, `usePotions=false`, `escape=Flee to bank`. Seed: scimitar, shield 1540, lobster 12, tele anchor. Combat 40+ recommended so the script is not food-starved in 150 ticks — if the script does not hard-stop on combat level, leave stats fresh and treat a panic-flee without XP as a **fixture** miss, not a pass. |
| Baseline | In field z≥3520, shield worn, 0 dragon bones / 0 hide 1753, no strength XP. |
| Core witness | Strength XP ≥ 1 **and** Dragon bones 536 or hide 1753 acquired after Start. |
| Bank cycle | Full pack / panic: walk Edgeville booth, deposit except keep-list, withdraw food (+ scimitar/shield if missing), return north of 3520, another kill. |
| Options | `greendragon_special`: `useSpecial=true`, dragon dagger 1215, energy ≥ 250, one spec spend (`%sa_attack`). `greendragon_mage`: Staff of fire 1387, Fire Strike, autocast, no inventory fire runes. `greendragon_tele`: `escape=Teleport to Varrock`, Magic 25, Law×1+Air×3+Fire×1 × (`TELE_STOCK`+1=3), panic path casts Varrock **and** lands (not a queued button). `greendragon_bury`: Prayer XP, bones 536 consumed. `greendragon_pker`: `Game.attackedByPlayer` true only with local face ≥ 32768; flee/tele runs. Clue-true → pre-Start refusal. |
| Gate | Special / autocast / `Game.teleport` / `attackedByPlayer` as combat design; DeathRecovery at bank tile; `suppliesOf` for potion qty. |
| Fixture vs capability | Missing shield (script requires it) / not in wilderness → fixture. Spec bar click never spends energy → special capability. Tele option never changes tile → teleport capability. |

---

## 6. FireGiant

Source: `FireGiant/{FireGiant,FireGiantLogic}.ts`. TARGET `'Fire giant'`.
Does **not** construct DeathRecovery (death in the dungeon **parks** if the
amulet is on the pile). Waterfall approach: raft (2510,3493,0), rope on
Rock, rope on Dead tree, Ledge, Door with Glarial's amulet 295. Melee
anchor `(2575,9893,0)`; mage/range safespot `(2568,9892,0)` fallback
`(2568,9893,0)`. Default escape `Barrel (free)` to `(2527,3413,0)` then
Ardougne West `(2616,3332,0)`. Default bank tile follows Camelot
`(2725,3491,0)` until escape is Barrel — implementer must set `bankTile`
to `BARREL_BANK` (2616,3332,0) when proving barrel, or leave Camelot and
seed that teleport.

Quest: `Quests.status('Waterfall Quest') === 'notStarted'` parks. Exact
waterfall varp is **unresolved** here; reuse the existing
`~completequests` + DrainDialogs path already in `crates/scenario/src/lib.rs`,
or a named setvar if root already has one. Do not invent a varp.

| | |
|---|---|
| Core | Melee, barrel exit, `bankTile=(2616,3332,0)`. Seed: Waterfall past notStarted, amulet 295, rope 954, lobster 12, tele raft stand (or already in dungeon `(2575,9893,0)` if the approach is a separate cell). |
| Baseline | Amulet held or worn, rope held, in melee room **or** at raft with quest started; 0 big bones; no strength XP. |
| Core witness | Strength XP ≥ 1 on a Fire giant. If the cell starts at the raft, also a post-Start tile with z≥9000 (inside dungeon) before that XP. |
| Bank cycle | Barrel (or chosen tele) out, booth, deposit except food/ammo/weapon/amulet/rope/escape runes, withdraw food, re-enter, another kill. |
| Options | `firegiant_mage` / `firegiant_range` from the west safespot (do not walk off 9892/9893). `firegiant_camelot`: `escapeTele=Camelot`, Magic 45, Air×5+Law×1, land `(2757,3478,0)`, bank Seers. One of Falador/Varrock/Ardougne is enough extra tele proof; do not run all four. `firegiant_bury`: Prayer XP on Big bones 532. |
| Gate | `Game.teleport` only for tele-escape cells; loc useOn rope; quest snapshot; banking family. |
| Fixture vs capability | Quest still notStarted / no amulet (script parks) → fixture. Rope useOn never moves the player → loc/useOn capability. Camelot option queues a button without landing → teleport capability. |

---

## 7. ArdyFighter

Source: `ArdyFighter/ArdyFighter.ts`. Default target `Guard`, anchor
`(2661,3306,0)` r12, bank `(2655,3286,0)`. Eats **stolen cakes only**
(CAKE_ITEMS: cake 1891, bread 2309, chocolate slice 1901). Loadout food
ignored. PeriodicBank + DeathRecovery r6 + SolveClue. Default loot names
are contains-match: clue scroll, blood/nature/chaos rune, body talisman,
steel arrow, iron ore.

| | |
|---|---|
| Core | Tele market, `solveClues=false`, PeriodicBank Off, `combatStyle=strength`. Seed Thieving 5 (`advancestat thieving 5` + drain) so Baker's stall restock can run; no pre-seeded cakes required. |
| Baseline | At anchor, 0 cakes, no strength XP. |
| Core witness | Strength XP ≥ 1 on a Guard **and** a post-Start cake/bread/slice acquire (stall steal) or consume. |
| Bank cycle | `Loot count` after stolen loot slots ≥ `bankAtLootSlots` (inject 2 for a short cell), deposit named loot (+ common junk if on), return to market, further XP. |
| Options | `ardyfighter_knight`: `target=Knight of Ardougne` (still melee). `ardyfighter_junk_off`. Clue-true → pre-Start refusal. Do not also vary leash. |
| Gate | PeriodicBank; cake stall loc `Baker's stall` / `Steal from` at (2667,3310,0); DeathRecovery to ANCHOR. |
| Fixture vs capability | No Guard at market → fixture. Loot-count never opens `(2655,3286,0)` booth → PeriodicBank. |

---

## 8. AutoFighter

Source: `AutoFighter/{AutoFighter,AutoFighterData}.ts`. Generic fighter.
Default target `Guard`, spot `Start position`, leash 8, melee/strength,
food blank → Trout 333 ×10, banking `Auto`, `solveClues=true` (inject
false), buryBones false. Default custom tile `(3273,3427,0)` is **not**
used unless `spot=Custom coordinates`.

| | |
|---|---|
| Core | Tele Ardougne Guard `(2661,3306,0)` so Start position **is** the market. `banking=None`, `solveClues=false`, `useSpecial=false`. Seed trout 8. |
| Baseline | On that tile, 8 trout, no strength XP. |
| Core witness | Strength XP ≥ 1. Optional Bones 526 only if `buryBones` on. |
| Bank cycle | `banking=Auto`, `bankAtLootSlots=1`, seed a listed loot name after a kill (uncut sapphire 1623), nearest-bank deposit, return to ANCHOR, further XP. |
| Options | `autofighter_mage`: Fire Strike + staff (must be wielded), autocast. `autofighter_range`: bronze arrows. `autofighter_special`: dragon dagger, energy spend. `autofighter_custom`: `spot=Custom coordinates` at Lumbridge chickens `(3235,3295,0)`, `target=Chicken` — only if that NPC display name exists in both selected worlds (unresolved here; if the name is missing, do not invent a substitute). `autofighter_bury`: Prayer XP on Bones 526. `autofighter_timed_bank`: `bankEveryMinutes` > 0 with loot in pack. Clue-true → pre-Start refusal. |
| Gate | Autocast / special as combat design; DeathRecovery; `scriptFood` already maps trout. |
| Fixture vs capability | Start tile has no matching NPC name → fixture. Mage never arms varp 108 → autocast. |

---

## 9. ArdyThiever

Source: `ArdyThiever/ArdyThiever.ts`. Default Guard, Flee, food from
cakes, bank `(2655,3286,0)`, FLEE_TILE `(2655,3298,0)`. Spots:
Guard/Knight `(2661,3306,0)` leash 19/29; Paladin `(2655,3311,0)` r12;
Hero `(2657,3311,0)` r17. Pickpocket op `Pickpocket`. Stun ticks from
generated facts (Guard 8). Thieving requirements 40/55/70/80.

| | |
|---|---|
| Core | `thieveTarget=Guard`, `guardResponse=Flee`, `solveClues=false`, PeriodicBank Off. Seed `advancestat thieving 40`, hitpoints 40, tele Guard stand. Cakes via stall (Thieving 5 is below 40; stall still works). |
| Baseline | At Guard stand, Thieving 40, 0 coins 995, no thieving XP beyond the advancestat baseline. |
| Core witness | Thieving XP ≥ 1 (468 per success at Guard) **and** coins 995 up after Start. Stun without XP is not a pass. |
| Bank cycle | `Loot count` once loot slots hit the injected threshold; deposit coins/loot; restock cakes; return; further pickpocket. |
| Options | `ardythiever_knight` (lv 55) **or** paladin (70) **or** hero (80) — one extra target, not all four. `ardythiever_fight`: `guardResponse=Fight`, `isHostileAttacker` true on a combat Guard not targeting another player, Fight XP, not FLEE_TILE. `ardythiever_flee`: caught steal walks FLEE_TILE `(2655,3298,0)`. Clue-true → pre-Start refusal. |
| Gate | `isHostileAttacker` for Fight; PeriodicBank; cake stall. |
| Fixture vs capability | Thieving 39 (script cannot succeed Guard) → fixture. Fight mode never sees HOSTILE_NAMES because the predicate throws → hostile-attacker capability. |

---

## 10. ArdyCakes

Source: `ArdyCakes/ArdyCakes.ts`. Baker's stall only. STAND `(2668,3312,0)`,
STALL `(2667,3310,0)`, bank `(2655,3286,0)`, FLEE `(2655,3298,0)`.
Requires Thieving 5 (throws otherwise). DeathRecovery r6 to STAND.
Default Flee. Food is cake forms; `foodWithdraw` 10 on recovery.

| | |
|---|---|
| Core | `guardResponse=Flee`, `solveClues=false`. Seed `advancestat thieving 5`, tele STAND. |
| Baseline | At STAND, 0 cake 1891, no thieving XP beyond seed. |
| Core witness | Thieving XP ≥ 1 **and** cake/bread/chocolate slice acquired after Start. |
| Bank cycle | Inventory cakes deposited at booth, return to STAND, another steal. This card has no Off; banking is the loop when the pack is full. |
| Options | `ardycakes_fight`: Fight + `isHostileAttacker` as ArdyThiever. Clue-true → pre-Start refusal (Fight drops medium clues). |
| Gate | Loc steal; hostile Fight; DeathRecovery to STAND. |
| Fixture vs capability | Thieving 4 throws at Start → fixture. Full pack never opens booth → bank open capability. |

---

## 11. GnomeMagicChopper

Source: `GnomeMagicChopper/GnomeMagicChopper.ts`. Settings: only
`fletchLogs` (default true). Magic tree pins (2372,3425,0) /
(2433,3410,0) / (2491,3413,0). Gnome bank `(2445,3425,1)` via stairs
(2444,3416,0) or (2445,3443,0). WC 75 is a **warning**, not a stop.
Fletch: knife 946 useOn logs, ChatDialog make `short` at Fletching 80 or
`long` at 85. Products are unstrung ids 72 / 70 even though the display
is `Magic shortbow` / `Magic longbow`. Death/gear recovery (Lumbridge
men, Bob steel axe Shop.buy 250 gp, Port Sarim boats) is **script-owned**,
not DeathRecovery; do not use that path as core proof.

| | |
|---|---|
| Core | `fletchLogs=false`. Seed `advancestat woodcutting 75`, steel axe 1353 wielded or held, tele west magics (2372,3425,0). |
| Baseline | At a Magic tree pin, axe held, 0 magic logs 1513, no WC XP beyond seed. |
| Core witness | Woodcutting XP ≥ 1 **and** magic logs 1513 up after Start. |
| Bank cycle | Full pack of 1513 deposited at gnome booth (level 1), return to a tree pin, another log. Stairs are part of the script, not a second nav policy. |
| Options | `gnome_fletch_short`: `fletchLogs=true`, Fletching 80, knife 946, witness **unstrung** 72 up and logs 1513 down (not strung 861). `gnome_fletch_long`: Fletching 85, unstrung 70. Missing knife at the gnome bank stops — that stop is the card's own failure, not a pass. Shop.buy axe is not a core cell; if exercised, stock/inv must move (combat-design Shop.buy). |
| Gate | Loc chop already mapped; ChatDialog.make / count-dialog for fletch; Shop.buy only on the axe-recovery cell; Banking.open upstairs booth. |
| Fixture vs capability | WC 1 at a Magic tree (may refuse chops) → fixture if no XP in 150 ticks. Fletch cell: knife useOn never opens make-menu / makeX answers before count dialog → production-dialog capability. Name-only witness that counts strung 861 as the unstrung product → invalid fixture. |

---

## 12. CoalTrucks

Source: `CoalTrucks/{CoalTrucks,CoalTrucksLogic}.ts`. **No SETTINGS
branches.** Mining 30 required (throws). Coal loc ids **2096, 2097**
hardcoded; Mine op; truck loc `Coal Truck`; Seers Remove-coal. Tiles:
mine `(2582,3481,0)`, mine truck stand `(2575,3486,0)`, Seers truck
`(2695,3503,0)`, Seers bank `(2725,3491,0)`. Truck max 120; haul pulls ≤ 4.

| | |
|---|---|
| Core + bank cycle | This card **is** a bank/truck cycle. Seed `advancestat mining 30`, a usable pickaxe (steel 1269 is enough at 30), tele mine. |
| Baseline | At mine, pickaxe held, 0 coal 453, no mining XP beyond seed. |
| Witness | Mining XP ≥ 1 and coal 453 up, **then** coal deposited to the mine truck (inv down without Seers bank yet), **then** Seers Remove-coal (inv up), **then** Seers booth deposit (inv coal down, bank coal up). Seeds sitting in the bank fail. |
| Options | None declared. Do not invent a powermine flag. |
| Gate | Loc interact; `Bank.openBooth` at Seers; `bestPickaxe` stays `not impl` — the script's own `bestPickaxe` JS helper is foreign; host owes item/loc ops not a cloned tool planner. |
| Fixture vs capability | Mining 29 throws → fixture. Loc 2096/2097 missing on 289 while the name `Rocks` exists under another id → **fixture/content** (record the 289 id; do not silently retarget). Booth open fails with the player on `(2725,3491,0)` → banking capability. |

---

## 13. GnomeCourse

Source: `AgilityBot/AgilityBot.ts` (card display `GnomeCourse`). Start
`(2474,3436,0)` r20. Default obstacles
`Log balance,Obstacle net,Tree branch,Balancing rope,Tree branch,Obstacle net,Obstacle pipe`.
Advances on agility XP. No bank, no combat.

| | |
|---|---|
| Core | Tele start, default obstacles, `searchRadius=20`. No skill seed required (log is low-level); if the first loc refuses, that is a fixture/content miss. |
| Baseline | At (2474,3436,0), no agility XP. |
| Core witness | Agility XP ≥ 1 **and** a tile change away from the clicked loc after Start. Queued loc op without XP fails. |
| Bank cycle | None. |
| Options | `gnome_radius`: `searchRadius=8` still clears the log from the start tile. Do not permute the obstacle CSV; a wrong name that never XP-advances is a fixture error. |
| Gate | Loc interact + XP wait (15 s in source). DirectNavigator to start if not already there. |
| Fixture vs capability | Tele to Varrock and the course never reached → fixture. On the log, interact never awards agility XP → loc/settle capability. |

---

## 14. WildyAgility

Source: `WildyAgility/{WildyAgility,WildyAgilityLogic}.ts`. Ridge Door
`(2998,3917,0)` Open from approach `(2998,3916,0)`. Course north of gate
`(2998,3931,0)`. Obstacles: obstacle pipe, ropeswing, stepping stone,
log balance, rocks. `RIDGE_MIN_AGILITY = 52`. Edgeville bank
`(3094,3493,0)`. DeathRecovery with **`walkBack`** (food-only bank then
ridge) — required, not the dim RoguesPurse case. Default food blank →
Lobster, `foodWithdraw=20`, `minFood=1`, `acquireFoodAtStart=true`.

| | |
|---|---|
| Core | `acquireFoodAtStart=false`, `minFood=0` (skip startup bank), seed `advancestat agility 52`, lobster 5, tele approach (2998,3916,0). |
| Baseline | South of the ridge, Agility 52, 5 lobster, no agility XP beyond seed. |
| Core witness | Ridge success (agility XP **or** `You skillfully balance across the ridge`) **and** tile north of gate z>3931. A queued Open without XP/message fails. Wolf-pit fail is not a pass. |
| Bank cycle | `acquireFoodAtStart=true`, `minFood=5`, pack food 0, `givebank lobster 40`: startup withdraw at Edgeville, then ridge. That is the supported food bank, not a loot bank. |
| Options | `wildy_death`: induce death on the course; recovery must bank food then reach `RIDGE_APPROACH` (queued walk is not recovery). `wildy_food_override`: `food=Shark` 385 withdrawn instead of lobster. |
| Gate | DeathRecovery `walkBack`; loc Open/obstacles; banking family for food. |
| Fixture vs capability | Agility 51 (ridge will fail/refuse) → fixture. Death cell: chat `oh dear` seen but never returns to the ridge → DeathRecovery capability. |

---

## 15. BrimhavenAgility

Source: `BrimhavenAgility/{BrimhavenAgility,BrimhavenAgilityLogic}.ts`.
Entrance `(2809,3194,0)`, fee 200 coins, tickets 2996, varp 309,
pillars 24, default food lobster ×25, `bankAtTickets=1000`,
`stealRestock=false`. Ardougne bank `(2655,3283,0)`. Karamja general
`(2902,3146,0)` for Shop.sell food-for-boat. `STEAL_THIEVING_MIN` 20
stall / 40 guards (throws at Start if steal on and Thieving < 20).

| | |
|---|---|
| Core | `stealRestock=false`, `bankAtTickets` left high. Seed Agility 1 is enough for several edges; coins 995 × 1000 (entrance+boat), lobster 10, tele entrance. |
| Baseline | At entrance or in arena, coins ≥ 200, 0 tickets 2996, no agility XP. |
| Core witness | Agility XP ≥ 1 from an arena obstacle **or** ticket 2996 up after a dispenser tag. Entrance fee coins down is supporting, not sufficient alone. |
| Bank cycle | Inject `bankAtTickets=1` after one ticket is held: leave arena, Ardougne booth, deposit tickets, withdraw food/coins, re-enter (pay 200), another tag/XP. |
| Options | `brim_steal`: `stealRestock=true`, Thieving 40, cake stall when food gone **and/or** Guard pickpocket for coins. `brim_sell_boat`: stranded without fare → Shop.sell food at Karamja general, coins up, inv food down (combat-design sell). Do not also permute every arena edge. |
| Gate | Loc obstacles; Shop.sell on the boat-fare cell; banking family; `isHostileAttacker` unused here. |
| Fixture vs capability | 0 coins at the clerk (cannot enter) → fixture. In arena, obstacle interact never XP → loc capability. Sell cell: `Shop.sell` throws → shop capability. |

---

## 16. HillGiant

Source: `HillGiant/{HillGiant,HillGiantLogic}.ts`. TARGET display
**`Giant`** (DROP_DB key `Giant`, not `Hill giant`). Pit spots
(3110,9832,0) … (3120,9843,0) r14. Varrock West bank `(3185,3440,0)`.
Brass key 983 at `(3131,9862,0)` for the hut shortcut; nav uses the
public trapdoor without it. Default loot Limpwurt 225 + Big bones 532,
`buryBones=false`, `bankCommonJunk=true`, food Trout ×12, blank weapon
= already worn. DeathRecovery with `walkBack` to the pit.

| | |
|---|---|
| Core | Tele a pit spot (3110,9832,0), `buryBones=false`, blank weapon, trout 8. |
| Baseline | In the pit, 8 trout, 0 limpwurt / 0 big bones, no strength XP. |
| Core witness | Strength XP ≥ 1 on a `Giant` **and** (limpwurt 225 or big bones 532) up after Start. |
| Bank cycle | `lootSlots` inject 1: after one loot slot, Varrock West booth, deposit except trout + brass key, withdraw 12 trout, return to a pit spot, another kill. |
| Options | `hillgiant_bury`: Prayer XP, bones 532 consumed not banked. `hillgiant_weapon`: `weapon=Rune scimitar`, withdraw+wield if missing. `hillgiant_junk_off`: `bankCommonJunk=false` does not pick uncut gems. `hillgiant_key`: no key in pack, pickup 983 at KEY_SPAWN then hut route (optional; trapdoor core must still pass without it). |
| Gate | DeathRecovery walk-back; `weaponOf` only if a named weapon cell is run; common-loot pickup. |
| Fixture vs capability | Tele to the surface hut without entering the pit → fixture. Named weapon cell: scimitar in bank, never wielded → loadout/weapon capability. |

---

## Case split (implementer order)

Reuse one scenario name per row; `CATALOG_SCENARIO` maps to the frozen
card as Alcher variants do. Clue-true rows are refusal checks, not
gameplay passes.

| Scenario | Card | Why this cell exists |
|---|---|---|
| `duel_arena` | Duel Arena Combat Trainer | Partner Challenge → Fight XP |
| `duel_arena_defence` | same | Defence style branch |
| `chaos_druid` | ChaosDruidKiller | Edgeville core + loot |
| `chaos_druid_bank` | same | Trapdoor bank return |
| `chaos_druid_tower` | same | Thieving 46 picklock |
| `chaos_druid_yanille` | same | Agility 40 warriors |
| `rock_crab` | RockCrab | Melee wake/kill |
| `rock_crab_bank` | same | PeriodicBank loot-count |
| `rock_crab_mage` | same | Autocast |
| `moss_giant` | MossGiant | Melee + big bones |
| `moss_giant_bank` | same | Ardougne N return |
| `green_dragon` | GreenDragon | Melee + bones/hide 1753 |
| `green_dragon_bank` | same | Edgeville return |
| `green_dragon_special` | same | Spec energy |
| `green_dragon_tele` | same | Varrock teleport landing |
| `fire_giant` | FireGiant | Dungeon melee |
| `fire_giant_bank` | same | Barrel + Ardougne West |
| `ardy_fighter` | ArdyFighter | Guard + cake restock |
| `ardy_fighter_bank` | same | PeriodicBank |
| `auto_fighter` | AutoFighter | Start-position Guard |
| `auto_fighter_bank` | same | Auto nearest-bank |
| `ardy_thiever` | ArdyThiever | Guard pickpocket coins |
| `ardy_thiever_fight` | same | Hostile Fight vs Flee |
| `ardy_cakes` | ArdyCakes | Stall XP + cake 1891 |
| `ardy_cakes_bank` | same | Cake deposit return |
| `gnome_chop` | GnomeMagicChopper | WC logs 1513 |
| `gnome_chop_bank` | same | Upstairs booth |
| `gnome_chop_fletch` | same | Unstrung 72, not 861 |
| `coal_trucks` | CoalTrucks | Mine → truck → Seers bank |
| `gnome_course` | GnomeCourse | First obstacle XP + tile |
| `wildy_agility` | WildyAgility | Ridge north of gate |
| `wildy_agility_food` | same | Startup food withdraw |
| `brimhaven_agility` | BrimhavenAgility | Arena XP or ticket 2996 |
| `brimhaven_agility_bank` | same | Ticket deposit + re-enter |
| `hill_giant` | HillGiant | Giant + limpwurt/bones |
| `hill_giant_bank` | same | Varrock West return |
| `hill_giant_bury` | same | Prayer vs bank bones |
| `*_clue_refuse` | clue constructors | Pre-Start refusal only |

Do not add mage/range/special/tele/steal cells to this first wave unless
the matching Rust family is already reviewed. Core + bank-cycle + the
one distinct option that the card cannot fake with melee Off is enough
to start.

## Unresolved (do not guess)

- Combat NPC type ids for Rock Crab / Rocks, Moss giant, Green dragon,
  Fire giant, Giant, Chaos druid / warrior, Chicken (AutoFighter custom).
  Generated JSON has pickpocket NPCs only.
- 289 loc ids if coal rocks 2096/2097 or gnome/wildy/brimhaven obstacle
  names differ; verify against selected loc archives at implementation.
- 289 duel modal component ids if the interface archive disagrees with
  6575/6412/6733.
- Waterfall Quest varp name; use existing `~completequests` or a known
  setvar, not an invented one.
- RockCrab loot string `small oyster pearls` vs generated `Oyster pearl`
  (411). Do not retarget the script; do not witness that drop.
- AutoFighter `target=Chicken` display-name existence on both worlds.

## Explicitly not claimed

Clue solving, quester/gatherer/MarketMaker rewrites, import-blocked
`BrimhavenMossGiants`, dim `RoguesPurse`. A fixture that skips those
required branches must not mark the row supported. Off PeriodicBank and
idle DeathRecovery on core cells match source when those options are
disabled; they do not prove the queued execute paths.
