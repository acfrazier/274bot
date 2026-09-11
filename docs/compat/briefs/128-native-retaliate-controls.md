# Brief 128: expose native retaliate control identity

Campaign checkout/branch: current rs2b0t-multirevision / codex/rs2b0t-multirevision.
Read AGENTS.md and docs/execution.md once; apply fail-closed-dispatch.
Wait for native sequencing123 review. Root owns LIVE and evidence acceptance.

Exact cd49e147/aef3952d WildyAgility LIVE stopped after Start in both274/289
at `not impl: reader.retaliateControls` (first catalog100adccc; counterpart
8e7d965b is also running). Raw logs are in catalog-harness/live/*wildy*cd49*.
Frozen script ensureRetaliateOff calls reader.retaliateControls then
`actions.ifButton(controls.offComId)`. Do not change either frozen catalog.

Native api::Snapshot/ReadApi already exposes retaliate_controls returning
Option<ToggleControlsView>; api::Interact::set_retaliate already owns selection.
Implement posted transport for the actual native pair and a thin
reader.retaliateControls shape {onComId,offComId}, or null when unavailable.
No foreign IfType scans, hardcoded component ids, fake sentinel buttons,
JavaScript controller/policy, guessed pair from an enabled boolean, or new
client action API. Preserve full/delta publication, missing/cleared state,
relogin/reset and per-slot isolation. Inspect existing analogous schema seams
before choosing a minimal additive representation. Host Rust owns data.

Own only the required api/script transport bridge, schema/host_js/native bindings,
client_adapter.js and focused uniquely named tests; existing library test module
changes only when necessary. Do not modify other shim modules, shared
load_isolate integration test, scenario/catalog fixtures, docs STATE/matrix/ledger
or client repository. Serialize runtime edits through this task's dependency;
teleport follows its review. Do not commit another active owner's changes.

Verify native pair/shape, absent and subsequent clear, both toggle identities,
no stale session/slot leakage and existing affected transport tests. Use an exact
committed isolated export plus owned overlay if other WIP exists. Reuse an
exclusive completed cache or task-owned cache; run affected tests and strict
Clippy once. Provide concise report05z and evidence/native-retaliate-controls.
This is API fidelity/transport work: same-card reviewer handoff, then stop.
Root will rerun actual Wildy full lap cells only after review approval.
