# Qualify cooking, furnace smelting, and flax spinning bank cycles

Use grok46 defaults after resource fixtures t_aa061a28 complete SAME-card
review. Own only crates/scenario/src/lib.rs and catalog CoreCase/witness/tests
in crates/host-play/tests/catalog_boundary_live.rs, docs/compat/05q-station-
production-fixtures.md, and unique evidence/station-production-fixtures/.
No API/runtime/shim/client/nav/frontend/foreign/engine edits, no LIVE or ledger.

Implement coherent fixtures for CookBot Catherby Range (default Raw salmon
plus named Raw lobster), SmelterBot default Bronze plus supported Steel at its
real furnace, and FlaxSpinner's default bank/wheel loop. Verify both frozen
catalog sources before assuming settings/defaults and determine exact selected
274/289 input/product/range/furnace/wheel/stair/component IDs from native
content. Do not use foreign data tables as the authoritative source. Keep
CookBot Fire separate behind native fire work; Smithing main panel and
LeatherCrafter are not part of this fixture hop.

For each case require script-caused production with exact consumed input,
exact unnoted product and positive relevant XP after Start; actual deposit
of produced output into a fresh loaded bank; raw-input restock; closed-bank
return to the actual station; further production and XP. Baseline contains
no produced output. Adequate legitimate skill/tool/raw material seeds before
Start are allowed with acknowledgments, using existing fixture mechanisms.
Set skill high enough that ordinary cooking burn randomness does not replace
the product contract, documenting the fixture level. Never seed finished
product/XP gains or post-Start cheats. Preserve original deadlines/watch
windows and phase ordering. Do not let serial XP watch arm after the product
it must witness; use the reviewed Rune observation regression as guidance.

Audit every adapter operation on these core paths before handoff. Report
known missing native mappings precisely with source evidence and identify
existing runtime cards (Make-X t_4b04cb5f, fire t_79175534) when they gate LIVE.
Do not implement missing runtime operations here, invent IDs, weaken predicates,
extend timeouts or dim a foreign card. Root determines failure ownership and
runs only reviewed candidates with needed capabilities.

Meaningful witness tests reject seeded-only products/XP, name-only/wrong/noted
products, stale bank, no input consumption, no actual deposit/restock, wrong
level/station, or no further work after return. Preserve all prior fixtures,
especially Rune XP order/failure witness and Ardy/resource additions. All
frontends must continue to get shared scenarios through scenario::get/names.
No duplicated handwritten scripts or harness-side execution of the core.

Use exact host/client Git export plus owned overlay, independent empty target,
focused scenario/catalog tests, strict affected Clippy and fmt. Check disk
before builds; root owns compiler-cache cleanup. Commit only owned paths,
request SAME-card reviewer with exact identities/results and missing-op gates,
then STOP. Root owns native/headless acceptance and support accounting.
