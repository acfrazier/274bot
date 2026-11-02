# hold-model

A checked model of the auto-run hold. It is a developer tool. No product crate
depends on it, except as a dev-dependency of the conformance tests in
`crates/host-play` and `crates/script-js`.

The model follows the product at commit `178a00200` and the fix of finding F1
(`HOLD-MODEL-MERGE`: the host holds a renewed wait again). The explorer
searches it breadth first inside caps. A violation prints its shortest trace,
step by step, in the product's terms.

## What the model covers

A script can hold the host's auto-run press while it waits for the page of its
own interaction. The model has one slot with one script and at most one wait
lease. The lease kinds are:

| Kind | Owner | Bound |
|---|---|---|
| `Modal` | native `Dialogue` modal wait (`quester/families/dialogue.rs`) | page and acknowledgement waits on the active clock. End gap on observed ticks. |
| `Quiet` | native quest-journal quiet read (`quest_journal/native.rs`) | a window of observed ticks per phase, renewed at each phase. A 10 s wall bound on the lease. |
| `Compat` | compatibility (JS) quest-journal read (`script-js/.../quest_journal/isolate.rs`) | a wall bound from the begin, renewed at the click. A separate paint cap. |

Each frame has the product's order (`crates/host/src/lib.rs:739,758`):

1. The drain. A PLAYER_INFO is one game tick.
2. The host's session-boundary release (`host/src/lib.rs:1536-1546`).
3. The observe hook: the session reset, the clock sync
   (`host-play/src/script_observe.rs:608`) and the owner's poll (`:815-827`).
   The poll runs only while the slot is `Running`, on a tick edge or an
   evidence wake.
4. The host samples the lease (`script/src/slot.rs:2259-2294`, also while
   paused) and applies its wait policy (`host/src/lib.rs:225-267`).
5. The press decision (`host/src/lib.rs:1548-1588`).

The inputs are Pause, Resume, Stop, Start, a disconnect, a connect, a relog,
a modal that opens, closes or changes page, a guardian hold, a prayer-cleanup
hold, a death, a run echo, and the passing of time.

### Where each model function comes from

| Model function (`model.rs`) | Product code |
|---|---|
| `frame` | `host/src/lib.rs` `client_frame_observed` (`:739` observe, `:758` press), `script_observe.rs` |
| `observe_wait`, `deferred` | `host::ScriptRunPolicy::observe_script_wait`, `should_defer_auto_run` (`host/src/lib.rs:240-318`); the re-arm at a renewal is in the same-key arm |
| `sample` | `SlotScript::script_interaction_wait` (`script/src/slot.rs:2259-2294`) |
| `sample`, Modal arm | `WaitDeadline::owner_tick` (`script/src/native.rs:922`), `Ledger::modal_wait_tick` |
| `quiet_wall_release`, `sample`, Quiet arm | `Ledger::quiet_read`, `quiet_read_wait` (`script/src/native/ledger.rs:349-410`) |
| `sample`, Compat arm, `poll`, Compat arm | `CompatJournalLease::wait`, `renew_hold` (`script-js/.../quest_journal/isolate.rs:146-190`), `JournalRuntime::click` |
| `begin`, `send`, `to_gap`, `rearm` | `Dialogue::sync_modal_wait` (`dialogue.rs:361-380`), `ActionContext::begin_modal_wait`, `renew_modal_wait` |
| `poll`, Quiet arm | `JournalMachine::poll`, `arm_window` (`quest_journal/native.rs`) |
| the press in `frame` | `SlotLoop::apply_auto_run` (`host/src/lib.rs:1548-1588`) |
| the boundary in `frame` | `SlotLoop::after_drain` (`host/src/lib.rs:1536-1546`), `SlotScript::session_boundary` |

## Properties

Each property has a plain statement and an invariant in `model.rs`
(`check_frame`). The invariants are frozen. A mutation changes transitions
only.

