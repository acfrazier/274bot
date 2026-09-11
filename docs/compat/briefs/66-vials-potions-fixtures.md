# Qualify banked vial filling and potion production

Use grok46 defaults on codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch, current STATE.md and the reviewed
inventory useOn identity report. Do not spawn other agents. Start after the
PeriodicBank fixture review to serialize shared scenario/catalog harness files.
Root has completed the disjoint Superheater Attack-30 correction at 6c7075ce;
preserve it and all other committed source.

Add shared scenario/catalog_boundary_live fixtures for two actual frozen cards,
VialFiller and PotionMaker, on both catalogs and both revisions. Inspect each
frozen source/sibling/settings first, plus the selected generated data and local
content definitions for item IDs, levels, quest requirements and locs. No
foreign-source or runtime changes. Report missing mapped host capabilities to
root with the exact caller and Rust seam, instead of accommodating a foreign bug.

VialFiller: core buyVials=false, Falador West bank to real fountain; also cover
the supported Falador East path as a distinct bank setting when within existing
bounds. Empty pack baseline, acknowledged banked empty vials and no water vials.
Require actual empty-vial to water-vial ID transition at the fountain after
Start, a later fresh bank deposit of those script-produced vials, restock of
empty vials, return to fountain and at least one further fill. No XP is granted
for filling, so world/item transitions are the witness. Preserve per-use protected
operation pacing from the frozen script. Shop-buy option awaits the already
queued shop capability and must be recorded pending, not silently accepted.

PotionMaker: cover default Custom Guam leaf + Eye of newt and the explicit
named-selector branch supported by both catalogs. Use the smallest meaningful
second recipe only if it exercises a distinct supported selected-data behavior;
do not enumerate equivalent herb constants. Seed valid Herblore and quest state
before Start with acknowledgment; empty inventory, sufficient banked ingredients
for a real cycle, and no seeded unfinished/finished potions. Require exact herb +
water to unfinished potion, secondary consumption to finished potion, positive
Herblore XP, a fresh produced-potion bank deposit/restock and further product/XP.
The source intentionally queues useOn calls in a batch: preserve that policy;
no shim batching/controller rewrite or timeout change. Capture staged observations
so an unfinished or seeded result cannot count as completion.

Use attached ingame scene2 baselines, exact actor/catalog/script/profile identity,
ordered snapshots and existing bounded scenario/script deadlines. Setup actions
only before Start. Bank-loaded/generation and actual ID/count differences must
establish the cycle; queue sends, readiness, paint counters and normal exit alone
cannot pass. Private actors/stores only. Keep shared native/headless scenarios.

Own crates/scenario registry/modules, crates/host-play/tests/catalog_boundary_live.rs,
meaningful existing-family fixture tests, docs/compat/05i-vials-potions-fixtures.md
and evidence/vials-potions/. No client/api/shim/frontend/ledger edits and no LIVE
launch. Verify exact source export plus owned overlay in a NEW EMPTY target,
affected scenario/harness tests and strict Clippy. Pass each command its actual
working directory and CARGO_TARGET_DIR; a blocked export command is not applied.
Record verified source hashes and any failed/invalid build attempts. Commit only
owned paths, request SAME-card profile reviewer with evidence, then STOP.
