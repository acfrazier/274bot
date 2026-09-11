# Remaining combat and utility behavior branches (145 correction)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 16:55 UTC. Kind: bounded read-only corrective audit of brief
145 after audit143/t_46d26f49. Own only this report and
`docs/compat/evidence/combat-utility-option-audit/`. No code, build, LIVE,
STATE, support-matrix, ledger, or new cards. Root owns acceptance.

Read once: `AGENTS.md`, `docs/execution.md`, briefs 143 and 145. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Same fifteen-card scope as
143. Root is independently running/fixing existing cores; this audit does not
duplicate Mage132, fixture139 (GreenDragon wear / RockCrab dormant), or
native144 (FireGiant npcBox).

## Prior audit 143 (dated, not the remaining set)

Written 2026-09-11 16:42 UTC on campaign HEAD `7706f053`. Committed
`c9331e833`. Machine copy:
`evidence/combat-utility-option-audit/remaining-cells-143.json`.

143 named five extra scenarios (20 cells): `thiever_bank`,
`chaos_druid_tower`, `chaos_druid_yanille`, `ardy_cakes_fight`,
`ardy_thiever_fight`. It labeled AutoFighter `banking=Auto`, RockCrab
ammo/bank, GreenDragon special/potions, FireGiant approach/barrel/bank and
ArdyFighter Loot-count as later because those cores were still unqualified.
Tower/Yanille witnesses said honest loot if any. Thiever Auto was called
unwitnessed.

Root did not accept that as the complete remaining set. Unqualified core is a
dependency, not a scope exclusion. This file keeps 143 as the prior result and
corrects those three gaps.

## Corrected verdict

**Eighteen named extra scenarios (seventy-two catalog×revision cells), gated
after the already-named cores, plus those cores that still lack accepted
isolated PASS.** Drop `thiever_bank` as four extra cells: the Auto food
restock path is already witnessed on the exact frozen Thiever script by fleet
N1/N32. Restore actual selected loot on Tower/Yanille. Do not mark final PASS.
Do not dim. Do not invent spell/ammo lists, melee XP fields, Time/Either bank
triggers, or per-card mage cells.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `324d7b5946534db28305942d9bfba75fdd2e5dc8` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 145 SHA-256 | `c97590b29bb3864b4a7fd0717a5f26465d7aa6530ca709db7e6c9f5e97a2243a` |
| Brief 143 SHA-256 | `8e9da0c2512f7312e23582adc54bdd3280cd7de36047eb3ad1ed3875ccf58ff4` |
| Kanban card | `t_a7f32d56` (parent `t_46d26f49` / `c9331e833`) |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Isolated chicken/thiever core | `f6b9ec4b2e7afe22f4a5bb58a837f3b9a992cb5f` |
| Isolated chicken bank + gnome lap | `b1cff8a7fdf6d18a5f7a3c7c23c6fa8a020e173b` |
| Isolated door/gate | `6401d3a4a0fd` |
| Isolated Ardy thieve bank cycles | `91289e9bac158721849209a301dcae5c26988eb9` |
| Isolated Wildy 8c | `8c56eb85a846` |
| Fleet Thiever Auto | host `6d750e65` / client `56d8027` / catalog `8e7d965` / script `5cff25a75db3166a3bf3ab6ca1a8ba37ed77e0e9d48292178f79767316b66ad9` |
| Named cff Chaos diagnostics | `cffd62c598ceab2ea80b9bcc29e0eaf58f4dd885` / `evidence/combat-ground-serialization/fresh-chaos-diagnostics.json` |

Card/sibling SHA-256 values remain
`evidence/combat-utility-option-audit/source-hashes.json`. `cmp` is 0 on both
catalogs for every listed sibling. Concurrent working-tree edits are not a
tested binary.

## Corrections

### 1. Later bank/escape/range is still required

Native PeriodicBank / Special / Autocast existence, and Chicken Loot-count
PASS, do not qualify another card's source option. Each distinct branch is
either witnessed, mapped to an owned cell, or named and gated after its core.