| Name | In plain words |
|---|---|
| S0 | The host holds the press only while the sample of this frame returned a wait. |
| S1 | No press goes out while a lease is live and its owner's own deadline, in the owner's own clock, has not passed. |
| S2 | After Stop, a session boundary or a death cancel, no older lease or hold stays (bound: see below). |
| S3 | The host ends a hold without the script only as a logged terminal failure, and not before the owner's deadline plus the margin of one tick. |
| S4 | A lease's identity never carries a released state to a new transaction. |
| S5 | The journal page stops being hidden at the wall cap from its begin. |
| S6 | An owner that sends a verb (Talk, Continue, answer, click, close) gets no press in the same frame. The press would close the page that the verb opens (`docs/api/nav.md`: the click never goes out without a live hold). |
| L | Bounded liveness: a hold that is live at tick T has an owner deadline that did not pass at tick T minus the margin. The deadline is counted in the owner's clock (active clock, ticks or wall). |

### The bound of S2, in frames

- Stop. `SlotScript::stop` revokes the native runtime
  (`script/src/slot.rs:1654`) between frames. The runtime generation is no
  longer reported (`host-play/src/script_observe.rs:1317-1321`), so the host
  clears its hold at once (`host/src/lib.rs:203-209`). The bound is **0 frames
  after the next sample**: the first frame after the Stop holds nothing.
- Relog, disconnect, session change. The host releases in the drain
  (`host/src/lib.rs:1536-1546`). The observe hook calls
  `reset_session_work` before the poll (`host-play/src/play_slots.rs:1551`),
  which revokes the native runtime (`script/src/slot.rs:1757`) and the compat
  lease (`script-js/src/runtime/isolate.rs:1274-1279`). The bound is **0
  frames**: the frame that sees the boundary holds nothing from the old
  session.
- Death. The runner cancels its step when it reads the death line
  (`script/src/quester/runner.rs:3010`). Between zero hitpoints and that line it
  does not poll its step (`:3039`). The bound is **the first owner poll after
  the death line**. If no owner poll comes, the hold ends at the owner's
  deadline plus the margin (property L).

## Abstractions

Each abstraction is on purpose. The reason says why it is safe, or what it
cannot show.

1. **Time is three small counters.** Ticks (`tick`), wall units (`wall`) and the
   owner's active clock (`active`). The active clock moves with wall time
   only while the slot is eligible. Every comparison in the product is `<` or
   `>=` between a bound and a clock, so the model keeps the order of the
   events and drops the values. One unit is about 2.5 s: a Dialogue page wait is 3
   units, an acknowledgement wait 1, the 10 s wall bounds 4.
   The profile `alt` uses other ratios. A bound that is longer in the product
   is longer in the model, so an ordering race between two bounds stays visible.
2. **Time between the poll and the sample is one unit.** The product reads
   `Instant::now()` after the poll (`script_observe.rs:1141`). The model lets
   a unit pass there (`drift`). A real gap is smaller than a unit. The
   orderings are the same: the sample sees a bound that the poll did not.
3. **One lease at a time.** The ledger has one foreground owner. A runner
   runs one step, and the compat read belongs to a JS run. Two native leases
   at once, and a compat read next to a native one, are not reachable.
4. **The owner is a small machine.** The `Dialogue` phases are collapsed to
   `Page`, `Ack` and `Gap`. A poll does one phase. The product continues in
   the same tick after a begin or a completion (`may_continue_this_tick`).
   The model needs a second frame for that. The owner may also do nothing at
   any poll (`Stall`): a snapshot without chat facts, a spent budget. A stall at
   an expired wait is the case that drove finding F1, now fixed.
