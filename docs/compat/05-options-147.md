# Ranged and combat-consumable option branches (brief 147)

## Scope

Four named cells and nothing else: `auto_fighter_range`, `rock_crab_range`,
`green_dragon_special`, `green_dragon_potions`. The work is scenario/core proof
fixtures, the shared host-play core witness, its regressions, this report and
`evidence/options-147/`. No runtime, client, JavaScript, matrix, ledger or STATE
change; no LIVE. Root owns live acceptance.

Brief 145's audit is a proposal list, so every id, value, position, statistic and
piece of gear below was re-derived from the frozen card sources under
`.superpowers/inputs/rs2b0t-<catalog>/` (identity-checked by SHA-256 in the
suite) and from `crates/api/data/game-data/274.json` / `289.json`. What each
branch actually does is recorded per case in
`evidence/options-147/source-declarations.json`.

## Fixtures

Each case is a separate scenario; the existing melee `auto_fighter`, `rock_crab`
and `green_dragon` fixtures are untouched.

| Case | Card | Prepared before Start | Injected source options |
| --- | --- | --- | --- |
| `auto_fighter_range` | AutoFighter | Ranged 40, Hitpoints 40, Trout 8, worn Maple shortbow 853 and Bronze arrow 882 x200, then a pre-Start teleport to the Ardougne Guard | `target=Guard`, `spot=Start position`, `combatStyle=range`, `rangeStyle=rapid`, `ammo=Bronze arrow`, `ammoWithdraw=200`, `food=Trout`, `foodWithdraw=8`, `banking=None`, `solveClues=false`, `useSpecial=false`, `buryBones=false` |
| `rock_crab_range` | RockCrab | Ranged 40, Hitpoints 40, Lobster 8, worn Maple shortbow 853 and Bronze arrow 882 x200, dormant Rocks acknowledged at the safe stand | `combatStyle=range`, `rangeStyle=rapid`, `bow=Maple shortbow`, `ammo=Bronze arrow`, `ammoWithdraw=200`, `minStack=1`, `collectRange=12`, `solveClues=false`, `bankStrategy=Off` |
| `green_dragon_special` | GreenDragon | Attack 60, Strength 40, Hitpoints 40, Lobster 12, worn Dragon dagger 1215 and Dragonfire shield 1540, and the worn dagger's 250-cost special covered by the transmitted `%sa_energy` before Start | `combatStyle=melee`, `meleeStyle=strength`, `useSpecial=true`, `usePotions=false`, `escape=Flee to bank`, `solveClues=false`, `buryBones=false`, `weapon=Dragon dagger`, `shield=Dragonfire shield` |
| `green_dragon_potions` | GreenDragon | Attack/Strength/Hitpoints 40, Lobster 12, Super attack(3) 145 and Super strength(3) 157 with no two-dose flask and no live boost acknowledged, worn Rune scimitar 1333 and Dragonfire shield 1540 | `combatStyle=melee`, `meleeStyle=strength`, `useSpecial=false`, `usePotions=true`, `escape=Flee to bank`, `solveClues=false`, `buryBones=false`, `weapon=Rune scimitar`, `shield=Dragonfire shield` |

Preparation, acknowledgement and the teleports into the Guard area, the crab
field and the wilderness field all happen before the frozen script Starts; every
post-Start step is a pure witness. No loot, guard, RNG, pickup or energy state is
seeded as a result. The 180-second script deadline and the existing 150-tick
watch budgets are unchanged.

`crates/scenario/src/lib.rs` gained one shared ranged builder
(`combat_range_scenario`) for the two bow branches; the two GreenDragon cases are
built from the existing prepared `combat_core_scenario` plan plus their own
pre-Start acknowledgement steps.

## Qualification

The shared `CombatCoreCycle` in `crates/host-play/src/catalog_core.rs` now
carries the branch-specific transitions beside the existing Strength and Fire
Strike witnesses:

- **Ranged** (`auto_fighter_range`, `rock_crab_range`): Ranged XP above the
  pre-Start baseline, `com_mode` varp 43 observed at rapid (1), and a worn
  projectile stack that actually falls below its observed peak. Measured Ranged
  XP alone is not the branch, which is why the two extra transitions are
  required.
