# Bounded native reachability

Date: 2026-09-11. Kanban: `t_25797d04`. Scope: brief 140 native API flood metadata, additive snapshot transport, host projection, thin script binding, generated host declaration, and focused regression coverage. No client, catalog, scenario, fixture, navigation-controller, or LIVE changes.

## Result

Coordinate `Reachability.canReach` now honors the foreign expansion budget without running a JavaScript flood. The existing native scene flood records each reachable tile's zero-based dequeue rank while preserving the established `W E N S NW NE SW SE` enqueue order. A second native pass over that same queue records the earliest rank that reaches either the exact target or a wall-valid orthogonal neighbor. It does not run a second flood.

The packed view carries two `u16` maps using the existing `lx * height + lz` index:

- `exact_rank`: earliest exact dequeue rank;
- `adjacent_rank`: earliest exact or valid-adjacent dequeue rank;
- `65535`: unreachable sentinel.

The O(1) bounded answer requires the corresponding legacy reach bit, a complete rank map, a non-sentinel rank, and `rank <= maxSteps`. This matches `SceneQuery::can_reach`, whose destination/adjacency check occurs before incrementing and rejecting the expansion count. Omitted `maxSteps` uses 400. Explicit zero accepts the origin and a legitimate origin-adjacent target. Invalid budgets, dimensions, planes, regions, missing maps, and unavailable scenes fail closed.

The former NPC/ground-tile shortcut was removed from `Reachability.canReach`. Coordinate targets now use one uniform native budget even when an entity row occupies the tile. Entity query convenience booleans remain unchanged. `walkable`, `canStep`, native navigation clocks, and native routers are unchanged.

## Adjacency

A whole-flood adjacency bit is not sufficient for a bounded answer. During the queue-order pass, each reachable source tile contributes its own dequeue rank only to orthogonal target tiles whose target-facing wall mask permits interaction. The target's exact rank remains eligible through the cloned exact map. Tests compare the packed O(1) answer against the existing bounded Rust BFS on open floor, a one-tile corridor, a collision pocket, a blocked adjacent tile, and a wall-blocked adjacent tile. They cover zero, explicit small, default 400, and larger-than-scene budgets, including a target whose adjacent answer becomes true before its exact answer.

## Protocol and lifecycle

The FlatBuffer `Reach` table appends `exact_rank` and `adjacent_rank` after the existing `step` vector. Old buffers still verify, but bounded coordinate reach fails closed when either rank map is absent or has the wrong tile count. Full snapshots post both maps. Fingerprints include both maps, so unchanged deltas omit the whole reach table and the isolate retains its current view without recreating V8 arrays. An available-to-unavailable post clears bits, ranks, step masks, dimensions, and availability together. Fresh replacement isolates start unavailable and cannot inherit another isolate's ranks. Existing host session-reset behavior clears the per-slot fingerprint, forcing the next post to be a keyframe; scene-unavailable, missing-player, plane, and region checks remain fail closed.

The generated host declaration now includes `ReachQueryView` and `Snapshot.reach`; its freshness test passes.

## Size accounting

For the typical 104 by 104 scene (10,816 tiles):

- three packed reach/walk bitsets: 4,056 bytes;
- step masks: 10,816 bytes;
- two `u16` rank maps: 43,264 bytes;
- derived Rust view total: 58,136 bytes;
- increase over the prior 14,872-byte derived view: 43,264 bytes per available view.

These are exact logical vector payload sizes, not a heap/RSS or serialized FlatBuffer measurement. They exclude vector capacities, FlatBuffer framing/alignment, temporary host packing storage, and JavaScript array representation. No performance or memory-savings claim is made. The existing producer still derives and fingerprints reach data per running-slot post; this change guarantees that unchanged deltas do not encode the rank vectors or materialize new V8 arrays.

## Verification

All Cargo commands used `CARGO_TARGET_DIR=.superpowers/task-targets/bounded-reachability-t_25797d04`, `--locked`, and `--offline`.

- API full crate tests: PASS (206), including native projected-vs-BFS parity and the default-400 rejection.
- Script library tests: PASS (83).
- Script reachability isolate tests: PASS (7), then the lifecycle/replacement test independently after its final assertion update.
- Script local-walk integration tests: PASS (8).
- Generated host declaration tests: PASS (2; regeneration helper ignored).
- Host-play library tests: PASS (144), including decoded host rank transport.
- Strict Clippy for affected API, script, and host-play targets with `-D warnings`: PASS.
- `rustfmt --check` on scoped Rust files: PASS.
- scoped `git diff --check`: PASS.

Evidence and provenance are under `docs/compat/evidence/bounded-native-reachability/`.

LIVE requalification remains root-owned by brief 140 and was not run here.