5. **A new send, a re-armed gap and a new read are new leases.** A change of
   phase that keeps the lease (acknowledgement to gap, each journal phase) is a
   renewal of the same lease and the same key
   (`dialogue.rs:361-380`). The host cannot tell a renewal from a new send
   that kept its key; it sees the same key and a later deadline. After the
   terminal release, such a sample holds again (`HostHold::deadline`,
   `boundary`). A boundary release, and a repeat of the deadline, do not. Keys
   come from counters. The product's tag bits and the compat key's XOR of
   generation and token (`slot.rs:2285-2287`) would collide only at a token
   above 2^32.
6. **The modal is one flag.** The host's press check and the script see the same
   modal state: both read the drain of the frame. A page change is a counter.
   An acknowledgement is a page that differs from the page at the send.
7. **The press is the policy.** Energy and the run echo are one event
   (`RunOff`). A session boundary clears the run latch.
8. **Death, guardian hold and prayer-cleanup hold are three flags.** Zero
   hitpoints stalls the owner until the death line. A guardian hold stops the
   clock and the poll. A prayer-cleanup hold stops the poll only.
9. **Caps.** Ticks at most 4, wall at most 5 units, ids 4, pages 4, epochs 2,
   gaps 1 to 2 ticks (the product allows 30), one re-arm. Each run is bounded by
   a horizon in actions, a state cap, a frontier cap and a byte cap.
10. **The JS isolate thread is one poll.** The product posts a snapshot and the
    isolate steps on its own thread. The model steps it in the host's poll.

The model cannot show a defect that needs two leases, more than one tick per
frame, a gap larger than the caps, or the real owner's phase logic. The real
`Dialogue` and `JournalMachine` are not in the conformance test: see below.

## Run it

Every run is bounded, and it runs on chardonnay only, in a memory scope:

```sh
LAB_HOST=chardonnay /Volumes/dev-scratch/274bot-evidence/OVERNIGHT-1027/aubaine.sh run hold-model \
  'cargo build --release -p hold-model && systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 -- ionice -c2 -n7 \
   target/release/hold-model --profile modal --horizon 14 --all-violations'
```

Options: `--profile NAME|all-profiles`, `--mutation NAME`, `--horizon N`,
`--state-cap N`, `--frontier-cap N`, `--byte-cap N`, `--all-violations`. The
exit code is 0 for a complete clean run, 1 for a violation and 3 when a cap
stopped the run. The profiles are `smoke`, `modal`, `quiet`, `compat`, `all`,
`alt` and `handoff` (an owner may begin a read in the frame that revokes the
old one). `--mutation` takes `none`, the seven `check-*` self-checks, the
earlier designs `m1-hold-two-ticks` to `m6-compat-no-renew` and
`f1-renewal-keeps-release` (the host at `0a3e700f8`, before the fix of F1).

## Findings at the head

| | State |
|---|---|
| F1 | Fixed. The host ended the acknowledgement hold, and the end gap that follows is the same lease renewed, so the press went out during the gap. The host now holds a renewal that moves the deadline later (`f1-renewal-keeps-release` is the old host; S1 and S4 find it in 6 steps). |
| F2 | Known limit, S6, profile `quiet`: the journal click loses its hold in its own frame when the lease's wall bound passes between the owner's poll and the host's sample. |
| K1 | A wait that the owner bounds in ticks is not on the active clock. A paused owner loses its end-gap hold. |
| K2 | The terminal release is only as good as the owner's polls. |
| K3 | The compat read never gets a terminal release. |

## Expected results of the profiles

Release build on chardonnay, `--all-violations`, no cap reached (HOLD-MODEL-MERGE,
2026-11-02). "S6 (6)" is finding F2 with a trace of 6 steps. No run shows a
violation of S0 to S5 or L. A run that differs from this table is a finding.

