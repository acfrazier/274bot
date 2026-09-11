# Thin native mappings for three combat startup helpers

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 11:35 UTC. Kind: bounded read-only design for brief 112.
Not implementation, compilation, LIVE, fixtures, ledger, STATE, cache
cleanup, or a foreign combat/restock/planner copy. Root owns
acceptance, any LIVE snapshot, serialized implementation insertion, and
implementation handoff. No routine reviewer card. Do not spawn
implementation cards from this run.

Read once: `AGENTS.md`, `docs/execution.md`, brief 112. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `f8a3f4a5c0d5c76f3e6968ac7bc95c0e7948b1f2`. Client gitlink
`9d090ed04957e4efc254f073cda97bc5510ca72b`. LIVE cells were recorded
on frozen host `04d7b1f48180ab9b93bbd6fec45a655df4385233` / same
client. Concurrent special / cook / paint / teleport / shop / Make-X
and remaining-fighter fixture WIP must not be touched. Work was
read-only except this report and
`docs/compat/evidence/combat-start-helper-design/`.

## Verdict

**Root04d actual289 (and 274) old-catalog FAILs are missing host
mappings, not broken imported scripts.** MossGiant melee `onStart`
always calls `rangeLoadoutOf(WEAPON, AMMO)` even when `WEAPON` is
`''`. HillGiant first-tick `foodInPack()` calls `foodCount`.
AutoFighter `onStart` calls `SettingsStore.displayString` for the raw
`combatStyle` string (legacy split; not `this.settings.str`).

Map three thin projections onto facts the isolate already has:

1. `rangeLoadoutOf(weapon, ammo)` — dart vs bow shape from posted
   `content.items` aliases ending `_dart`. Empty weapon stays empty.
   Do not read inventory or worn gear. Do not fill `DARTS`/`BOWS`.
2. `foodForms` / `isFoodItem` / `foodCount` — slot name/shape over
   posted inventory plus generated aliases. Exact ci match always.
   Cake family from `cake` / `partial_cake` / `cake_slice` only.
   Do not copy `FOOD_FORMS` / `FOOD_HEAL` / `DEFAULT_FOOD_HEAL=8`.
   Do not invent eat/bank policy. `foodHealAmount` /
   `shouldEatToUseFood` already exist.
3. `SettingsStore.displayString` and `saved` — stringify the posted
   `settingsBag` value, or schema fallback for display only. Never
   coerce options. Never replace a supplied value with a default.
   `resolve` stays bag-or-null.

| | Hypothesis | Status |
|---|---|---|
| (1) | Imported fighters are broken; dim the cards | **Rejected.** Frozen onStart/first-tick callers. Shim throws / missing member. Missing host support. |
| (2) | Fabricate Maple shortbow / Trout / melee when empty | **Rejected.** Truthful empty/inapplicable. No implicit inventory fallback. |
| (3) | Copy foreign DARTS / FOOD_FORMS / FOOD_HEAL tables | **Rejected.** No foreign ranged-weapon or food database. |
| (4) | Recreate health/banking/restock policy in food helpers | **Rejected.** `shouldEatToUseFood` already exists. `eatAtHpThreshold` stays `not impl`. |
| (5) | `displayString` returns schema default even when bag has a value | **Rejected.** AutoFighter needs raw `'defence'` not coerced `'melee'`. |
| (6) | Thin projections over posted items/inv/settingsBag | **Accepted mapping.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `f8a3f4a5c0d5c76f3e6968ac7bc95c0e7948b1f2` |
| LIVE host (frozen Root04d) | `04d7b1f48180ab9b93bbd6fec45a655df4385233` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 112 SHA-256 | `c87c81589e718a7bb051f0c2337305501e4cfcdfab1035164c946da0a1d852ff` |
| Kanban card | `t_f95493de` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| MossGiant.ts (both; sha256 equal) | `202ee1216b46a1c5eca0206772b4b3c78c9baf1205aad36f360e677efc0d22b5` |
| HillGiant.ts (both; sha256 equal) | `615436b5fd4b3a727c6593d3e18a86e592b5fc160164b1ca823d9eb1d4c70c00` |
| AutoFighter.ts (both; sha256 equal) | `d82972d038849d4a7f897a264ead850b4de218239d54eab16007a4e0c12266c2` |
| ranged.ts / food.ts / Settings.ts (both; sha256 equal) | `7f5c54ea…cbc0` / `58a35ca0…3127` / `960737c8…10de` |
| Native `ranged.js` | `2da809c4e92ed607eee68048c6f65f8a38f941f6eef0210509968b28869fdd81` |
| Native `food.js` | `4e9de0e3c041cd3658dbe858f87b0d368698b596a3db96d63338aaaf5fab6827` |
| Native `settings.js` | `dde9428e64be986d92efe2b532f226f488d15fd06db7b16bfa5198dd1bb11517` |
| Native `combat_equipment.js` | `8b98c49cb133e44005a890651af7ecc71194e25de8a33266cf504ee7af63dec5` |
| r289 moss / hill / auto logs | `040290f3…d58f` / `44e19ffa…888a` / `bd918a47…22a` |

