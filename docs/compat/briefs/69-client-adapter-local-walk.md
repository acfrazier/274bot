# Map ClientAdapter local coordinates and scene walk

Use profile grok46 defaults on codex/rs2b0t-multirevision. Read applicable
instructions, docs/execution.md, fail-closed-dispatch, current STATE.md and
reviewed coordinate/canStep reports. Do not spawn agents. Root owns LIVE.

Exact 3be68eaf FlaxPicker LIVE now picks actual flax on both revisions, then
fails at tick 88/near the full pack on `not impl: reader.toLocal`. Both failures
are retained in evidence/catalog-harness/live/*flax-picker*3be68eaf*.log.
Frozen FlaxPicker.travelTo/walkLocal calls reader.toLocal(world x,z), then
synchronous actions.walkTo(local lx,lz) and awaits observed position itself.
Both adapter members are missing in crates/script/src/shim/client_adapter.js.

Implement those two thin mappings only. The frozen ClientAdapter.toLocal
subtracts mapBuildBaseX/Z and returns {lx,lz} inside the 104x104 scene, else
null. actions.walkTo passes local coordinates to the existing client move.
Our reviewed posted reach view already carries native scene base, dimensions,
level and availability. Use those facts for the coordinate shape adaptation;
queue the existing `walk-to` Rust interaction with world coordinates/current
plane as DirectNavigator.walk already does. Do not copy foreign tryMove,
pathfinding, collision, retries, clamping or Traversal policy into JavaScript.
No extra snapshot/scene copy, native flood, service, packet, or runtime field.
Queue admission is not an observed movement outcome; preserve the existing
host command semantics and the caller's own arrival wait.

Fail closed when scene facts are unavailable, detached/stale or coordinates
are outside the native scene/host coordinate contract. Check existing integer
and plane semantics rather than inventing a new navigation policy. Current
scene base/plane changes must affect the very next conversion/action. Missing
old-buffer reach must not silently route via a zero base. Pause/Stop/session
admission remains owned by the existing Rust runtime.

Own ONLY crates/script/src/shim/client_adapter.js, a new unique
crates/script/tests/client_adapter_local_walk.rs (plus report/evidence). Do not
edit shared load.rs, isolate_fb.rs, service.rs, runtime.rs, shim/mod.rs,
load_isolate.rs, API or scenario/catalog files: DeathRecovery and vial/potion
workers own those areas. If the existing posted facts cannot support this exact
mapping, report the missing seam to root instead of crossing that boundary.

Use real isolate tests with nonzero bases, edges, wrong/out-of-scene inputs,
unavailable/old data, base/plane change, and exact queued world walk coordinates
and operation order. An unavailable query/action must queue nothing; two slots
must not exchange scene bases. Verify exact export plus owned overlay and client
bytes in a NEW EMPTY target. Focused script test, formatting and strict affected
Clippy suffice; no broad test churn. Write 04f-client-adapter-local-walk.md and
evidence/client-adapter-local-walk/, commit only owned paths, inspect scope,
request SAME-card profile reviewer, then STOP. No foreign-source or LIVE changes.