- **Special** (`green_dragon_special`): `%sa_attack` varp 301 observed armed and
  `%sa_energy` varp 300 falling by at least the wielded Dragon dagger's 250 cost
  from either the pre-Start pool or the highest pool seen since. A queued bar
  that never pays cannot qualify.
- **Potions** (`green_dragon_potions`): the three-dose flask becoming its
  two-dose form while in combat, followed by the native attack or strength boost
  landing. A dose without the boost, or a boost without the dose, cannot
  qualify.

`combat_spec` gained `projectile` and `consumable` fields and the matching
baseline gates, so a wrong preparation is refused before Start: a melee loadout,
a bagged bow, missing arrows, low Ranged, absent dormant Rocks, an already-armed
special bar, a seeded two-dose flask, a pre-existing boost or a shield-only kit
all fail `validate_case_baseline`.

Baseline and cycle regressions live in
`crates/host-play/tests/catalog_boundary_live.rs`
(`ranged_branches_need_the_sources_own_mode_and_projectile_spend`,
`special_branch_needs_an_armed_bar_and_a_paid_pool`,
`potion_branch_needs_a_dose_and_its_native_boost`). Each named false positive is
held in *every* observed frame, because the witness only accumulates positive
transitions — a later frame that restores the transition would otherwise mask a
real false positive.

A fourth regression,
`option_branch_injects_match_both_frozen_card_schemas`, parses each card's own
`SETTINGS` schema out of the SHA-verified frozen source plus the keys the card
reads through `this.settings.<kind>`, and refuses an injected id the card does
not read or a value outside the card's declared type, option list and numeric
bounds. It covers both catalogs and both client revisions.

## Verification and limits

Implementation commit `c5102901b37f836e4c41681461abc7941c4c93fb` on
`codex/rs2b0t-multirevision`, client gitlink
`52c37f9ce50d1f184656d5b4469c007ec8a5791a`. Checks ran in the exact isolated
export `.superpowers/review-exports/t_30bc0807-r1b` (a `git archive` of the
commit, with `.superpowers/inputs` and `vendor/fr-client-rust` symlinked to the
canonical frozen inputs and submodule checkout, and an exclusive target
directory): scenario 108 passed, host-play library 177 passed,
`catalog_boundary_live` 53 passed with its one LIVE test ignored, strict Clippy
on `scenario` and `host-play` all targets clean, `cargo fmt --all --check` clean.
The raw log is `evidence/options-147/isolated-checks.log`; the verbose log stays
in the export.

Unresolved constraints, reported rather than worked around:

1. **No LIVE.** The four catalog x revision cells are root-owned and unproven
   here. Two of them depend on engine-side behaviour that cannot be observed
   from this checkout: the `%sa_energy` pool the account actually logs in with
   (the special cell's pre-Start acknowledgement fails loudly if a fresh
   account's pool is below the dagger's 250, instead of passing unarmed), and the
   server applying the super-potion boost and consuming the arrow that was fired.
2. **Both frozen cards are byte-identical across the two catalogs** (AutoFighter
   `d82972d0…`, RockCrab `7e12b775…`, GreenDragon `46b667c4…`), so these four
   cells currently differ only by client revision, not by catalog.
3. **`bankStrategy` is read but not schema-declared.** RockCrab reads
   `this.settings.str('bankStrategy', 'Off')` at RockCrab.ts:238 without listing
   it in `SETTINGS`; the range cell injects that same default so the bank branch
   stays out of this cell. The regression accepts schema-declared keys and
   accessor-read keys, and documents the gap.
4. **The support matrix does not yet list these four cells.** The LIVE harness
   resolves its card row by card name, so the cells are dispatchable
   (`CATALOG_SCENARIO=auto_fighter_range|rock_crab_range|green_dragon_special|green_dragon_potions`),
   but `docs/compat/support-matrix.json` carries no per-case status or proof ref
   for them; matrix refresh is root-owned and this task did not touch it.
5. An iterated dose form other than the three-dose flask (Super attack(2)/(1),
   Super strength(2)/(1)) is out of this cell: the audit scope is the card's own
   default plan of one three-dose flask per potion.
