# Auto-run hold model (TLA+)

A checked model of the host's auto-run hold, written in TLA+ and checked with TLC.
It models the product at commit `178a002005a62b49b650544653bae9f8eca9ac62`
(branch `nav-prereq-closure`). It is arm T of the HOLD-MODEL-AB comparison.

The model has three phases: the model and its head results, the earlier designs as mutations (phase 2), and
conformance with traces of the real code (phase 3).

The model covers the wait lease of a script (the `Dialogue` modal wait, the native
quest-journal quiet read, the compatibility quest-journal read), the host's
`ScriptRunPolicy` hold, and one host frame: drain, slot sync, owner poll, lease
sample, hold update, press. `HoldModel.tla` has the full list of stages with
`file:line` references, and the list of abstractions (A1 to A8) with the reason for
each.

## Files

| File | Purpose |
| --- | --- |
| `HoldModel.tla` | The specification, the invariants S1 to S4, LB, and the temporal property `HoldEnds`. |
| `head-*.cfg` | Runs of the product at the head commit. |
| `fix-*.cfg` | Runs of `Mut = "FixRearm"`, the proposed fix of defect D1. |
| `sanity-*.cfg` | Runs of mutations that show an invariant can fail. |
| `live-core.cfg` | The liveness run (`FairSpec`, property `HoldEnds`). |
| `mut-*.cfg`, `phase2.py` | Phase 2: the earlier designs M1 to M6 (see below). `phase2.py <dir>` writes one config per invariant. |
| `HoldTrace.tla`, `HoldHostTrace.tla` | Phase 3: TLA+ trace validation (see below). |
| `tracegen.py`, `livegen.py`, `tracerun.sh` | Phase 3: turn test traces and live logs into TLC inputs, and run them. |
| `trace_summary.py` | Prints a TLC counterexample as one line per step, in product terms. |

## Tools

TLC from the official release `v1.7.4` (TLC 2.19 of 08 August 2024):

```
https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar
sha256 936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
```

Java 11 or later (the lab host has OpenJDK 17 at `/usr/bin/java`). Nothing else is installed.

## Commands

Run only on the Linux lab host (chardonnay), never on merlot. Every run has a heap cap, a
worker cap, a wall-time stop and a memory scope. The state space is bounded by the
`Bound` constraint and by the constants in the `.cfg` file; a run without a bound is not allowed.

One configuration, from this directory, with `JAR` set to the jar path:

```
systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 -- ionice -c2 -n7 \
  java -Xmx12g -XX:+UseParallelGC -Dtlc2.TLC.stopAfter=1800 -cp "$JAR" tlc2.TLC \
  -workers 8 -config head-core.cfg -metadir /tmp/states-head-core HoldModel.tla
```

`-Dtlc2.TLC.stopAfter` stops state exploration after that many seconds. A run that stops at
that limit prints "Model checking completed" although states are left on the queue. Read the
line `N states left on queue`: it must be `0`. The temporal-property check
(`live-core.cfg`) runs after the exploration and is not covered by the limit.

Read a counterexample:

```
python3 trace_summary.py head-core-s1.out
```

## Properties

Each invariant is a state predicate in `HoldModel.tla`. None of them reads the mutation name `Mut`.

