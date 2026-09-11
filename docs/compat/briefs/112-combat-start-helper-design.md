# Design thin native mappings for three combat startup helpers

Use grok46 defaults, read-only design. Own only
 docs/compat/04ae-combat-start-helper-design.md and unique
 evidence/combat-start-helper-design/. No code/builds/LIVE/STATE/ledger edits.
Commit scoped report, complete this design card and stop.

Root04d actual289 old-catalog cells fail at MossGiant ranged.rangeLoadoutOf,
HillGiant foodCount, AutoFighter SettingsStore.displayString. Raw records live
under evidence/catalog-harness/live/r289-{moss-giant,hill-giant,auto-fighter}-
100adccc-04d7b1f4.*. These are missing host mappings, not broken imported scripts.
ChaosDruid preparation and Duel combat failures are separate; don't expand into
those or generic combat policy.

Read both frozen callers and helper contracts, current shim/{ranged,food,
settings}.js, native posted loadout/food/item/settings facts and existing Rust
queries. Propose the smallest host-owned facts/projection glue, preserving
user-selected loadout and canonical setting keys. Melee initialization can read
a range-loadout helper even when range is not used. Distinguish a truthful
empty/inapplicable native value from fabricated success or a foreign fallback
loadout. Food forms/count/heal/shouldEat helpers must not recreate a foreign
health/banking policy; identify which actual native values/rules already exist,
which are mere name/shape mapping, and which stay explicitly unsupported.
SettingsStore display must reflect the actual selected settings bag or schema
fallback contract, not silently force defaults for a supplied value.

No copied foreign ranged weapon/food databases, planner, controller, traversal,
restock or ancillary helper implementation. User permits ancillary stubs and
preserves host behavior. Identify exact missing native facts and bounded tests
for selected settings, no implicit inventory fallback, food multi-form/count,
unknown inputs and explicit empty range loadout. Propose one coherent narrow
implementation handoff with file ownership avoiding the active special/teleport/
shop/Make-X/cook work. Root owns implementation scheduling and LIVE.
