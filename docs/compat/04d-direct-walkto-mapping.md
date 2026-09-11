# DirectNavigator.walkTo mapping

Task `t_f7fd9f55` on `codex/rs2b0t-multirevision`. Isolated checks used
git archive of host `4be9084389211a1dfb9ed7a1c2b79daee75cf354` plus only
the owned overlay (`direct_navigator.js`, `direct_walkto.rs`). Client
gitlink `56d80272bcbda3eb1e22db096c1c5e21d3497de4`. Not LIVE, fixtures,
Gnome radius-8, or foreign-script diagnosis.

## Bridge

`DirectNavigator.walk` still queues scene `walk-to` and returns immediately.
`DirectNavigator.walkTo(dest, radius = 2, timeoutMs = 45000)` is the awaited
name: it forwards those positional arguments to existing
`Traversal.walkTo(dest, { radius, timeoutMs })`. Explicit radius `0` and
timeout `10000` (Gnome `repositionForRetry`) are preserved. Missing arguments
use the frozen DirectNavigator defaults, not Traversal's `radius 0` /
`timeoutMs 60000`.

Arrival is the existing kernel Chebyshev wait, including same-plane.
A queued send is not success. Traveller (`walk` / `walk-near`) is not armed.
The foreign DirectNavigator retry/±48 clamp/`arrivalProbe` controller is not
copied. Traversal, Execution, api/nav/host/client algorithms are unchanged.

Invalid dest (`null` / non-numeric `x` or `z`) returns false without a queue,
matching `DirectNavigator.walk`.

## Pre-existing wait limitations (unchanged)

- One scene WalkTo per call; no stall re-issue.
- `reset_session_work` clears the interact queue and does not reject a parked
  `Execution.delayUntil`. Later posted coordinates can still satisfy the wait.
- `ThreadMsg::Stopped` clears the host interact buffer. ScriptRunner.stop
  while parked ends the isolate without resolving walkTo true.

## Proof (isolated empty target)

Export `.superpowers/task-exports/t_f7fd9f55-4be90843` with
`CARGO_TARGET_DIR=.../target-t_f7fd9f55` (`isolated_build=true`).
Raw logs: `docs/compat/evidence/direct-walkto/`.

- walkTo queues `InteractReq::WalkTo`, stays pending, returns true only after
  posted `here` satisfies Chebyshev on the dest plane.
- Default radius 2 treats Chebyshev 1 as already arrived; explicit 0 does not.
- Other-plane dest does not arrive on the current plane and does not arm
  Traveller.
- Unreachable dest with timeout 80 ms returns false (no 45 s wait).
- Stop while parked does not resolve true. Session reset leaves the parked
  wait in place.
- Existing `DirectNavigator.walk` and `Traversal.walkTo` isolate tests still
  queue scene WalkTo.

`cargo clippy --locked --offline --no-deps -p script --all-targets -- -D warnings`
passed. rustfmt --check on the new test file passed.

## Out of scope

LIVE, Gnome radius-8 / foreign resync, `canStep`, coordinate publication,
Tile, catalog/scenario/ledger, frontend, Traveller, and host arrival
semantics remain separate. A green isolate test is not live success and
does not dim GnomeCourse.