| Name | In plain words |
| --- | --- |
| S1 | No press goes out while a lease is live and its owner's own deadline, in the owner's own clock, has not passed. A relog and a Stop revoke the lease, and a death makes the owner drop it, so such a lease is no longer live. |
| S2Stop | After a Stop, a hold of an old transaction suppresses at most 1 more press. The host updates its hold after the slot lock is released, so a sample taken before the Stop is applied after it. |
| S2Bnd | After a session change, a hold of an old transaction suppresses no press: `after_drain` releases it at the drain. |
| S2Dead | After a death message, a hold of an old transaction suppresses no press on a frame where the owner polls. |
| S2DeadAny | The same, counted on every frame. It is not an invariant of the product: a Paused, held or hung owner keeps its lease. `head-death-any.cfg` shows this as a known limit. |
| S3 | The host releases without the script only as the logged terminal failure, at least 1 tick after the lease's deadline, and only when the owner's own deadline has passed in the owner's own clock. |
| S4a | The host hold's key and the sampled lease's key name the same transaction. |
| S4b | A released hold on a lease that the sampler still reports belongs to a deadline that has passed. Stated where the host view is fresh (after its update, before the next poll). |
| LB | An effective hold on a lease whose stored bound is in ticks does not outlive that bound plus the margin. |
| HoldEnds | If the owner's clock keeps running, every effective hold eventually ends. Checked with `FairSpec`. |

### Fairness for HoldEnds

`FairSpec` has weak fairness for the six frame stages (the loop runs). It has strong fairness
for time passing (`WallAdv`, `ActAdv`), for ticks (`TickArrive`), for `Resume` and for `Unhold`,
because each of these is enabled only at some stages, so weak fairness would not force them.
It does not force Stop, Hang, relog or death, and it does not force the owner to end a wait: the
owner's obligations are in `OwnerOpts`. The property has the antecedent `<>[]Eligible`: an
active-clock lease needs the owner's clock to run, and a paused owner keeps its hold on purpose
(`docs/api/nav.md`, the paragraph on the hold). The liveness run turns on `Room` (a lease starts
only with room to reach its bounds) and `Wrap` (at `MaxTick` a tick slides every stored tick down by
1, so ticks are unbounded in effect with a finite state space).

## Bounds

All clock values are small naturals (abstraction A1). The code compares them only with `>=`,
`<` and a margin of 1, so the order relations are all that matter.

| Constant | Safety runs | Code value |
| --- | --- | --- |
| `MaxTick`, `MaxWall`, `MaxAct` | 5, 4, 3 | not bounded |
| `Margin` | 1 | `SCRIPT_WAIT_MARGIN_TICKS` = 1 |
| `HB` (compat hold), `QW` (quiet wall bound) | 1, 2 | 10 s each |
| `AckB` (page/ack wait, active clock) | 1 | 3 s, 8 s |
| `GapT` (end gap), `WinT` (journal window) | 1, 1 | up to 30, 5 |
| `MaxToggles` (Pause or freeze changes) | 1 | not bounded |
| `MaxTxn` (lease begins) | 2 | not bounded |
| `MaxRelogs`, `MaxStops`, `MaxDeaths` | 0 or 1 each, one class per run | not bounded |

The classes are split because their product is large. `head-core` has no relog, Stop or death;
`head-relog`, `head-stop` and `head-death` have one of each. A defect that needs two of them at
once is outside these runs. The bounds are small on purpose: a defect that needs a gap of 30 ticks,
or three lease begins, is outside them. The deadline arithmetic is the same at larger values; the
liveness run and the conformance runs (phase 3) are the checks on that claim.

## Mutations

`Mut` selects a variant of the transitions. `head` is the product. The invariants do not change.
The variants are listed in the header of `HoldModel.tla`. Phase 1 has the sanity variants
(`NoBoundary`, `StopNoRevoke`, `KeyReuseModal`, `NoTerminal` and `Margin = 0`) and the fix `FixRearm`.
Two variants are equivalent mutants: `NoSessionRelease` and `StopKeeps` remove a release that the
product does twice (the slot also revokes the lease), so no invariant fails.

## Results

The table is from the final run of the phase-1 model on the lab host (32 cores, 8 workers, 12 GB heap).
A violated invariant stops the run at the first counterexample, so the state counts of those rows are partial.
`Left on queue` is `0` for every run that finished the whole space.

