# Repair combat witness generations and bounded CoalTrucks preparation

Active campaign codex/rs2b0t-multirevision. Read AGENTS.md and docs/execution.md.
After135, before132 in the fixture lane. Own only relevant scenario/lib.rs
preparation and host-play/tests/catalog_boundary_live.rs observations/tests.
No runtime, API/client, foreign script or LIVE changes. Same-card reviewer.

Current214 completed logs are in catalog-harness/live/*-21446414.{json,log}.
Read failed AutoFighter274/289 and ChaosDruid274 alongside passing ChaosDruid289
and MossGiant controls. AutoFighter shows real attacks/Strength XP and thousands
of counted engagements but zero defeats; CombatCoreCycle sets defeated=true
for any initial health0 and carries prev.defeated forever. A positive-health
spawn then increments engagements on every repeated snapshot because new_spawn
remains true. record_defeat refuses that index forever. This is an observation
state bug, not evidence that thousands of fights occurred.

Correct the bounded per-NPC spawn/engagement/death state: initial unknown zero
health must not mark an unengaged NPC dead; use total_health/known positive health
and actual selected/local combat evidence. Clear prior death exactly when a
credible new spawn appears, count an engagement once per life, require actual
post-Start selected health loss/death (or separately supported disappearance /
matching fresh drop), and require fresh further work after that death. Do not
let stale global looted=true qualify disappearance of an unrelated NPC, nor
count other actors' combat as the script's. Preserve full XP/target/loot/extra
requirements and bounded observation memory. Add meaningful repeat-frame,
initial-zero, respawn, selected-target, stale-loot and further-work tests.

AutoFighter currently holds an unworn Adamant scimitar; inspect its actual foreign
policy and fixture before deciding whether preparation must equip it. If needed,
prepare and acknowledge equipment before Start using existing native fixture
operations; do not implement foreign loadout policy in JS. Do not weaken Guard
kill/further-combat requirements or inflate combat stats/deadlines as a shortcut.

CoalTrucks30/Steel pickaxe with27 free slots makes only a few coal before the
existing150-tick pack-fill watch; both214 cells fail at arrival-to-truck. Use the
same honest nonproduct ballast preparation pattern as resource125 to require a
small actual mined full pack, a real mine-truck deposit, and further mining.
No seeded Coal/noted Coal/truck contents, no post-Start cheat, no enlarged clocks,
no accelerated server. Keep Mining30, combat-safe48 stats, tool and settings.
Baseline must acknowledge exact nonproduct ballast/available slots, zero
product, tool and skills before Start. If foreign policy cannot complete that
cycle with ballast, report the fact instead of substituting a fake witness.
Preserve existing declared coal core scope; do not claim full Seers haul unless
that complete sequence actually runs and has its own witnesses.

Use an exact committed export with owned overlay; focused scenario/catalog
regression suites, formatting and strict affected Clippy. Report05zc-combat-coal-
qualification-repairs.md (one filename) and evidence/combat-coal-qualification.
Retain old PASS/FAIL artifacts; root requalifies affected cells after review.
Commit owned files, request same-card reviewer, then stop.

Completed214 addendum (14:45 UTC): include two concrete combat preparation
repairs in this same fixture scope. HillGiant starts inside its cave without
Brass key, so the frozen script immediately goes to fetch it and never reaches
the combat watch. Prepare/acknowledge the real key before Start for this declared
inside-cave combat case; do not claim key-fetch/entrance coverage from it.
GreenDragon274/289 baselines are already under attack with empty equipment and
held shield, HP25/22, then die to dragonfire before gearing. Prepare/acknowledge
its protective shield using native fixture equip while still on the safe tile,
then enter the hostile field; do not increase stats or suppress dragon attacks.
Retain real post-Start worn-shield and dragon combat/drop/further-work evidence.
Explicitly record that pre-equipped shield does not qualify the foreign
GearEquip branch; that branch remains separately unqualified, not silently PASS.
Keep current weapon semantics unless separately justified by native readiness.
FireGiant's missing Quests transport is native137, outside this fixture scope;
existing Waterfall Quest preparation must remain truthful and acknowledged.
ArdyFighter slow-tick termination is covered by134's cake projection correction;
do not substitute seeded cakes or remove its real stolen-food requirement.
