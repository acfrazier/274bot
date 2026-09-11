# Bounded coordinate reachability bridge design

Use configured grok46 defaults for an architecture/fidelity design, no runtime
implementation. Work in codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch and this brief. Deliver a concise design
in 04d-coordinate-reachability-design.md. No additional agents.

Isolated host 50f2be8a/client 56d8027 passes complete default GnomeCourse laps on
both frozen catalogs and revisions. Radius-8 old-catalog/289 failed with
`tick 131: not impl: Reachability.walkable` after lap 1 and a re-sync from log
balance to obstacle pipe. Raw evidence:
evidence/catalog-harness/live/r289-gnome-course-radius-100adccc-50f2be8a.*.
The current error is a missing bridge operation. Do not dim the card or change
host semantics to accommodate the imported script. Radius-8 re-sync could also
be a foreign script defect; diagnose that separately without inventing success.

AgilityBot.ts repositionForRetry checks four coordinate faces with synchronous
Reachability.walkable(f) && Reachability.canReach(f), then DirectNavigator.walkTo.
Current shim/reachability.js has no walkable; canReach only finds a matching
posted npc/ground row, so arbitrary empty-tile queries incorrectly get false.
Rust already exposes SceneQuery::walkable(WorldTile), collision_at and reach/path
queries in crates/api/src/query.rs. Preserve those existing Rust semantics,
selected scene/revision identity, unavailable/stale behavior and session reset.
No collision algorithm or routing policy belongs in JS; do not copy foreign
Reachability, import its world, or hardcode course/face/tile coordinates.

Inspect the actual script snapshot/FlatBuffer publication, isolate threading and
native callback seams before choosing the smallest viable synchronous query
bridge. Consider whether an immutable snapshot/Arc can be made available to
native isolate callbacks, or an existing bounded cached Rust-derived query view
can serve these calls. Explain snapshot consistency, invalidation, ownership,
thread safety, per-bot memory/copy/CPU cost and scope. Do not deep-copy the world
or flood-fill separately on every JS read. Do not casually add a generic IPC
RPC service to solve this one family. Existing entity canReach behavior must
remain compatible; identify how plain coordinate inputs should map honestly.

Identify the frozen enabled callers that need these exact two operations,
coordinate validation/default-level/fallback contract and the allowed minimal
source seams. Name a small falsifiable experiment and meaningful tests for
walkable blocked/open/unknown tiles, unreachable isolated floor, walls,
nonzero-plane/stale-session behavior and the synchronous JS-to-Rust call.
LIVE remains root-owned. Do not run new live cells or edit source/fixtures.

Write a concrete implementation recommendation and stopping rule, with any
independent radius-8 foreign algorithm finding clearly distinguished from the
missing native query mapping. A source review is not live acceptance. Commit
only the design report and complete this design card with actual source refs;
implementation will receive its own bounded card and same-card reviewer gate.