| Config | Result | Distinct states | States generated | Left on queue | Depth | Trace (states) | Wall (s) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `head-core` | S4b violated (defect D1) | 2088986 | 4709651 | 382910 | 33 | 29 | 10 |
| `head-core-s1` | S1 violated (defect D1) | 2536281 | 5830095 | 431048 | 35 | 30 | 10 |
| `head-core-rest` | holds | 12081836 | 33344787 | 0 | 64 | - | 57 |
| `head-relog` | holds | 16807642 | 69833582 | 0 | 68 | - | 107 |
| `head-stop` | holds | 45721669 | 129818152 | 0 | 66 | - | 230 |
| `head-death` | holds | 55501219 | 160368484 | 0 | 82 | - | 344 |
| `head-death-any` | S2DeadAny violated (known limit) | 46475 | 100148 | 16201 | 20 | 14 | 1 |
| `fix-core` | holds | 12060326 | 33301317 | 0 | 64 | - | 78 |
| `fix-relog` | holds | 16782517 | 69755177 | 0 | 70 | - | 116 |
| `fix-stop` | holds | 45663799 | 129701586 | 0 | 66 | - | 288 |
| `fix-death` | holds | 55472794 | 160282039 | 0 | 76 | - | 376 |
| `live-core` | HoldEnds holds | 8681913 | 27266029 | 0 | 85 | - | 2333 |
| `sanity-noboundary` | S2Bnd violated (expected) | 60860 | 156816 | 21985 | 21 | 14 | 2 |
| `sanity-stopnorevoke` | S2Stop violated (expected) | 105151 | 199502 | 40290 | 20 | 15 | 1 |
| `sanity-keyreuse` | S4a violated (expected) | 9446 | 15406 | 3485 | 18 | 11 | 1 |
| `sanity-margin0` | S3 violated (expected) | 1319 | 1995 | 523 | 13 | 7 | 1 |
| `sanity-live-noterminal` | HoldEnds violated (expected) | 341099 | 950693 | 0 | - | 52 | 30 |
| `sanity-nosessionrelease` | holds (equivalent mutant) | 16820392 | 69851951 | 0 | 70 | - | 105 |
| `sanity-stopkeeps` | holds (equivalent mutant) | 45721669 | 129818152 | 0 | 66 | - | 298 |

## Phase 2: earlier designs of the hold

`Mut` takes the earlier designs as transition-only variants (the invariants S1 to S4 and LB are
unchanged). Each is a configuration in `mut-*.cfg` (all core invariants) and, one invariant per
run, from `phase2.py <dir>`. The model's time scale is small, so "25 observed ticks" is 2 ticks (the
longest gap is 3), keeping "limit shorter than the longest legal wait".

| Name | Design | Where in `HoldModel.tla` |
| --- | --- | --- |
| `M1` | Round 2: no leases. The host ends the hold when a modal opens or 2 observed ticks after the click. | `NoLeaseHost` in `Update` |
| `M2` | Round 3: as `M1` with 3 ticks. | `M1Ticks` |
| `M3` | Round 4: leases, but the terminal release is a fixed limit counted from the first sample of the key. | `FixedHost`, `M3Limit` |
| `M4` | Round 4: `M3` plus one constant key for every compat lease. A compat read may end and the next begin in one poll. | `ConstC`, `BackToBackC` |
| `M5` | Round 5: the active-clock bound becomes a wall deadline once, when the bound is installed. | `WallConv` in `SampleResult`, `wdl` |
| `M6` | Round 5: the compat hold has one bound from the begin and is not renewed at the click. | `NoClickRenew` in `COpts` |
| `BackToBackC` | Head plus a compat read that ends and the next begins in one poll. Needed by `M4`; head must still pass. | `COpts` |
| `*+Fix` | A mutation on top of `FixRearm`. Used so that defect D1 does not hide or fake a result. | `FixOn` |

Because head has defect D1 (S1 and S4b fail at 29 to 30 states), a mutation is judged on the `+Fix`
base when D1 would give the same verdict. Results (`caught` = a frozen invariant fails; the trace is
in model states):

