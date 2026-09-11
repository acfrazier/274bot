# Prepare paired catalog trade and duel functional witnesses

Use grok46 defaults. Independent fixture work on NEW unique paths only:
crates/host-play/tests/paired_catalog_live.rs,
crates/host-play/tests/support/paired_catalog.rs,
docs/compat/05r-paired-catalog-fixtures.md and unique evidence/paired-catalog-fixtures/.
Do not edit shared scenario/lib.rs, catalog_boundary_live.rs or existing
support/two_slot_isolation.rs. Resource then station fixtures own those paths.
No API/runtime/shim/client/nav/frontend/foreign/engine changes and no LIVE.

Use the existing shared production Play two-slot construction and selected
immutable profile/template from two_slot_isolation_live.rs as the bootstrap
reference. Keep actors distinct and owned by the fixture; no operator accounts.
Load both actual frozen catalog sources via the real registry and controller.
Do not replace either script's core loop with harness interactions. This is
paired-scenario harness work, not a new gameplay runtime or a refactor of the
existing isolation proof. Preserve the existing isolation harness intact.

Prepare two bounded cases, both revisions and catalogs selected by env:
1. NatureCrafter supported Air variant, actual Master plus Runner with exact
   partner names. Observe real trade requests, both offer/confirmation phases,
   matched counterpart identity, unnoted essence1436 moving from Runner to
   Master, Master positive RunecraftXP plus exact Air556, Runner actual bank
   restock/return and another transfer/craft. No seeded rune/XP result and no
   transfer from an unrelated actor may qualify. Pre-Start acknowledged initial
   supplies and legitimate starting tiles are allowed; if starting beside the
   altar to bound the first setup leg, explicitly distinguish seeded first
   supplies from the subsequent script-caused bank withdrawal/return/transfer.
   Keep Nature island/unnoting/boat branches separate, not accepted by Air.
2. Duel Arena Combat Trainer, two actual scripts: request/accept a duel with
   matching peer, enter actual combat, positive melee XP caused by hits, real
   duel end/reset and further script-caused challenge/combat if the existing
   gold window permits. A seeded modal, queued Challenge, player-facing flag
   or readiness is not a duel. Do not seed interface state or award XP. Verify
   selected generated duel controls, legal arena/starting tiles, weapons and
   level gates from native data. Keep unavoidable randomness in honest limits.

Before writing fixture loops, audit actual calls against current committed
native mappings and report any blocker by exact operation/source/call shape.
Named-bank metadata t_bced5c76, special t_68de6f48, teleport t_1bf9a22e and shop
 t_1591d140 are separate runtime owners. Do not implement them here or dim
foreign cards. LIVE must wait for any needed runtime reviews; root decides.
If a claimed full phase cannot fit unchanged existing gold bounds, preserve
that precise limitation and provide an explicit partial witness, never fake
completed cycles or enlarge script timeouts. A pair fixture can share one
observation window after both actors are qualified ingame/scene2.

Reuse existing local fixture seed mechanisms; acknowledge every mutation before
both baselines and Start. No post-Start cheats, save-file writes, invented
server endpoints or JS policy clones. Record immutable source/profile/client/
content identities, baseline/latest and ordered witnesses per actor. Emit FAIL
+exit1 with accumulated observations on failure. Print PASS only when the named
witness is satisfied, then cleanly stop owned actors using existing controls.

Meaningful local witness tests reject wrong partner, one-sided confirmation,
seed-only inventory/XP, stale trade/duel state, transfers from wrong actor,
missing conservation, no actual bank restock, no real duel combat, and no
further work for full-cycle claims. Exact Git host/client export plus only
owned files, independent empty target, focused host-play test and strict
Clippy. Check disk and leave cache cleanup to root. Commit scoped new files,
request SAME-card reviewer with precise supported/blocked limits and identities,
then STOP. Root performs headless/native acceptance and all ledger updates.