| 143 label | Source | Corrected disposition |
|---|---|---|
| AutoFighter banking Auto | custom Bank, not PeriodicBank; core injects None | `auto_fighter_bank` gated after `auto_fighter` |
| RockCrab bank | PeriodicBank Loot-count at Seers + food return | `rock_crab_bank` gated after `rock_crab` |
| RockCrab ammo | `AmmoLogic.sweepPlan` + projectile equip | `rock_crab_range` gated after `rock_crab` |
| GreenDragon special | `Special.ready`/`arm`; core injects false | `green_dragon_special` gated after `green_dragon` |
| GreenDragon potions | `boostPotions` mid-fight sip; not Eat | `green_dragon_potions` gated after `green_dragon` |
| GreenDragon escape/bank | Flee-to-bank / pack restock Edgeville | `green_dragon_bank` gated after `green_dragon` |
| GreenDragon Teleport | distinct `escape` setting | `green_dragon_tele` gated after core **and** brief 37 `Game.teleport` |
| FireGiant approach | `EnterDungeon` raft/rope/ledge; core starts in-room | `fire_giant_approach` gated after `fire_giant` |
| FireGiant barrel/bank | Barrel wash-up + Ardougne West restock | `fire_giant_bank` gated after `fire_giant` |
| ArdyFighter Loot-count | PeriodicBank after stolen loot; core Off | `ardy_fighter_bank` gated after `ardy_fighter` |
| Chaos/Moss/Hill bank | script-owned trip end, not PeriodicBank | `chaos_druid_bank` / `moss_giant_bank` / `hill_giant_bank` gated after cores |
| All range = substitution | Chicken/AutoFighter ammo withdraw+equip; RockCrab sweep | `auto_fighter_range` family cell; `rock_crab_range` separate |

Chicken Time/Either still only flips `shouldBankNow` on the already-witnessed
PeriodicBank trip. Melee XP fields stay the same `setCombatStyle` dispatch.
Per-card mage stays Mage132 (`auto_fighter_mage`). AutoFighter `useSpecial`
maps to `green_dragon_special`. FireGiant spell teles map to
`green_dragon_tele`; Barrel does not.

### 2. Thiever Auto is not globally unwitnessed

Exact identity vs frozen Thiever:

- Registered display_name `Thiever` → `ThievingBot.ts` hash `5cff25a75db3…`
  on **both** catalogs (source-hashes `equal: true`).
- Fleet controller is **not** catalog-harness isolated `thiever`. It is the
  TUI memory IsolatedEnv overlay plus `crates/host-play/src/memory.rs`
  `start_script` when `BOT_MEMORY_SUSTAIN=1`, which posts `banking=Auto`,
  loadout `Memory food` / Lobster, `foodWithdraw=22`, `bankAtFood=3`.
- Isolated `thiever` still injects `target=Guard`, `banking=None` at Ardougne
  (not catalog default Man). Guard vs Man is Pickpocket substitution.
- Evidence: `docs/compat/evidence/fleet-prerequisite/README.md`,
  `n1-results-6d750e65.json` PASS both revisions (274 observation gain 2,902
  XP, 289 2,574), N32 274 and 289 qualification.json PASS all 32 actors with
  real food depletion, loaded bank, replenish to 22, return, further XP.
- Host `6d750e65`, catalog `8e7d965` only. Old-catalog siblings are
  byte-identical; that is not a second Auto mode.

This **qualifies the script Auto bank path**. It does **not** automatically
create four extra `thiever_bank` catalog-harness cells. Final integrated N32
refresh remains a campaign-wide gate.

### 3. Tower/Yanille loot stays the real transition

`Loot.validate` requires `!Game.inCombat()` and a wanted drop. Matcher is
exact `herb` / `law rune` / `nature rune` (unidentified Herb 199-family, Law
563, Nature 561). The agreed combat core already requires that pickup after
Start. 143's "honest loot if any" is withdrawn. Timeout without the pickup is
FAIL. Do not force loot RNG, dim, or change predicates/clocks. Same rule on
the existing Edgeville `chaos_druid` core.

## Per card (corrected)

ChickenKiller — none extra of its own. f6 melee + b1 Loot-count stand.
Range/ammo maps to `auto_fighter_range`. Mage → Mage132.

Thiever — none extra isolated cells. f6 Guard/`banking=None`. Auto → fleet
N1/N32 on the same script. Suicide is the out-of-food continue path.

