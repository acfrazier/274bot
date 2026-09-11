# Map DirectNavigator.walkTo onto the existing native scene walk

Use configured grok46 defaults after the parent source review. Work on
codex/rs2b0t-multirevision. Read applicable instructions, docs/execution.md,
fail-closed-dispatch and current STATE.md ownership policy. No extra agents.

The coordinate reachability design identified the next missing call in enabled
GnomeCourse recovery: DirectNavigator.walkTo(dest, 0, 10000). Existing shim
DirectNavigator.walk already queues the native scene WalkTo packet. Existing
Traversal.walkTo dispatches that same operation and waits for posted arrival.
crates/host-play dispatch_script_interacts maps WalkTo to Interactions::walk;
Traveller is a distinct host operation. These ownership boundaries must stay.

Map DirectNavigator.walkTo(dest, radius=2, timeoutMs=45000) through the existing
host scene-walk/wait capability, using the current Traversal bridge where that
provides the required shape. The argument adaptation must preserve explicit
zero radius and caller timeout (10000 in Gnome recovery); missing defaults
follow the exact frozen DirectNavigator surface. Reuse existing host semantics.
Do not copy the foreign DirectNavigator retry/clamping/controller, add a route
planner, change native arrival semantics, or special-case Gnome/course tiles.
A queued send is not a successful awaited arrival. If a real native operation
is missing rather than a thin name/argument mapping, report that exact seam
before inventing a new service or JavaScript navigation policy.

Allowed scope: shim/direct_navigator.js, minimal export/import wiring only if
needed, one focused isolate integration test file, report
04d-direct-walkto-mapping.md and evidence/direct-walkto/. Existing Traversal,
Execution, api/nav/host/client algorithms and timeouts remain unchanged. Do not
edit coordinate-query publication, Tile geometry, catalog/scenario/ledger,
frontend, fixture engines, LIVE or remotes. Preserve concurrent WIP.

Meaningful regression proof: real isolated DirectNavigator.walkTo queues the
same scene WalkTo, remains pending before observed arrival, returns true only
when posted host coordinates satisfy the existing native bridge condition,
preserves explicit radius 0 versus default 2 and requested timeout, and does
not arm Traveller or succeed on a different plane. Exercise an unreachable/
expired request and Stop/session abort using existing harness seams; do not
wait 45 seconds merely to mirror a default literal. Verify the original direct
walk and Traversal tests retain their behavior. Keep any pre-existing wait
limitation explicit rather than expanding this bounded mapping task.

Use a new empty Cargo target on the exact committed export plus only owned
overlay; record source/client hashes and affected tests/Clippy. Source review
is not LIVE proof. Commit only your owned files; request SAME-card profile
reviewer with evidence, then STOP. Root owns the preserved radius-8 failure,
new isolated LIVE diagnosis, and any later foreign-card dim decision.
