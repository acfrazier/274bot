# ClientAdapter local coordinates and scene walk

Task `t_15c05b42` on `codex/rs2b0t-multirevision`. Isolated checks used
git archive of host `d903cc882da7784ec2434817cc0d269d724a9aa3` plus only
the owned overlay (`client_adapter.js`, `client_adapter_local_walk.rs`).
Client git archive `56d80272bcbda3eb1e22db096c1c5e21d3497de4` (tarball
sha256 `ffe863519e87cc44e56823bab93ab275f42408a9f0ccbf8fcbc070cf7c881a0f`),
not `git rev-parse` inside the gitless export. Not LIVE, fixtures, or
foreign FlaxPicker/picking diagnosis.

## Bridge

`reader.toLocal(x, z)` subtracts the posted native scene `base_x`/`base_z`
and returns `{lx,lz}` when that point sits inside the posted width/height.
Otherwise it returns null. Non-integer world inputs and a missing,
unavailable, or zero-dimension reach view also return null. An old buffer
without `reach` does not convert through the unavailable zero base.

`actions.walkTo(lx, lz)` is synchronous. In-scene integer locals queue the
existing `walk-to` Rust interaction at world `base + local` on the posted
scene plane, matching `DirectNavigator.walk`. Queue admission is not
arrival; the caller still waits on observed position. Out-of-scene,
non-integer, or unavailable inputs return false and queue nothing.

No foreign tryMove, pathfinding, collision, retries, clamping, or Traversal
policy. No extra snapshot/scene copy, flood, service, packet, or runtime
field. Pause/Stop/session admission stays in the existing Rust runtime.

Current scene base/plane changes affect the next conversion/action. Two
isolates keep separate posted bases.

## Proof (isolated empty target)

Export `.superpowers/task-exports/t_15c05b42-d903cc88` with
`CARGO_TARGET_DIR=.../target-t_15c05b42` (`isolated_build=true`).
Raw logs: `docs/compat/evidence/client-adapter-local-walk/`.

- Nonzero 104×104 origin/edges map; west/east/south/north of the scene are
  null. `toLocal` queues nothing.
- Non-integer, missing, swapped-out, and unavailable/old data return null
  or false and queue nothing, including world `(5,5)` against a zero base.
- `walkTo(5,10)` on base `(3222,3228)` plane 1 queues
  `InteractReq::WalkTo { x: 3227, z: 3238, level: 1 }` and returns true on
  the same tick.
- Only in-scene integer locals queue, in call order. Edges `0,0` and
  `103,103` queue; `-1` and `104` do not.
- Frozen Flax pair `toLocal` then `walkTo` queues one scene WalkTo at the
  world dest.
- A later base/plane post changes the next conversion and queued world
  coordinates. Two slots do not exchange bases.
- An omitted unchanged-reach delta keeps the last scene; an unavailable
  post clears it.

`cargo clippy --locked --offline --no-deps -p script --test client_adapter_local_walk -- -D warnings`
passed. `rustfmt --edition 2021 --check` on the new test file passed.

## Out of scope

LIVE, Flax 274 full-pack timeout diagnosis, picking/fixture behavior,
Traversal, DirectNavigator.walkTo, canStep, DeathRecovery, and shared
runtime/API files remain separate. A green isolate test is not live flax
success.