| Profile | Horizon | States | Time | Peak RSS | Result |
|---|---|---|---|---|---|
| `smoke` | 8 | 1 410 765 | 1.3 s | 0.13 GB | clean |
| `modal` | 14 | 48 187 697 | 80.0 s | 4.01 GB | clean |
| `quiet` | 14 | 79 665 442 | 135.3 s | 7.03 GB | S6 (6) |
| `compat` | 14 | 78 047 719 | 121.3 s | 6.93 GB | clean |
| `all` | 11 | 35 451 083 | 51.8 s | 3.46 GB | S6 (6) |
| `alt` | 10 | 16 666 098 | 20.0 s | 1.74 GB | S6 (4) |
| `handoff` | 11 | 40 156 047 | 50.3 s | 3.58 GB | S6 (6) |

## Where the runs go

- The model's tests and both conformance tests run in the existing lanes. CI
  `cargo test --workspace` and the Windows nightly's `test-workspace` lane build and run
  `hold-model`, the `host-play` library tests (`script_hold_conformance`) and
  the `script-js` library tests (`hold_conformance_tests`). The same host-play
  tests also run in `cargo test -p host-play --no-default-features --lib`.
  Together they take about 27 s (host-play) and 8 s (script-js) after the
  build.
- The full profiles do not belong in those lanes: the longest needs 7 GB and
  2 minutes, and the seven together about 8 minutes. `tools/nightly` has no
  step for them. Its steps (`lanes`, `build`, `panel`, `tui`, `latency`) run
  cargo lanes and live cells, and the Linux nightly box (`malbec`) has 15 GiB.
  Run the profiles on chardonnay, one run at a time in a 16 GB scope, after a
  change to the hold or to its model: the command in "Run it", once for each
  row of the table, with the horizon of the row. Compare the result with the
  table.

Tests: `cargo test -p hold-model`.

## Conformance

Two product tests drive the real code with traces from the model. Each trace
is a shortest trace that ends a lease of one kind for one cause (its page, its
own bound, a drop, Stop, a session change, a death, the host's terminal
release or the lease's wall bound).

- `crates/host-play/src/script_hold_conformance_tests.rs` runs the native
  kinds on the production frame path: `Host::run_client`,
  `script_observe_cached_with_channels`, the real `host::ScriptRunPolicy`,
  the ledger's lease sampling, the owner's revocation, the active clock and
  `apply_auto_run`. After each frame it compares the host's hold and press
  with the model. The owner is a test double that does what the model says
  through the real lease API (`begin_modal_wait`, `renew_modal_wait`,
  `begin_quiet_read`, `renew_quiet_read`). The real `Dialogue` and
  `JournalMachine` phase logic is not driven. One model unit is 150 ms of
  real time; a trace that keeps a quiet lease to its 10 s wall bound runs at
  2.5 s per unit, and at most two such traces run.
- `crates/script-js/src/compat/quest_journal/hold_conformance_tests.rs` runs the
  compat read: the real journal runtime and the real `CompatJournalLease`,
  with `backdate` for wall time. It compares the lease's hold and paint-hide
  state with the model. The host policy for this lease is the host-play
  seam.

`HOLD_CONFORMANCE_TRACES=N` sets the number of traces per release (the default
is 3 for host-play and 6 for script-js). A run with 200 covers all the shortest
traces of every native release except the page release of the quiet read.
`the_renewed_end_gap_is_held_on_the_real_code_where_the_earlier_host_pressed`
runs the counterexample of the earlier host (finding F1, mutation
`f1-renewal-keeps-release`) on the real host code. The real code must hold the
renewed wait like the head model. The test fails on host code that has the
earlier behaviour. The script-js test does not reach the host's policy, so a
change of the host cannot fail it; the host policy for the compat lease is
trivial (K3).

## When the hold changes

1. Change the transition that matches the product code in `model.rs`. The
   comments name the product function.
2. If a property changed, change the invariant and say why in the commit.
3. Run the profiles. A new finding is a product defect, a model defect or a
   known limit. Write each in the findings test.
4. Run the conformance test. It drives the product code with traces from the
   model and compares the press and hold decisions at every frame.
