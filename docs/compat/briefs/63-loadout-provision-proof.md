# Add an observed loadout provisioning proof

Use grok46 defaults on codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch, STATE.md and
04-capabilities-loadouts.md. Source 89767527 is independently reviewed;
native Save/Duplicate/quantity/search proof is already recorded in
evidence/loadout-ui/. What is missing is completed real bank provisioning.
Do not spawn other agents.

Prepare a bounded ignored LIVE integration test in a NEW standalone
crates/host-play/tests/loadout_provision_live.rs. Reuse existing production Play,
ScriptStartHandle, selected immutable profile/template, preparation helpers and
snapshot observation patterns from catalog_boundary_live/revision_boundary_live.
Do not edit shared scenario, API, script runtime, client, panel or TUI files.
Keep any small test-only preparation/script data in this test or a uniquely
named sibling fixture directory. Root owns actual LIVE launches.

The test checks the existing component contract, not an imported catalog row.
Use a short test-authored compat script with selectedLoadout, weaponOf,
suppliesOf, Bank.ready/withdrawX and Equipment.equip through production dispatch.
Adapt the existing meaningful isolate composition in script/tests/loadouts_bag
into an observed sequence: wait fresh bank, withdraw configured quantity and
weapon, observe inventory, equip, observe actual selected slot. No invented
planner, queued-success substitute, direct client gameplay after Start, snapshot
injection or hidden operator-state changes. Do not copy foreign runtime/policy.

Use a disposable local actor and PRIVATE temporary home/loadout/vault storage
with normal APIs; never read/write operator loadouts or credentials. Resolve
274/289 with explicit selected local ports, cache and nav identity. Require
LIVE=1, fail plus exit 1, and ingame && scene_state==2 before Start. Prepare
selected-data valid weapon skill requirements and acknowledged bank contents
only before Start. Choose a non-default supply quantity such as 17 to exercise
Withdraw-X. Baseline must lack the configured weapon and supply in inventory
and equipment, with sufficient bank stock. Post/select a preset containing an
explicit righthand slot and that exact carry quantity.

Require later fresh bank generation, exact inventory count and bank decrement,
actual equipped weapon ID/slot and preservation of exact supplies after equip;
record ordered snapshots and IDs. The test must not report success from an
empty/stale bank, a queued request or seeded equipment. Preserve existing host
timeouts, asynchronous acknowledgement and cleanup semantics. Evidence should
separate build/fixture checks from actual gameplay, and component proof from
full catalog/frontend acceptance.

Allowed scope: this standalone test and uniquely named supporting fixtures,
05h-loadout-provision-proof.md, evidence/loadout-provision/. Report production
defects to root instead of patching outside scope. Check exact committed host
plus owned overlay/client export in a NEW EMPTY target; record hashes and
affected no-LIVE fixture tests/strict Clippy. Do not add tests that merely
mirror implementation. Commit scoped paths, request SAME-card reviewer with
evidence, then STOP. No actual LIVE or engine mutation on this task.