| Mutation | S1 | S3 | S4a | S4b | LB | S5Click (extra) | S2Stop | S2Bnd | S2Dead |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `BackToBackC+Fix` | pass | pass | pass | pass | pass | pass | not run | not run | not run |
| `M1` | **14** st | pass | pass | **7** st | **18** st | pass | not run | not run | not run |
| `M2` | **14** st | pass | pass | **7** st | **18** st | pass | not run | not run | not run |
| `M3` | **22** st | **21** st | pass | **21** st | **18** st | pass | pass | pass | pass |
| `M4` | **22** st | **21** st | **17** st | **21** st | **18** st | pass | not run | not run | not run |
| `M5` | **16** st | **15** st | pass | **15** st | pass | pass | pass | pass | pass |
| `M5+Fix` | **16** st | **15** st | pass | **15** st | pass | pass | not run | not run | not run |
| `M6` | **30** st | pass | pass | **29** st | pass | **12** st | pass | pass | pass |
| `M6+Fix` | pass | pass | pass | pass | pass | **12** st | not run | not run | not run |

A bold number is the length of the shortest counterexample in model states (a run stops at the first). States explored and wall time of every run: `runs-p2/` in the evidence root; each caught run explored 1,481 to 2,378,257 distinct states and took 1 to 10 s; each passing core run explored 12 to 69 million distinct states in 56 to 444 s.

Reading it:

- `M1`, `M2`: caught by S1 (14 states), S4b (7) and LB (18). S3 and S4a do not apply: with no
  leases there is no terminal release and no key.
- `M3`: caught by S3 (21), S4b (21), S1 (22) and LB (18). S3 is the property that sees a release
  before the owner's own deadline. S4a passes: the keys are still unique.
- `M4`: as `M3`, and S4a (17) catches the constant key.
- `M5`: caught by S3 (15), S4b (15), S1 (16). Its `+Fix` run gives the same. LB passes: the
  stored bound is still in ticks. The release comes while the owner is paused and its own deadline
  has not passed, which S3 states.
- `M6`: **not caught by the frozen invariants** once D1 is removed (`m6f`: all pass). On head, S1 and
  S4b fail at 30 and 29 states: that is D1, the same trace as head, not M6. Why: the model has no
  "send" event apart from the lease phase, and S1 to S4 speak only of presses. A compat click that
  goes out with its hold already lapsed does not change a press. The extra invariant `S5Click` (added
  in phase 2, not frozen) states it and catches `M6` and `M6+Fix` at 12 states. The abstraction
  that hides it: the click and the page are one lease phase, not two events.
- `BackToBackC+Fix` (head plus the back-to-back read, with the fix): all pass.

S2 (Stop, relog, death) runs are not repeated for `M1`, `M2`, `M4` and the `+Fix` variants: the
mutations do not touch the Stop, relog or death transitions, and the `M1`/`M2` state space is
3 to 4 times larger. `M3`, `M5` and `M6` pass `S2Stop`, `S2Bnd` and `S2Dead`.

## Phase 3: conformance (trace validation)

`HoldTrace.tla` and `HoldHostTrace.tla` check the model against traces of the real code. TLC accepts a
trace only if the model has a behaviour that reaches its end with every recorded output equal to the
model's output. `Accepted` is `Invariant NotDone is violated`. Bookkeeping steps change only the trace
variables; the model's own actions (`TickArrive`, `Drain`, `Obs1`, `Obs2`, `Update`, `Press`, `Pause`,
`Resume`, `Stop`, `WallAdv`, `ActAdv`) are the other steps. What is input, not a model step, is stated at
the top of each module: the owner's lease events in a rig trace, the sample in a host trace.

Sources (all from the product's own tests, with trace output behind `cfg(test)` only; product
behaviour is unchanged):

