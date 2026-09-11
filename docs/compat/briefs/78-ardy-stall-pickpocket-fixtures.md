# Qualify Ardougne stall and pickpocket catalog loops

Use grok46 defaults after RuneCrafter/MuleCrafter fixture review releases
scenario/lib.rs and catalog_boundary_live.rs. Read current STATE ownership/stub
rules and sections9-10 of fixtures/remaining-combat-world.md. That dated fixture
design supplies leads; verify selected content and current operations. No LIVE
or subagents. Root owns actual gameplay, ledgers and frontend acceptance.

Implement coherent shared scenario/CoreWitness fixtures for ArdyCakes and
ArdyThiever on both frozen catalogs/revisions. ArdyCakes default Flee core must
actually steal cake/bread/pastry with ThievingXP, deposit the acquired stock in
a fresh bank, return to the stall and steal again. ArdyThiever Guard default
Flee must actually pickpocket coins with ThievingXP; add the distinct Knight
target branch. Banking/return is required when enabled; explicitly set and
record a bounded loot-count banking option rather than treating Off as a bank
proof. Fight is gated by the separately queued hostile-attacker capability and
stays pending, not silently skipped or labelled supported.

Set solveClues=false for these ordinary-work fixtures. The optional clue helper
remains an honest stub for a future native project and is not grounds to dim
these cards. Keep existing DeathRecovery/PeriodicBank behavior. Verify exact
NPC/loc/item/skill/control identities in each selected engine/cache before seeds.
Use legitimate pre-Start thieving/HP/food preparation; choose levels adequate
for a controlled bounded fixture, explicitly acknowledge every seed and set
XP/items baselines afterward. No post-Start cheats or claimed success from seed,
stun, queued sends, an empty bank window, or constructor readiness.

Inspect the actual scripts and choose only the minimal distinct cases needed
for stall, Guard and Knight plus supported banking/flee observations. A caught
steal and Flee displacement can be captured within the natural cycle; don't
invent a hostile fact or shorten a game timer. Preserve current scenario gold
deadlines and existing watch windows; do not stretch timeouts. If this makes a
required behavior unprovable, report that precise fixture/capability boundary
instead of weakening the witness. Disabled Fight isn't a Fight proof.

Own only these new cases/registry rows in crates/scenario/src/lib.rs, new
CoreCase/witness/meaningful tests in crates/host-play/tests/catalog_boundary_live.rs,
docs/compat/05m-ardy-thieving-fixtures.md and evidence/ardy-thieving-fixtures/.
Preserve every prior fixture, including root1694cec68's West tile expectation.
No runtime, shim, API, client, nav, generated-data, engine, frontend, ledger or
foreign-source changes. Missing host operations are BLOCKED findings with the
call and source evidence; root dispatches those separately.

Witness tests should reject seed-only deltas, wrong IDs/target, stale/closed bank,
deposit without fresh acquisition, no return and no subsequent stealing. Check
actual setup order and current script settings, not fabricated success payloads.
Use exact committed host/client bytes plus owned overlay in a fresh isolated
export and new empty target. Affected scenario/catalog tests and strict affected
Clippy, focused formatting. Commit only owned files, request SAME-card reviewer
with exact identity and checks, then STOP. Root performs LIVE after review.
