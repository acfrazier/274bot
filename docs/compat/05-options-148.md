# Alternate camp and Fight guard-response option branches (brief 148)

## Scope

Four named cells and nothing else: `chaos_druid_tower`, `chaos_druid_yanille`,
`ardy_cakes_fight`, `ardy_thiever_fight`. The work is scenario/core proof
fixtures, the shared host-play core witness, its regressions, this report and
`evidence/options-148/`. No runtime, client, JavaScript, matrix, ledger or STATE
change; no LIVE. Root owns live acceptance.

Brief 145's audit and `06ao-combat-utility-option-audit.md` are proposal lists,
so every id, value, position, statistic and piece of gear below was re-derived
from the frozen card sources under `.superpowers/inputs/rs2b0t-<catalog>/`
(identity-checked by SHA-256 in the suite) and from
`crates/api/data/game-data/274.json` / `289.json`. What each branch actually does
is recorded per case in `evidence/options-148/source-declarations.json`.

## Fixtures

Each case is a separate scenario; the existing Edgeville `chaos_druid`, Flee
`ardy_cakes` and Flee `ardy_thiever` fixtures are untouched.

| Case | Card | Prepared before Start | Injected source options |
| --- | --- | --- | --- |
| `chaos_druid_tower` | ChaosDruidKiller | Attack/Strength/Hitpoints 40, Lobster 8, Adamant scimitar 1331, Thieving 46, empty selected loot pack, teleport to the Tower surface stand (2562,3356,0) | `location=Chaos Druid Tower`, `combatStyleIndex=1`, `loot=Herb,Law rune,Nature rune`, `food=Lobster`, `foodWithdraw=8`, `buryBones=false`, `solveClues=false` |
| `chaos_druid_yanille` | ChaosDruidKiller | Attack/Strength/Hitpoints 40, Lobster 8, Adamant scimitar 1331, Agility 40, empty selected loot pack, teleport into the warrior room (2580,9501,0) | `location=Yanille Dungeon`, same style/loot/food/bury/clue defaults as Tower |
| `ardy_cakes_fight` | ArdyCakes | Thieving 5, Attack/Strength/Hitpoints 40, 22 retained Knives, worn Adamant scimitar 1331, empty cake pack, Baker's stall stand | `guardResponse=Fight`, `solveClues=false` |
| `ardy_thiever_fight` | ArdyThiever | Thieving 40, Attack/Strength/Hitpoints 40, worn Adamant scimitar 1331, empty pack, market Guard stand | `thieveTarget=Guard`, `guardResponse=Fight`, `bankAtLootSlots=1`, `solveClues=false` |

Preparation, acknowledgement and the teleports into the Tower surface, Yanille
warrior room and Ardougne market all happen before the frozen script Starts;
every post-Start step is a pure witness. No loot, guard, RNG, pickup or energy
state is seeded as a result. The 180-second script deadline and the existing
150-tick watch budgets are unchanged.

`crates/scenario/src/lib.rs` reuses `combat_core_scenario` for the two druid
camps (with Thieving / Agility prerequisites on the shared plan) and adds two
dedicated Fight builders beside the existing Flee Ardy fixtures.

## Qualification

The shared witness in `crates/host-play/src/catalog_core.rs` carries branch-
specific cycles beside the existing Edgeville combat and Flee thieving cells:

- **Tower** (`chaos_druid_tower`): `CombatCoreCycle` at the Tower field with
  target display `Chaos druid`, Strength XP, and selected Herb/Law/Nature pickup.
  Baseline requires Thieving 46 and refuses the Edgeville field stand.
- **Yanille** (`chaos_druid_yanille`): same combat cycle shape with target display
  `Chaos druid warrior` and Agility 40. Baseline refuses the Tower stand and a
  plain Chaos druid identity.
- **Cakes Fight** (`ardy_cakes_fight`): Baker's stall Cake 1891 + Thieving XP,
  then a verified Guard defeat with Strength XP, without landing on the Flee kite
  tile (2655,3298,0). Chocolate cake / bread / chocolate slice are not stall food.
- **Thiever Fight** (`ardy_thiever_fight`): pickpocket coins + Thieving XP, verified
  Guard defeat with Strength XP (no Flee kite), then the same coins bank /
  closed return / further pickpocket cycle as the Flee cell.

`combat_spec` / case baselines refuse a wrong preparation before Start: low
Thieving or Agility, wrong field tile, missing combat kit on Fight cells, or a
melee loadout that cannot FightBack.

Baseline and cycle regressions live in
`crates/host-play/tests/catalog_boundary_live.rs`
(`alternate_druid_camps_need_their_own_field_prereqs_and_target_identity`,
`fight_guard_response_needs_a_kill_and_rejects_the_flee_kite`). Each named false
positive is held across the observed frames so a later frame cannot mask it.

Inject regressions
(`option_branch_injects_match_both_frozen_card_schemas`,
`camp_and_fight_option_injects_match_frozen_card_schemas`) parse each card's own
`SETTINGS` schema out of the SHA-verified frozen source plus the keys the card
reads through `this.settings.<kind>`, and refuse an injected id the card does
not read or a value outside the card's declared type, option list and numeric
bounds. They cover both catalogs and both client revisions.

## Verification and limits

Implementation on `codex/rs2b0t-multirevision`, client gitlink
`52c37f9ce50d1f184656d5b4469c007ec8a5791a`. Checks ran against the worktree with
required features (see `evidence/options-148/isolated-checks.log`): scenario 109
passed, host-play library 177 passed, `catalog_boundary_live` 56 passed with its
one LIVE test ignored. No LIVE process was launched.

Unresolved constraints, reported rather than worked around:

1. **No LIVE.** The four catalog x revision cells are root-owned and unproven
   here. Tower door picklock and Yanille web/ledge approach are out of cell;
   fixtures teleport onto the prepared field stands.
2. **Both frozen cards are byte-identical across the two catalogs** for the
   scripts used here, so these four cells currently differ only by client
   revision, not by catalog.
3. **Fight on ArdyCakes does not bank.** The frozen Fight path steals then
   FightBack; the cell witnesses steal + kill and refuses the Flee kite. Full
   bank-on-full packing remains the Flee cell.
4. **The support matrix does not yet list these four cells.** The LIVE harness
   resolves its card row by card name, so the cells are dispatchable
   (`CATALOG_SCENARIO=chaos_druid_tower|chaos_druid_yanille|ardy_cakes_fight|ardy_thiever_fight`),
   but `docs/compat/support-matrix.json` carries no per-case status or proof ref
   for them; matrix refresh is root-owned and this task did not touch it.