Machine-readable copies: `evidence/combat-start-helper-design/{refs,hypotheses}.json`.

## 1. Actual FAILs

Frozen binary `catalog-boundary-04d7b1f4`, catalog `100adccc`, both
revisions. Preparation completed (`ingame && scene_state==2`); Start
then first script tick:

| Cell | Tick | Error | Prepared pack |
|---|---|---|---|
| r289 moss_giant | 34 | `not impl: ranged.rangeLoadoutOf` | Adamant scimitar + 10 Lobster at 2553,3406,0 |
| r274 moss_giant | 36 | same | same shape |
| r289 hill_giant | 37 | `not impl: foodCount` | scimitar + 8 Trout in the pit; log line still printed |
| r274 hill_giant | 38 | same | same shape |
| r289 auto_fighter | 36 | `SettingsStore.displayString is not a function` | scimitar + 8 Trout, Guard, combatStyle melee |
| r274 auto_fighter | 34 | same | same shape |

ChaosDruid preparation and Duel combat stay out of this hop.

MossGiant melee still hits the helper: `WEAPON` is `''` for melee, then
`const loadout = rangeLoadout()` at onStart line 692. That is required
behavior, not a reason to skip the call or invent a bow.

HillGiant `foodName = scriptFood(this.settings, 'Trout')` already
resolves through `__rs2b0t_food_of` (LIVE: Trout). `foodCount` is only
the slot counter. AutoFighter then calls `SettingsStore.saved` on the
next line; include `saved` in this hop or the cell dies on the next
missing member. Default LIVE injects `combatStyle=melee` /
`meleeStyle=strength`, so `SettingsStore.save` (legacy rewrite) is
not reached. Leave `save` / `globalBag` `not impl`.

## 2. What source does now

Posted isolate facts (do not add a native callback or world copy):

- `content.items`: `{obj: alias, id, name, cost}` from selected
  `GameItem`. Includes `bronze_dart` (id 806, stackable, wearpos 3)
  and cake aliases `cake` / `partial_cake` / `cake_slice`.
- `content.food_heals`: `[display_name, base]` for
  `qualification==fixed_hp_heal` only. Unknown names already throw
  `foodHealAmount`.
- Snapshot `inv[]` rows with `name` / `count` / `id` / `slot`.
- `settingsBag`: host `merge_bag` (schema defaults, then overrides,
  then inject). Isolate tests may post a partial bag.
- Loadout: `selectedLoadout` / `foodOf` / `scriptFood` already map
  user-selected carry through generated heals. Blank loadout keeps
  the script fallback (`Lobster` / `Trout`), not pack contents.

Shim gaps:

