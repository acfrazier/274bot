# Memory campaign behavior contract

Working clarification, 2026-09-07. The operator explicitly permits reconsidering
application architecture while retaining the script compatibility layer and
client fence for maintainability. This document makes a reviewable distinction;
it does not authorize an unreported script-visible semantic change. Existing
running task briefs keep their stronger regression obligations through review.
The resource targets and final qualification in performance-finish-plan.md remain.

## Required outcomes

- Preserve supported script API names, argument and result meanings, ordering
  and completeness of collections, numerical behavior, supported errors, and
  lifecycle/wait/cancellation semantics. Keep the compatibility layer as the
  adapter onto Rust host operations; do not recreate the foreign runtime.
- Preserve protocol correctness, game interactions, independent clients and
  accounts, operator controls, visual correctness, and configured renderer modes.
  Keep the maintained client fence; move no bot action API into the client.
- Preserve meaningful ordering: an action must not execute against another
  client's state or a different script invocation; a stopped invocation must
  not cause a later account to execute its pending commands.
- Preserve immutable observations for their documented lifetime. A script or
  navigation reader retaining an observation must not see it mutate underneath
  its decisions. Sharing storage is permitted; mixing incompatible epochs is not.
- Preserve responsiveness, lifecycle cleanup and memory/CPU goals as independently
  tested requirements. A lower allocation count cannot excuse a missed gate.

## Game fidelity and inherited client timing

Operator clarification: the 20ms client tick derives from the Java client's
execution model; architectural freedom must preserve what makes the game behave
like the game. Treat logical client progression and ordering as semantic until
source and functional evidence prove a mechanism is incidental.

Host scheduling mechanics may change, but this does not authorize fewer logical
client updates, a variable-step simulation, catch-up batching, or altered ordering
of packet processing, input, movement, animation, camera, audio and interface work.
Any such proposal needs a separate explicit fidelity analysis and equivalent
observable sequences under load and transitions. Similar average FPS is not that
proof. Keep the current 20ms logical client scheduling target and existing cadence
gates while optimizing the surrounding host. Preserve client fence and port
maintainability; do not turn a host memory optimization into a game-engine rewrite.

## Mechanisms open to redesign

Ownership, queues, caches, threads, scheduling mechanics, internal state transport,
and snapshot production can change when the required outcomes still hold. There
is no intrinsic requirement to preserve the number of snapshot shells, rebuild
walks, copies, queue objects or allocations. A current publication boundary is
not automatically a public guarantee: trace its consumers before changing it.
The script compatibility layer and client fence themselves remain maintained.

Internal wire bytes are not assumed public or private: audit producers and
consumers first. Existing byte-for-byte tests remain required for the currently
scoped fingerprint candidate; removing that requirement needs an explicitly
identified replacement contract, not deletion of a failing test.

Incidental timing and accidental error strings need not become permanent API
promises. Equally, calling an outcome a bug is not evidence: identify the broken
contract, provide a reproduction, and demonstrate the corrected outcome. Preserve
intentional unsupported operations and deliberate errors until that capability
is explicitly in scope. Do not quietly change configured timeouts or render
cadence to meet a budget.

## Concrete review scenarios

| Scenario | Required evidence |
|---|---|
| Host and script share widget storage | Identical script-visible values and ordering; retained older observation remains stable after a new publication; no cross-client reuse |
| Eliminate an internal pre-drain snapshot | Trace guardian, navigation and script consumers; compare action/hold/observation sequence around a packet arrival, including a pending wait |
| Restart a slot while an old script finishes | Old commands/observations cannot reach the replacement account; cancelled work releases its owners |
| Replace worker-per-slot scheduling | Same required progress, waits, command ordering and cancellation; measured latency/cadence under load; no client-fence violation |
| Fix Unknown error | Identify whether it represents termination, a script exception, or another failure; preserve supported error semantics and prove the original failure case |
| Change private state encoding | Audit all consumers; prove equivalent values, omitted/empty distinctions, ordering and lifetime at the compatibility surface |

## Applying the distinction

For each architecture proposal, list the public outcomes it affects, internal
mechanisms it changes, the oracle for each affected outcome, and an estimate
linked to measured owners. Separate existing bug fixes from performance changes
in the evidence. Report any externally observable difference explicitly before
calling the change behavior-preserving. Do not claim the entire architecture is
frozen merely because one current task uses exact implementation-level oracles.

The first decision is whether shared immutable families can eliminate measured
duplication without altering even the current publication contract. If not,
report the exact counterexample and assess a host publication redesign against
these outcomes. Do not force unsafe generation-only sharing to satisfy a design.