| Source | Trace | Real code that runs |
| --- | --- | --- |
| `crates/host-play/src/script_progress_tests.rs`, `HoldRig` | `holdrig-*.jsonl`, one line per frame | `Host::run_client`, `script_observe_cached`, a real slot with Pause, Resume and Stop, a real modal-wait lease from `begin_modal_wait` |
| `crates/host/src/lib_tests.rs` (and the same call sites) | `host-*.jsonl` | `observe_script_wait`, `sync_runtime`, `release_at_session_boundary`, `apply_auto_run` |
| `compat_journal_read_to_read_hand_off_takes_a_fresh_hold` | `compat-*.jsonl` | the slot's `script_interaction_wait` with compat journal waits |
| `RELEASE-029-NAV-RUNNER/closure-r6` live cells (`BOT_DEBUG=1` logs) | `host-live-*.jsonl` by `livegen.py` | the host's own log lines (no fresh live run was needed) |

Record, convert, check (on the Linux lab host; `isohome` is the repo's throwaway-HOME wrapper):

```
HOLD_TRACE_DIR=/tmp/t isohome cargo test -p host --lib -- --test-threads=1 script_wait hung_owner renewed_wait session_change stopped_runtime thirty_tick d1_ auto_run_
HOLD_TRACE_DIR=/tmp/t isohome cargo test -p host-play --lib -- --test-threads=1 auto_run_hold compat_journal_read_to_read
python3 tracegen.py rig /tmp/t /tmp/tla && python3 tracegen.py host /tmp/t /tmp/tla
cp HoldModel.tla HoldTrace.tla HoldHostTrace.tla /tmp/tla/ && JAR=$JAR ./tracerun.sh /tmp/tla $(cd /tmp/tla && ls *.cfg | sed 's/.cfg//')
```

| Trace | Verdict | States |
| --- | --- | ---: |
| `H_compat_altered_S4_constant_key` | S4h-VIOLATED | 6 |
| `H_compat_tests_script_progress_compat_journal_read_to_read_han` | ACCEPTED | 87 |
| `H_host_live_L_host_script_compat_journal_twice_1` | ACCEPTED | 17 |
| `H_host_live_L_host_script_compat_journal_twice_2` | ACCEPTED | 17 |
| `H_host_live_L_host_script_compat_journal_twice_3` | ACCEPTED | 17 |
| `H_host_live_L_host_script_progress_journal_1` | ACCEPTED | 23 |
| `H_host_live_L_host_script_progress_journal_2` | ACCEPTED | 23 |
| `H_host_live_L_host_script_progress_journal_3` | ACCEPTED | 23 |
| `H_host_live_Q_kitchen_1` | ACCEPTED | 13 |
| `H_host_live_Q_kitchen_2` | ACCEPTED | 13 |
| `H_host_live_Q_kitchen_3` | ACCEPTED | 13 |
| `H_host_tests_a_hung_owner_is_released_after_its_deadline_plus_` | ACCEPTED | 51 |
| `H_host_tests_a_renewed_wait_moves_its_deadline` | ACCEPTED | 87 |
| `H_host_tests_a_session_change_releases_the_script_wait` | ACCEPTED | 31 |
| `H_host_tests_a_stopped_runtime_releases_the_script_wait` | REJECTED | 8 |
| `H_host_tests_a_thirty_tick_quiet_end_gap_keeps_the_hold_to_its` | ACCEPTED | 127 |
| `H_host_tests_auto_run_20_0_20_sends_twice` | ACCEPTED | 7 |
| `H_host_tests_auto_run_does_not_send_on_the_title` | ACCEPTED | 5 |
| `H_host_tests_auto_run_waits_for_the_open_bank_to_close` | ACCEPTED | 13 |
| `H_host_tests_auto_run_yields_to_script_action_and_recurs_after` | ACCEPTED | 40 |
| `H_host_tests_d1_a_renewed_wait_after_the_terminal_release_stay` | ACCEPTED | 15 |
| `H_host_tests_script_wait_holds_the_press_through_any_packet_or` | ACCEPTED | 37 |
| `R_altered_S1_press_through_the_hold_0` | REJECTED | 57 |
| `R_auto_run_hold_ends_when_the_script_is_stopped_0` | ACCEPTED | 2204 |
| `R_auto_run_hold_ends_when_the_script_is_stopped_1` | ACCEPTED | 1723 |
| `R_auto_run_hold_ends_when_the_script_wait_expires_0` | ACCEPTED | 162975 |
| `R_auto_run_hold_lasts_a_thirty_tick_wait_to_its_deadline_0` | ACCEPTED | 1714179 |
| `R_auto_run_hold_of_a_hung_owner_ends_after_its_deadline_plus_t_0` | ACCEPTED | 166008 |
| `R_auto_run_hold_of_a_paused_acknowledgement_wait_lasts_to_the__0` | ACCEPTED | 16516353 |
| `R_auto_run_hold_of_a_paused_page_wait_lasts_to_the_owners_own__0` | ACCEPTED | 15375029 |
| `R_auto_run_hold_survives_a_modal_in_a_later_drain_than_its_pla_0` | ACCEPTED | 15282 |
| `R_auto_run_hold_survives_full_five_packet_drains_0` | ACCEPTED | 9346 |
| `R_auto_run_hold_survives_queued_updates_before_the_page_0` | ACCEPTED | 41014 |
| `R_auto_run_hold_waits_for_the_page_and_its_close_0` | ACCEPTED | 4013 |

Negative controls (the same real traces, a mutated model; `__M3` uses a limit of 25):

| Trace against model | Verdict | Reason |
| --- | --- | --- |
| thirty-tick wait, `M3` | REJECTED | the fixed limit releases the hold before tick 30 |
| script wait expires (12 ticks), `M3` | ACCEPTED | 12 is under the limit of 25 |
| queued updates before the page, `M1` and `M2` | REJECTED | the hold ends at 2 or 3 ticks |
| modal in a later drain, `M1` | REJECTED | the hold ends at 2 ticks |
| page and close, `M1` | ACCEPTED | the page opens within 2 ticks |
| paused page wait, `M5` | REJECTED | the wall deadline releases the hold during the pause |
| paused acknowledgement wait, `M5` | REJECTED | the same |

Strength and limits:

- Each recorded frame fixes the inputs (tick edge, modal, sleep, Pause, Resume, Stop) and two outputs
  (the hold after the update, and whether the press went out). The rig traces check the host and
  sampling half against real code: the stored deadline, the active clock under Pause, the terminal release
  and its margin, the press decision, and Stop. The model has no slack except `E` = 1 extra time unit per
  frame (one unit is 25 ms: the real frame takes CPU time).
- The owner is input. The test machine is not the `Dialogue`, so the traces do not check the owner's
  transitions (`MOpts`, `QOpts`, `COpts`) against code. D1's owner half (the dialogue renewing the same
  lease from an active-clock bound to its end gap) is read from `dialogue.rs:372-381`, not replayed.
  The host half of D1 is pinned by `d1_a_renewed_wait_after_the_terminal_release_stays_released`
  and accepted by the model (`H_host_tests_d1_...`).
