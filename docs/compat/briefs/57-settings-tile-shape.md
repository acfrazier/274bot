# Preserve Tile behavior through posted settings

Use configured sol defaults. Work on codex/rs2b0t-multirevision in the campaign
checkout. Read applicable instructions, docs/execution.md and fail-closed-dispatch.
This is a bounded shape correction and native geometry connection.

Operator clarification at 02:50 UTC: both distanceTo and walkable belong to
host-owned geometry/navigation. Tile remains a small JavaScript value wrapper;
distanceTo must call Rust. Coordinate walkability/reachability is being designed
separately in t_d49b087b and is outside this card's implementation.

Root's isolated host 50f2be8a/client 56d8027 DoorOpener cell on local 274 and
catalog 100adccc failed at the first task tick with `stand.distanceTo is not a
function`. Preserve `evidence/catalog-harness/live/r274-door-opener-100adccc-50f2be8a.*`.
The frozen DoorOpener onStart calls `this.settings.tile('stand', DEFAULT_STAND)`;
the settings bag contains the valid posted coordinates `{x:3208,z:3212,level:0}`.
PRELUDE in crates/script/src/shim/mod.rs returns that raw object. The existing
Tile class in shim/tile.js provides distanceTo/translate/equals/toString. Foreign
SettingsBag.tile expects a Tile. This is our lost compatibility shape, not an
established foreign-script bug; do not dim or edit the imported card.

Map valid posted tile coordinates to the existing compatible Tile shape so
settings-derived positions support the same methods/instance identity as the
Tile import and Game.tile. Preserve coordinates and level, current fallback
semantics for absent/invalid data, list/scalar behavior and current posting.
Inspect both frozen callers and native settings-store normalization before
choosing the narrow seam. If the explicitly imported shim SettingsBag has the
same missing tile member for an enabled caller, use the same constructor/helper
there; avoid another independent math/class implementation. Replace the existing
JavaScript distance arithmetic with a synchronous call to a Rust geometry helper,
using the existing host distance primitive where appropriate. Inspect
crates/nav/src/tile.rs::chebyshev and crates/api/src/query.rs::chebyshev_to:
the former deliberately ignores plane; the latter returns i32::MAX across
planes. Do not change either host policy to accommodate imported scripts.
Preserve the existing Tile.distanceTo observable behavior, including its
cross-plane representation, through a small compatibility conversion in Rust
if needed. Native world coordinates and invalid input must be handled without
overflow/panic or silent reinterpretation. Do not copy the
foreign runtime or settings store, change navigation policy, clamp valid target
coordinates to the current scene, or implement any unrelated missing function.

Allowed files: shim/mod.rs PRELUDE settings shape, shim/tile.js if needed for a
single shared constructor, shim/settings.js only for this same contract, and
focused existing settings_bag/load_isolate tests; a small native geometry seam
in api or nav and minimal script dependency/lock changes if needed; a narrow
Rust script adapter and synchronous load.rs callback registration. PeriodicBank
t_51452e47 is concurrently editing load.rs: inspect current changes, keep the
geometry callback separate, and do not stage or overwrite its work. Report 04c-settings-tile-shape.md and
concise raw evidence in evidence/settings-tile-shape/. No catalog/scenario,
navigation engine policy, client, native UI, ledger, LIVE, remote or gitlink edits.

Regression proof must post a settings bag through the real isolate path, call
settings.tile, and exercise distanceTo/translate/equals or the actual bounded
DoorOpener task without the prior TypeError. Check nonzero level, fallback and
invalid input while preserving ordinary list/scalar settings. A JSON coordinate
round trip alone would miss this regression. Verify the Rust callback is used
and cover same-plane and cross-plane distances. Keep tests risk-proportionate.
Use a new empty Cargo target on an exact frozen export with owned overlay;
record host/client refs, overlay hashes and target. No shared campaign target.

Commit only owned source/report/evidence, request the SAME card's profile
reviewer with kanban_request_review, then STOP. Never self-complete. Root owns
new isolated LIVE proof and the whole-branch review. No more agents.
