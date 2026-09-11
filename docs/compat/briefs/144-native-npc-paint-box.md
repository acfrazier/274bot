# Project NPC outline corners through the native boundary

After bounded reachability140 review, implement the required reader.npcBox
capability for FireGiant paint. Campaign branch codex/rs2b0t-multirevision,
client branch codex/bothost-274-289, unchanged pin aef3952d at brief creation.
Use Sol defaults, fail-closed-dispatch and normal same-card reviewer handoff.
Own this bounded native observation/projection/transport/shim addition, tests,
report docs/compat/04aj-native-npc-paint-box.md and evidence/native-npc-paint-box/.
No LIVE, main, remote, STATE, support-matrix or ledger changes. No foreign
runtime/planner copy. Root owns source-frozen LIVE and acceptance.

Exactcff289 FireGiant failed at script tick164: `not impl: reader.npcBox`.
Receipt/log: docs/compat/evidence/catalog-harness/live/r289-fire-giant-100adccc-combatcff.*.
Frozen FireGiant.ts1267 calls reader.npcBox(targetIdx) from outlineTarget/onPaint.
ClientAdapter.ts549 contract: 8 screen/overlay-canvas corner points, ground ring
then top ring same winding; null for absent/off-scene/unprojectable NPC. This is
real optional geometry, not a successful no-op binding. Both frozen sources and
both revisions are required. Scope is NPC box only unless an inseparable API
primitive is needed; do not expand to locBox/playerBox or unrelated overlays.

Rust owns projection and snapshot lookup; JS must be a thin validated native
read. Reuse the bothost client's existing projection/math/geometry ownership:
crates/client/src/render/draw.rs project_overlay is a pure read used by native
nav-debug paint, currently pub(crate); get_overlay_pos wraps it. Determine the
smallest reusable native API needed. Do not clone foreign TypeScript projection
or mutate renderer state for a read. Client changes, if required, stay reusable
in the bothost fork; no host crates or bot action API inside the client.

Honor actual NPC size/height, camera/viewport and scene plane. Specify coordinate
space and match the existing host canvas origin/scaling, not synthetic corners.
Return null only for actual unavailable/offscreen/invalid state, never always
null. Preserve scene1 last-FBO freeze and CPU/GPU ownership; stale generation,
restart, target despawn and per-isolate routing must not leak the prior box.
Keep data bounded: no deep world copies or large projection history each read.
Prefer existing snapshot/transport conventions and compatibility for old buffers.

Verify successful projection geometry plus absent/behind-camera/scene transition,
actual reader call through native serialization, old-buffer/stale clearing and
per-slot isolation where applicable. Run affected focused crates with required
features and client integration tests separately if client behavior changes.
Renderer refactoring, if unavoidable, needs equivalence tests for the existing
projection, not a new visual policy. Exact source export for checks; never
restore concurrent files. Commit scoped host/client work, report exact checked
hashes and limitations, then request reviewer on this same card and stop.
