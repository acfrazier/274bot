# Remaining combat and utility behavior branches

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 16:42 UTC. Kind: bounded read-only campaign audit of brief
143. Own only this report and `docs/compat/evidence/combat-utility-option-audit/`.
No code, build, LIVE, STATE, support-matrix, ledger, or new cards. Root owns
acceptance.

Read once: `AGENTS.md`, `docs/execution.md`, brief 143. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Frozen cards+siblings are the two
catalog inputs. Isolated PASS is taken from named harvest summaries/core-results,
not lexicographic last `host_commit` and not giant logs. Concurrent working-tree
edits are not a tested binary. Fresh `cffd62c59` combat is not a complete batch;
only the named Chaos diagnostics file is cited for those two cells.

## Verdict

**Replace “remaining supported settings” / stale BLOCKED-missing-bank/recovery
text on these fifteen cards with five named extra scenarios (twenty
catalog×revision cells), plus the already-named cores that still lack accepted
isolated PASS.** Do not mark final PASS. Do not dim. Do not invent spell/ammo
lists, melee XP fields, Time/Either bank triggers, or per-card mage cells.

New still-needed scenarios:

1. `thiever_bank` — `banking=Auto` food restock.
2. `chaos_druid_tower` — `location=Chaos Druid Tower` (Thieving 46).
3. `chaos_druid_yanille` — `location=Yanille Dungeon` (Agility 40, warrior).
4. `ardy_cakes_fight` — `guardResponse=Fight`.
5. `ardy_thiever_fight` — `guardResponse=Fight`.

Already-named cores still unqualified: `chaos_druid` (Edgeville), `moss_giant`,
`hill_giant`, `auto_fighter`, `rock_crab`, `green_dragon`, `fire_giant`,
`ardy_fighter`, `ardy_cakes` Flee. Leave cff refresh to root. Mage132 owns
`auto_fighter_mage`. Clue stubs stay stubs. Brimhaven is already dimmed.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `7706f053b2a71069e4487cd990784b7258e97552` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 143 SHA-256 | `8e9da0c2512f7312e23582adc54bdd3280cd7de36047eb3ad1ed3875ccf58ff4` |
| Kanban card | `t_46d26f49` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Isolated chicken/thiever core | `f6b9ec4b2e7afe22f4a5bb58a837f3b9a992cb5f` |
| Isolated chicken bank + gnome lap | `b1cff8a7fdf6d18a5f7a3c7c23c6fa8a020e173b` |
| Isolated door/gate | `6401d3a4a0fd` |
| Isolated Ardy thieve bank cycles | `91289e9bac158721849209a301dcae5c26988eb9` |
| Isolated Wildy 8c | `8c56eb85a846` |
| Named cff Chaos diagnostics | `cffd62c598ceab2ea80b9bcc29e0eaf58f4dd885` / `evidence/combat-ground-serialization/fresh-chaos-diagnostics.json` |

Card/sibling SHA-256 values: `evidence/combat-utility-option-audit/source-hashes.json`.
`cmp` is 0 on both catalogs for every listed sibling.

Registered names vs source path: Thiever → `ThievingBot.ts`; GnomeCourse →
`AgilityBot.ts`. No other identity mismatch on the fifteen cards.

## Method

Meaningful remaining work is default vs bank/return/restock, combat
mode/autocast/ranged, death-recovery policy, or a different location/guard
task. Not a Cartesian product of scalars, spell names, ammo, or melee XP
fields. One default does not qualify every string name. The same generated
action on another content row is data substitution. Unknown content/API is
not substitution. Matrix `BLOCKED: missing periodic bank/death recovery` is
stale: native PeriodicBank, DeathRecovery and Autocast.arm already exist.
Do not weaken predicates/clocks or dim FAILs. Do not prescribe post-Start
fixture cheats or forced combat/loot RNG.

## Per card

### ChickenKiller — none remaining

Covered on isolated `f6` all four: default melee, `bankStrategy=Off`,
`buryBones=true` (Strength 12, Prayer 4, Hitpoints 3). Covered on isolated
`b1` all four: `bankStrategy=Loot count`, `lootMatch=feather`, deposit,
closed return, further feathers. `Time`/`Either` only change
`shouldBankNow` in `bankRules.ts`; the bank trip is the same PeriodicBank
run. Mage is ArmAutocast, owned by Mage132, not a second Chicken cell.
Range/ammo names are CombatStyleLogic substitution. DeathRecovery idle is
not a missing capability.

### Thiever — one remaining (`thiever_bank`)

Covered: default Man / Pickpocket / `banking=None`, isolated `f6` all four
(Thieving 46). Other `PICKPOCKET_TARGET_NAMES` are the same Pickpocket op
on another NPC. `suicide` is the out-of-food continue path, not a bank
mode. Auto banking withdraws food and returns; unwitnessed.

### ChaosDruidKiller — two extra locations; Edgeville core still open

`DRUID_SPOTS` has three keys. Default Edgeville Dungeon `(3110,9936,0)` is
the existing `chaos_druid` cell. Isolated 04d7/214/b18 FAILs retained.
Named cff diagnostics (not batch acceptance): both older-catalog cells
timed out `looted=false`, no serializer panic, zero Ground calls; 274 has
6 script Attack calls, 289 has none and stays on native retaliation/eating
with Strength XP. Orch source note, not verified in this checkout:
`Player.isInWilderness` treats underground `z>=9920` as wilderness, hunt
`nottoostrong` applies only outside wilderness, and frozen `Loot.validate`
requires `!Game.inCombat`. That can starve loot under continuous combat.
It is not a conclusive script defect. Do not move the fixture, alter aggro,
or force loot RNG.