- A model press that the real host did not send is not an error in a host trace: the host also withholds
  a press for title, energy and readiness (HoldModel A4). A press that the host sent and the model forbids
  is an error. The rig traces also check that: a model press without a recorded press is accepted only after
  a recorded press (`run_on`).
- `a_stopped_runtime_releases_the_script_wait` is rejected: the test calls `sync_runtime` and
  `observe_script_wait` in separate places, and the model's `Update` does both in one step, as the
  product's frame does. This is a limit of the model's atomic step, not a product defect.
- The live traces have 6 to 11 events: the host logs only key changes and presses, so the frames between are
  filled by an extractor rule (`livegen.py`). They check order, not timing.
- Negative controls (the same real traces against mutated models) show the check has teeth:
  the 30-tick wait is rejected by `M3` with a limit of 25; the queued-updates traces by `M1` and `M2`; both
  paused-wait traces by `M5`. The short page-and-close trace is accepted by `M1` and the 12-tick expiry
  trace by `M3` with 25, as they should be.
- Two failing cases. `R_altered_S1_press_through_the_hold` is the page trace with the press moved to the
  click frame, through the hold: REJECTED (the model cannot press through a live lease; S1 would be broken).
  `H_compat_altered_S4_constant_key` is the compat hand-off with the second read given the first read's key:
  the model accepts the outputs (the hold stays released) and the host-side invariant `S4h` fails at the
  second sample (S4 broken).

