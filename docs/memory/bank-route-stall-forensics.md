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

The host did find a route and issued one driver-accepted walk command. The
time series shows substantial southward movement, followed by a guardian
stun/hold and northward reversal before the route later expired. It does not
prove that the guardian interruption caused the eventual bank failure: the
diagnostic and navigation ticks are different domains, and there is no
per-tick route/hold join. It also does not prove collision, scene loading, wire
rejection, a missing nav pack, food depletion, or a shared-`NavWorld`
regression.

## What is proven

The terminal snapshot for `live121fc_14` contains:

- position `(2660, 3305, 0)`, HP `47/50`, food `3`, `bank_open=false`,
  `bank_loaded=false`, `hold=false`, and no stun state at the terminal sample;
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

The sampled diagnostic series adds important temporal evidence before that
terminal record. At elapsed 143.321--150.454 seconds, slot 14 moves from
`(2662,3307)` to `(2662,3303)`, `(2662,3301)`, and `(2662,3297)`; subsequent
samples show it reversing through `(2662,3301)`, `(2662,3303)`, and `(2660,3305)`.
The client samples report `hold=true` during the reversal, animation 424 and
spot animation 245 at the onset, chat `You've been stunned!`, then repeated
`You're stunned!`; the hold later clears with chat `It's not here for you.`.
This is guardian/random-event evidence, not terminal-only state. The
corresponding diagnostic `client_tick` values (242--254) must not be equated
with navigation tick 230 or runtime tick 297. The nav generation and request
batch count remain unchanged, and the capture shows only one `WalkAttempt`.

Thus `FindEnd=Routed`, `WalkAttempt refusal=None`, movement in both directions,
guardian hold/stun samples, `Leg Failed`, `Stalled/Expired`, and `tries=1` are
all direct observations. The route was not a `NoPath` result and the send was
not synchronously refused. `refusal=None` means the driver accepted/queued the
interaction synchronously; it is not a wire or server acknowledgement.
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
The existing route/generation safeguards are not shown to be the cause by this
trace: generation 1 was found and followed, with no competing generation in
the captured slot evidence. Separately, `crates/host-play/src/lib.rs:2435-2439`
intentionally returns early while guardian `hold` is true, leaving the armed
hop latched for later resumption. That preserves ordering and does not itself
show whether the game moved the player, rejected a command, or changed the
effective route during the hold.

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
The current capture does not show whether the original policy would have retried
this particular interrupted movement, nor whether the game's walk command was
accepted on the wire and then ignored, superseded, blocked by a live actor, or
unable to advance because of scene/collision state. The existing guardian
freeze/resume behavior is observable in source, but the capture does not join
its hold transitions to the traveller's hop state at the same tick.

Missing from the raw evidence are:

- the walk-command wire/action acknowledgement or client action queue result
  beyond synchronous driver acceptance;
- the loaded scene base/size and collision flags for the sent aim and the next
  route tiles at tick 230 and during the stall;
- per-tick server-confirmed position transitions between the accepted send and
  expiry, beyond the sampled snapshots;
- any client chat or action rejection that would explain the later bank stop;
- a same-domain (or explicitly correlated) hold/stun transition alongside the
  traveller hop and its tick budget.

The final snapshot's normal scene state and nonzero food do not fill those
holes. In particular, do not infer a missing nav pack, food exhaustion, death,
stun cause, or `NavWorld` sharing defect from this run.

## Smallest next step

Reuse the existing opt-in `BOT_NAV_CAPTURES=1` failure checkpoint for one
narrowly scoped TUI reproduction (not a benchmark or live rerun series).
The host already serializes a full `GameSnapshot`, including scene base,
dimensions, and collision flags, at the first navigation failure in
`crates/host-play/src/nav_capture.rs:42-58,102-108`; do not create a second
capture sink. Because the current enable/drain path is panel-gated, the
smallest implementation step is a data-only TUI enable/drain path that reuses
that checkpoint. Include only the existing failure detail plus a bounded
per-slot ring of route/guardian transition records if the checkpoint cannot
correlate the interruption. The reproduction should record:

1. the exact walk command payload and driver acceptance/result;
2. scene base/dimensions and collision flags from the existing full snapshot,
   with the current tile, selected aim, and next few route tiles identified;
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

The host has no wire/server ACK surface in this path; that remains an explicit
missing capability even after the capture is enabled. The raw run had
`nav_captures=false`, so the existing scene/collision checkpoint was simply
not emitted for this run. No code, client, fixture, server, timeout, policy,
or raw artifact was changed by this diagnosis.
