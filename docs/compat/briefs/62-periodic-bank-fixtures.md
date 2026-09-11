# Qualify an actual periodic bank trip

Use grok46 defaults on codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch, current STATE.md ownership policy and
04-capabilities-periodic-bank.md. PeriodicBank at 922dac24 passed actual same-card
Grok 4.5 corrective review 1211, after preserving the later Tile commit. Start
only after Superheater fixture review so shared scenario/harness files have one
owner. Do not spawn other agents.

Extend the existing scenario registry and catalog_boundary_live witness with
one meaningful ChickenKiller periodic-banking case for both frozen catalogs and
both selected revisions. Inspect the exact frozen ChickenKiller and Banking
settings first. Use a supported low loot threshold, melee and a bankable loot
item; do not fake post-Start loot or rewrite the imported script. Default core
ChickenKiller proof remains unchanged. The reviewed service keeps existing native
walk/bank timeouts, nearest packed booth and return radius 6. Choose a supported
nearby real target/anchor if the default pen cannot reach its selected bank under
those existing bounds; justify the selected content and branch from source,
without changing navigation or host behavior to accommodate the script.

Prepare only before Start with acknowledged seeds and exact selected-world
items/stats/tile. Capture attached ingame scene-2 baseline after preparation.
Require script-caused combat/loot before the trip, later exact inventory-to-bank
deposit against a fresh bank generation, return to the original anchor within
the service radius, and further combat XP or new exact loot after return.
Keep intermediate ordered snapshots; queueing walk/open/deposit is insufficient.
Verify tracked gear/kept supplies survive deposit if the chosen source uses
them. A distinct afterDeposit restock branch may be added only if it exercises
already reviewed capabilities; do not depend on pending autocast or other
future runtime work. Preserve existing scenario and SCRIPT_GOLD_DEADLINE bounds.

Allowed files: crates/scenario registry/modules; the existing
crates/host-play/tests/catalog_boundary_live.rs and meaningful focused fixture
tests; 05g-periodic-bank-fixtures.md and evidence/periodic-bank-fixtures/.
Keep panel and headless scenarios shared. No product/api/script/client/nav/
engine/catalog/ledger/frontend edits, no actual LIVE launch, no operator vault
or loadout edits. Report concrete capability or foreign-script defects to root
with source evidence; do not weaken the predicate or dim cards on this task.

Check exact committed export plus owned overlay in a NEW EMPTY Cargo target;
record source/client/overlay hashes, affected scenario/harness tests and strict
Clippy. Preserve concurrent changes. Commit only owned paths, request SAME-card
profile reviewer with evidence, then STOP. Root owns live runs and acceptance.