## Findings at the head commit

**D1 (product defect, narrow window): a renewed wait keeps a terminal release.** `head-core-s1.cfg`
gives a 30-state trace. The dialogue sends a Continue and takes a modal-wait lease with an
active-clock bound. The first host sample that sees the bound passed fixes the lease's deadline at
that tick (`native.rs:926-929`). The owner does not poll on the next tick (the script is Paused, or
held, or not ready), so the host releases the hold as the logged terminal failure
(`lib.rs:251-254`). Later the owner polls, sees the acknowledgement, and starts the end gap on the
same lease: the lease keeps its key (the epoch `(steps, rearms)` is unchanged,
`dialogue.rs:372-381`) and its bound moves to ticks. `observe_script_wait` keeps `released` for a
key it has seen (`lib.rs:233`), so the whole end gap runs with no hold, and the press goes out in the
quiet gap. S4b and S1 fail. `fix-*.cfg` (`Mut = "FixRearm"`: a deadline later than the one released
at re-arms the hold) passes every invariant in all four classes.

**Known limit: a Paused, held or hung owner keeps its lease.** `head-death-any.cfg` fails
`S2DeadAny`. After a death message the owner drops its lease at its next poll; while it cannot poll,
the hold stays until the lease's own bound plus the margin. A Paused script with an active-clock lease
keeps the hold for the whole pause (`docs/api/nav.md`, the paragraph on the hold).

**S1 and S2 need exemptions that the brief does not state.** A relog releases the host hold while
the lease is still live in the slot until the slot revokes it in the same frame; a Stop, relog and
death end the lease, so "live" means "not revoked and not dropped". S4b is stated only where the host
view is fresh for the same reason.

**Two releases are redundant.** `NoSessionRelease` and `StopKeeps` do not violate any invariant,
because the slot revokes the lease at the same boundary. This is belt and braces, not a defect.

## How a change to the hold updates the model

1. A new lease kind: add a kind letter, its begin, its `*Opts` set in the owner section, its
   branch in `SampleResult` and in `OwnerPassed`.
2. A new bound or a new margin: change the constant or the expression in `OwnerOpts`/`SampleResult`.
3. A change of order inside the frame: change the stage actions; `Unlocked` and `TimeAt` say where
   the world may change between two stages.
4. Run all `head-*.cfg`, then `live-core.cfg`. A change of behaviour shows as a new counterexample.

## CI

CI does not run this model. A CI lane would need Java 11 or later, the pinned jar (2.2 MB, SHA-256
above), a Linux runner with 8 cores and 12 GB of heap for TLC (peak resident set about 6 GB for the
safety runs), and for the trace runs the `isohome` wrapper and a cargo test run to record the traces.
Measured run time on 8 workers: `head-*` and `fix-*` about 35 minutes together, the liveness run
`live-core` about 39 minutes, the phase 2 core runs about 1 hour (the S2 runs of `M3`, `M5` and `M6`
about 1 hour more), the trace runs about 1 hour (the two paused-wait traces take about 13 minutes each).
A lane for `head-core`, `head-core-s1` and the rig traces other than the two paused ones takes under
5 minutes. No workflow was changed.