- `ranged.rangeLoadoutOf` throws. `DARTS`/`BOWS` are empty SETTINGS
  stubs on purpose (`combat_equipment.js`). Do not fill them here.
- `foodCount` / `foodForms` / `isFoodItem` throw.
  `eatAtHpThreshold` throws (keep). `shouldEatToUseFood` /
  `shouldEatFood` already exist as posted-opts rules.
- `SettingsStore` has `resolve` (bag value or `null`, never schema
  default-as-set; `isolate_settings_store_does_not_return_schema_defaults`)
  and throwing `globalBag`. No `displayString` / `saved` / `save`.

`eat_predicates` currently locks `foodForms('Shark')` as `not impl`.
Implementation must flip that lock: identity `['shark']`. Keep
unknown `foodHealAmount` and `eatAtHpThreshold` as `not impl`.

## 3. Accepted mapping

### rangeLoadoutOf(weapon, ammo)

JS only, `crates/script/src/shim/ranged.js`.

```
wanted = trim(String(weapon)).toLowerCase()
dart = content.items row whose obj ends with '_dart' and name ci-equals wanted
return {
  weapon: dart ? dart.name : String(weapon ?? ''),
  projectile: dart ? dart.name : String(ammo ?? ''),
  thrown: dart !== undefined
}
```

- `rangeLoadoutOf('', 'Iron arrow')` →
  `{weapon:'', projectile:'Iron arrow', thrown:false}`. Not Maple
  shortbow. Not the worn scimitar.
- `rangeLoadoutOf('Bronze dart', 'Iron arrow')` → thrown, both
  fields the generated display name, ammo ignored.
- `bronze_dart_p` / `bronze_dart_tip` do not end `_dart`; not thrown.
- `rockCrabRangeLoadout` stays a one-line alias. `rangeSupplyEmpty`
  stays `not impl` (melee does not call it).

### foodForms / isFoodItem / foodCount

JS only, `crates/script/src/shim/food.js`. Count **slots**, not
`row.count` (foreign `items.filter(...).length`; Trout is
unstackable).

`foodForms(foodName)`:

1. `key = String(foodName).trim().toLowerCase()`. Unknown → `[key]`
   (never throw; never heal-8).
2. Resolve `key` to a `content.items` display name (ci). Read `obj`.
3. If `obj` is `partial_*`, `half_*`, `half_a_*`, `half_an_*`, or
   `*_slice`, or starts `cert_`: identity `[key]`.
4. Else family aliases exactly `{obj}`, `partial_{obj}`, `{obj}_slice`.
   Return unique lowercase display names. Cake →
   `cake`, `2/3 cake`, `slice of cake`.

Do not add `half_plain_pizza` / `half_a_meat_pie` / `half_an_apple_pie`
/ `chocolate_slice` grouping. Those aliases are irregular English, not
one stem rule. Copying `FOOD_FORMS` to recover them is forbidden.
Pizza/pie/chocolate-slice stay identity until a generated family field
exists. HillGiant Trout does not need them.

`isFoodItem(name, foodName)` is `foodForms(foodName).includes((name??'').toLowerCase())`.
`foodCount(items, foodName)` counts slots passing `isFoodItem`.
Missing/non-array `items` → 0. Asking for Trout in a Lobster pack → 0.

Leave `foodHealAmount`, `MIN_EAT_HP`, `shouldEatToUseFood`,
`shouldEatFood`, `FOOD_OPTIONS` unchanged. Do not implement
`eatAtHpThreshold`. Do not restock.

### SettingsStore.displayString / saved

JS only, `crates/script/src/shim/settings.js`. One posted bag (running
script). Ignore `name` rather than inventing per-script browser
storage. Do not change `merge_bag` or `post_settings_bag`.

Stringify with the foreign `settingToString` shape: boolean
`true`/`false`; `string[]` joined `', '`; tile `{x,z,level}` as
`x,z,level`; else `String(value)`.