Tower and Yanille are distinct remaining locations (see named set).
`combatStyleIndex` 0–3 is the same `setCombatMode` dispatch.

### MossGiant — existing core only

Default melee/strength, bury off, DeathRecovery idle. b18 and 214 274-old
PASS are diagnostic, not card acceptance. Newer-catalog / 289 lack accepted
isolated PASS. Mage is Autocast (Mage132). Range/ammo substitution. Banking
is not the core cell.

### HillGiant — existing core only

Melee only. Target display Giant. 04d7/214 FAIL retained. In-progress cff
rows are not a complete batch. `shouldBank` is trip-end logic, not an
Off/Auto setting. `walkBack` DeathRecovery exists natively.

### AutoFighter — existing melee core; mage is Mage132

Covered fixture injects `banking=None`, special/clues off, melee/strength
Guard. 04d7/214 FAIL retained. Catalog default `banking=Auto` is a later
restock mode, not a current extra cell on an unqualified core.
`auto_fighter_mage` landed as a fixture on this HEAD; this audit does not
duplicate it or claim LIVE. Range is the same CombatStyleLogic family.
`solveClues` is a stub.

### RockCrab — existing core only

Default melee plus native Rocks → Rock Crab activation. 214 FAIL retained.
PeriodicBank native exists; core injects bank off. Ammo sweep is later,
after melee. Mage is Autocast.

### GreenDragon — existing core only

Default melee, shield 1540, bones 536 / hide 1753. 214 FAIL retained.
`escape` Teleport to Varrock awaits existing teleport work. `useSpecial` /
`usePotions` are later. Mage is Autocast. Clues stub.

### FireGiant — existing core only

Default melee already in the east room, Waterfall Quest, Glarial 295, rope
954, `escapeTele=Barrel (free)`. 214 FAIL retained. Camelot/Ardougne/
Falador/Varrock are the same teleport family. Approach/barrel/bank stay
unqualified on the core.

### ArdyFighter — existing core only

Default Guard/strength, post-Start stall steal, then combat. 214 FAIL
retained. `bankStrategy` Off vs Loot-count is later. Clues stub.

### ArdyCakes — Flee core still open; Fight extra

Default Flee stall+bank cycle: all isolated FAIL retained. audit142: native
guard LOS catch, keep enabled. `guardResponse=Fight` constructs `FightBack`
instead of `Flee` — named extra below. Clues stub.

### ArdyThiever — knight Flee covered; Guard 274 FAIL retained; Fight extra

Isolated `912`: knight Flee bank cycle PASS all four. Guard Flee PASS on
289 both catalogs; 274 both FAIL retained. Paladin/Hero are Pickpocket
substitution. Fight is the other guard task.

### GnomeCourse — none remaining

Default radius-20 complete lap + next log, isolated `3be`/`b1` all four.
`gnome_course_radius` (`searchRadius=8`) is the 05k imported AgilityBot
resync defect. Do not dim the default card. Obstacle CSV is the course
list, not a second mode.

### WildyAgility — none remaining as options

Isolated `8c`: 274 both catalogs and 289 newer PASS (full obstacles +
further pipe). 289 old FAIL after rope fall at HP10 retained. 15e/cd49
FAIL retained. Food names are eat-item substitution. `acquireFoodAtStart`
and timeout ticks are scalars. DeathRecovery walkBack is native.

### DoorOpener — none remaining

Isolated `6401` door and gate, all eight cells: selected closed loc
replaced by its open loc after Start. Stand/leash scalars are not modes.

## Minimal named extra set

| Scenario | Inject | Witness | Cells |
|---|---|---|---|
| `thiever_bank` | `banking=Auto`, Man, Pickpocket | Food withdraw, closed bank, return, further pickpocket XP | 4 |
| `chaos_druid_tower` | `location=Chaos Druid Tower` | Attack Chaos druid at tower field; Thieving 46; honest loot if any | 4 |
| `chaos_druid_yanille` | `location=Yanille Dungeon` | Attack Chaos druid warrior; Agility 40; honest loot if any | 4 |
| `ardy_cakes_fight` | `guardResponse=Fight` | Stall steal then FightBack kill, not Flee kite | 4 |
| `ardy_thiever_fight` | `guardResponse=Fight` | Guard pickpocket then FightBack, then bank/return | 4 |

Twenty extra cells. Existing cores above stay on `scenario::get` / `names()`.
Do not change frozen catalog source. Do not force loot/combat RNG.

## What this is not

- Not final card PASS and not a dim.
- Not a demand to click native/TUI controls on every remaining tier.
- Not a new Autocast task (Mage132) and not a new teleport task.
- Not a death-recovery capability gap.
- Not cff batch acceptance; not an Edgeville wilderness fixture move.
- Not integrated-source N32 refresh.

Machine copies: `evidence/combat-utility-option-audit/{refs,source-hashes,frozen-settings,isolated-pass,remaining-cells}.json`.
