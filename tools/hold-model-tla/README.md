# Auto-run hold model (TLA+)

A checked model of the host's auto-run hold, written in TLA+ and checked with TLC.
It models the product at commit `178a002005a62b49b650544653bae9f8eca9ac62`
(branch `nav-prereq-closure`). It is arm T of the HOLD-MODEL-AB comparison.

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

CI does not run this model. A CI lane would need Java 11 or later, the pinned jar, and about
70 minutes on 8 cores for the full set (the table above gives the measured times). The jar is 2.2 MB.