- `displayString(_name, key, def)`: if `settingsBag` has `key`,
  stringify that value (including `''` and `'defence'`). Else
  stringify `def.default` (or `''` if no def). Never coerce options.
- `saved(_name, key)`: stringify if the bag has `key`, else
  `undefined`. Partial isolate bags can omit schema defaults; host
  LIVE bags are merged, so LIVE `saved('meleeStyle')` is `'strength'`.
  That is host posting, not this mapping inventing a default.
- Do not change `resolve`. Do not implement `save` / `globalBag`.

AutoFighter: `displayString(..., 'combatStyle', SETTINGS.combatStyle)`
then `saved(..., 'meleeStyle')` then existing
`resolveSplitCombatSettings`. Default LIVE does not call `save`.

## 4. Implementation handoff

One card. Own only:

1. `crates/script/src/shim/ranged.js`
2. `crates/script/src/shim/food.js`
3. `crates/script/src/shim/settings.js`
4. New `crates/script/tests/combat_start_helpers.rs`
5. The `foodForms` assertion inside existing `eat_predicates`
   (`crates/script/tests/load_isolate.rs`) — hotspot, that test only

Do not edit `load.rs`, `shim/mod.rs`, `combat_equipment.js`,
`game_data.rs`, `settings_store.rs`, `catalog_boundary_live.rs`,
`scenario`, special/teleport/shop/Make-X/cook/paint files, catalogs,
or STATE.

Focused isolate checks (behavior, not source snapshots):

1. Empty `rangeLoadoutOf('', 'Iron arrow')` as above.
2. Bow `'Maple shortbow'+'Iron arrow'` → not thrown; ammo kept.
3. `'Bronze dart'+'Iron arrow'` → thrown; both fields Bronze dart.
4. Posted worn Maple + inv Bronze dart must not fill an empty weapon
   (no implicit inventory/equipment fallback).
5. `foodCount` of eight Trout rows is 8; empty inv is 0; Trout query
   over Lobster rows is 0; do not sum `count`.
6. `foodForms('Cake')` includes the three generated cake names;
   `foodForms('2/3 cake')` is identity `['2/3 cake']`;
   `foodForms('Shark')` is `['shark']`.
7. Unknown food: `foodForms` identity, `foodCount` 0,
   `foodHealAmount` still `not impl`.
8. `displayString` of bag `combatStyle='defence'` is `'defence'`, not
   `'melee'`. Missing key uses schema default string.
9. `saved` missing → `undefined`; present → raw string.
10. `resolve` still must not return schema defaults as if set.

No compiler / LIVE on the implementation card unless root says so.
Source review of a later patch is not moss/hill/auto LIVE acceptance.

## 5. Unsupported flavors (honest)

Leave `not impl`: `rangeSupplyEmpty`, `eatAtHpThreshold`,
`SettingsStore.save`, `SettingsStore.globalBag`, URL/session storage,
per-script bags other than the posted one, pizza/pie/chocolate-slice
family grouping, filling `DARTS`/`BOWS`/`STAFFS` option lists,
restock/panic/bank policy, ChaosDruid, Duel.

Do not walk, eat, or wield because a helper returned a name. Do not
treat an empty range loadout as success-with-bow.

## 6. Falsifiers

A later implementation is wrong if any of these hold:

- Any of the three LIVE throws remains, or is hidden by dimming.
- Empty weapon becomes Maple shortbow / worn scimitar / first dart.
- `foodCount` sums stack size, counts the wrong food, or returns a
  fabricated 1.
- Unknown food heals 8 or `foodForms` throws.
- `displayString` coerces `'defence'` to `'melee'` or overwrites a
  supplied bag value with `def.default`.
- `resolve` starts returning schema defaults as set.
- Foreign `FOOD_FORMS` / `DARTS` / planner / restock copied.
- `load.rs` / `mod.rs` / special / cook / paint / fixtures edited
  on this hop.
- World snapshot deep-copied for these helpers.

Root inserts implementation after this design. Not LIVE acceptance.
