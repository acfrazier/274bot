# Bank-route stall forensics

Task `t_7269f401`. Read-only diagnosis of managed run
`docs/memory/diagnostics/20260907T014123Z_tui_n16_active`; this is not a
qualification, performance, capacity, or N=16 support result.

## Bottom line

Slot `live121fc_14` stopped normally through the harness at tick 529 with the
cached script reason `could not reach Bank booth bank`. The stop was not an
outer watchdog timeout. The final diagnostic record is
`samples.diagnostics.jsonl` line 315, at elapsed `322.415266709` seconds. It
shows `stop_reason` in the terminal runtime snapshot, and the paint title
carries the same text.

The host did find a route, issued one accepted walk command, observed only
partial movement, and then terminated that route after the per-hop budget
expired. The evidence does not identify why movement stopped. It is consistent
with a client/game-side dropped or blocked walk, but does not prove collision,
scene loading, wire rejection, a stun, a missing nav pack, food depletion, or a
shared-`NavWorld` regression.

## What is proven

The terminal snapshot for `live121fc_14` contains:

- position `(2660, 3305, 0)`, HP `47/50`, food `3`, `bank_open=false`,
  `bank_loaded=false`, `hold=false`, and no stun state;
- `ingame=true` and `scene_state=2`;
- runtime `dispatched=506`, `last_completed_tick=529`, with the exact cached
  `stop_reason` above;
- recent requests ending in exactly one
  `WalkNear { x: 2656, z: 3286, level: 0, radius: 3,
  allow_teleports: false }`; preceding requests are pickpockets and one eat.

The navigation trace for the same slot is:

1. `FindStart generation=1 from=(2662,3307,0) to=(2656,3286,0) radius=3`.
2. `FindEnd generation=1 outcome=Routed`.
3. `Leg Start Walk tiles=27`, from `(2662,3307,0)` to `(2653,3289,0)`.
   The leg endpoint is within the requested radius, so this is a valid
   radius-approach route rather than evidence that the destination was
   unreachable.
4. `WalkAttempt tick=230 at=(2662,3307,0) aim=(2653,3289,0)
   refusal=None`.
5. The client later reaches `(2660,3305,0)` and remains there. At tick 297:
   `Leg Failed ...`; `FollowEnd ... outcome=Stalled { at=(2660,3305,0),
   aiming=(2653,3289,0), why=Expired, tries=1 }`.
6. The script eventually stops at tick 529 with the cached bank reason.

Thus `FindEnd=Routed`, `WalkAttempt refusal=None`, partial position change,
`Leg Failed`, `Stalled/Expired`, and `tries=1` are all direct observations.
The route was not a `NoPath` result and the send was not synchronously refused.
The managed process exited 1, has `observe-start` but no `observe-end`, and
must remain excluded from performance claims.

## Host path and state-machine interpretation

The JS compatibility path is `crates/script/src/shim/traversal.js:32-47`.
`Traversal.walkResilient` returns immediately when already within the radius;
otherwise it queues one `walk-near` request and waits in `Execution.delayUntil`
for the snapshot position to enter that radius. It does not itself re-BFS,
retry, or inspect a route outcome.

The request maps to `InteractReq::WalkNear` in
`crates/host-play/src/lib.rs:950-952`, which calls
`ScriptWalkArm::route_with_radius`. That path records a per-slot generation,
keeps one worker and one latest pending request (`lib.rs:2310-2319,
2356-2372`), and publishes only a routed result (`lib.rs:9442-9459`). The
existing route/generation safeguards are therefore not implicated by this
trace: generation 1 was found and followed, with no competing generation in
the captured slot evidence.

The route follower is `crates/nav/src/traveller.rs:605-634` and
`1054-1147`. On the first walk-leg poll it sends the selected aim once. Later
polls settle that same aim. Each poll is bounded by
`TravelOptions::budget_ticks_per_hop`, default 60 (`traveller.rs:274-285`),
and on expiry returns `Stalled`; the `tries` value remains 1 because no retry
hop was sent. In this run, the trace has no second `WalkAttempt`, no refusal,
and no route-generation change after the first attempt. That is the exact
mechanism by which a routed leg can move a few tiles and then expire.

This is a demonstrable behavioral limitation, not yet a proven root-cause
bug: the current `walkResilient` mapping is a single queued host request plus
one wait, while the captured host follower's `Expired` outcome ends the route.
The brief requires preserving the current bounded budgets, ordering, old-route
retention on failed search, per-slot generation/token ownership, pending-search
bound, retry-latch clearing, and the no-walk-when-adjacent behavior. Nothing in
this report proposes changing those semantics.

## Reference contract and missing evidence

The original ThievingBot/`Traversal.walkResilient` contract supplies bounded
walk attempts (the prior compatibility audit records four attempts) and the
current shim's relevant mapping is the one-call `walkResilient` path above. The
current capture does not show whether the original policy would have retried
this particular partial movement, nor whether the game's walk command was
accepted on the wire and then ignored, superseded, blocked by a live actor, or
unable to advance because of scene/collision state.

Missing from the raw evidence are:

- the walk-command wire/action acknowledgement or client action queue result;
- the loaded scene base/size and collision flags for the sent aim and the next
  route tiles at tick 230 and during the stall;
- per-tick server-confirmed position transitions between the accepted send and
  expiry, beyond the sampled snapshots;
- any client chat or action rejection that would explain the stop.

The final snapshot's normal scene state and nonzero food do not fill those
holes. In particular, do not infer a missing nav pack, food exhaustion, death,
stun cause, or `NavWorld` sharing defect from this run.

## Smallest next step

Use one narrowly scoped, opt-in failure-capture diagnostic for a single
reproduction (not a benchmark or live rerun series) that records, for the
failing slot and each player-info tick from `WalkAttempt` through `Stalled`:

1. the exact walk command payload and driver acceptance/result;
2. scene base/dimensions and collision flags for the current tile, selected
   aim, and the next few route tiles;
3. server-confirmed position, animation/hold/stun fields, and any new chat
   rejection line; and
4. route generation, aim/cursor, hop tick count, and whether a second attempt
   was dispatched.

Keep existing timeout, action ordering, route retention, generation/token,
pending-search, retry-latch, and adjacency behavior unchanged. The capture
should stop on the existing demonstrated failure and emit the same terminal
cached reason; it must not add shim retries, extend timeouts, alter loadouts,
reset the server, or weaken qualification. This is the smallest diagnostic
that distinguishes a wire/driver rejection, scene/collision blockage, and
state-machine expiry before anyone decides whether a deterministic host
regression is possible.

No host capability is silently assumed: the current diagnostic surface lacks
those wire/scene/path details, so a targeted capture is required before a
reproduction can be classified as a host defect. No code, client, fixture,
server, timeout, policy, or raw artifact was changed by this diagnosis.