ChaosDruidKiller — Edgeville core still open (loot required). Extra:
`chaos_druid_bank`, `chaos_druid_tower`, `chaos_druid_yanille`.

MossGiant / HillGiant — existing cores only, plus gated script-owned bank
cells. Mage/range map as above. Bury is ancillary.

AutoFighter — existing melee core; mage is Mage132. Extra: `auto_fighter_bank`,
`auto_fighter_range`. Special maps to `green_dragon_special`. Clues stub.

RockCrab — existing melee core; fixture139 owns dormant stand. Extra:
`rock_crab_bank`, `rock_crab_range`. Mage → Mage132.

GreenDragon — existing melee core; fixture139 owns wear. Extra: bank, special,
potions, tele. No range (shield slot). Clues stub.

FireGiant — existing in-room melee core; native144 owns npcBox. Extra:
approach, barrel+bank. Spell teles map to `green_dragon_tele`.

ArdyFighter — existing melee core. Extra: `ardy_fighter_bank`. Time/Either
same PeriodicBank trip. Clues stub.

ArdyCakes — Flee core still open (pack-full bank is that loop). Fight extra.

ArdyThiever — knight Flee bank covered (`912` all four). Guard Flee 289 PASS /
274 FAIL retained. Fight extra. Paladin/Hero substitution.

GnomeCourse / WildyAgility / DoorOpener — none remaining as options (same as
143: complete-lap, course except retained 289-old FAIL, door+gate).

## Minimal named extra set

| Scenario | Inject | Witness | Cells |
|---|---|---|---|
| `auto_fighter_bank` | `banking=Auto` | Nearest-bank deposit, return, further XP | 4 |
| `auto_fighter_range` | `combatStyle=range` | Ranged XP + ammo consume/quiver | 4 |
| `rock_crab_bank` | `Loot count` | Seers booth, restock, return, further XP | 4 |
| `rock_crab_range` | `combatStyle=range` | Rocks→Crab, Ranged XP, sweep/consume | 4 |
| `green_dragon_bank` | `escape=Flee to bank` | Edgeville booth, return z>=3520, loot 536/1753 | 4 |
| `green_dragon_special` | `useSpecial=true` | Spec energy spend during a kill | 4 |
| `green_dragon_potions` | `usePotions=true` | Mid-fight sip + boost | 4 |
| `green_dragon_tele` | `escape=Teleport to Varrock` | Spellbook land + Magic XP (gated on brief 37) | 4 |
| `fire_giant_approach` | start at raft | Post-Start z>=9000 then Strength XP | 4 |
| `fire_giant_bank` | `escapeTele=Barrel (free)` | Barrel, Ardougne West, re-enter, further XP | 4 |
| `ardy_fighter_bank` | `Loot count` | Stall steal, booth, return, further XP | 4 |
| `chaos_druid_bank` | Edgeville | Booth, trapdoor return, further kill+loot | 4 |
| `chaos_druid_tower` | `location=Chaos Druid Tower` | Attack + Herb/Law/Nature pickup | 4 |
| `chaos_druid_yanille` | `location=Yanille Dungeon` | Warrior Attack + Herb/Law/Nature pickup | 4 |
| `moss_giant_bank` | melee | Ardougne North restock/return | 4 |
| `hill_giant_bank` | strength | Varrock West restock/return | 4 |
| `ardy_cakes_fight` | `guardResponse=Fight` | Stall steal then FightBack, not Flee | 4 |
| `ardy_thiever_fight` | `guardResponse=Fight` | FightBack then bank/return | 4 |

Seventy-two extra cells. Existing cores stay on `scenario::get` / `names()`.
Do not change frozen catalog source. Do not force loot/combat RNG.

## What this is not

- Not final card PASS and not a dim.
- Not four extra `thiever_bank` catalog-harness cells.
- Not a new Autocast, Special ABI, npcBox, or wear/dormant-stand task.
- Not induced-death LIVE and not a death-recovery capability gap.
- Not cff batch acceptance; not an Edgeville wilderness fixture move.
- Not integrated-source N32 refresh (campaign-wide).
- Not Cartesian ammo names, melee XP fields, Time/Either, or per-card mage.

Machine copies: `evidence/combat-utility-option-audit/{refs,source-hashes,frozen-settings,isolated-pass,remaining-cells,remaining-cells-143}.json`.
