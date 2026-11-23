# hostlog-model

A checked model of the host stderr queue (ADR 0017): the reservation counter,
concurrent producers, the receiver, the shutdown sender clones and the
shutdown wait. It is a developer tool. No product crate depends on it, except
as a dev-dependency of the conformance tests in `crates/api`
(`src/hostlog_conformance_tests.rs`).

`explorer.rs` is a copy of the generic breadth first search in
`tools/js-teardown-model` (copied rather than shared, as ADR 0017 point 1
describes). A run is bounded by a horizon and by state, frontier and byte caps
(`main.rs` has defaults). The search returns the shortest trace of a violation.

## Product code and model actions

| Model action | Product function (`crates/api/src/hostlog.rs`) |
|---|---|
| `Acquire` (`busy`: the try-lock fails) | `StderrOutput::sender`, step `Start` of `Enqueue` |
| `Measure` (`lines` 0: too many lines) | `measure_entry`, step `Cloned`: a pass that allocates nothing and touches no shared state |
| `Reserve` | `Counters::reserve` through `StderrOutput::reserve`, step `Measured`: all the lines and the entry in one compare-and-swap of the state word |
| `Format` (`ok` false: the second pass differs) | `format_entry`, step `Reserved`: allocates only inside the reserved lines |
| `Send` | `SyncSender::try_send` and `Reservation::commit`, step `Held` |
| `Recv`, `WorkerExit` | `Receiver::recv` in `run_stderr_writer` |
| `Finish` | the sink write ends, `Counters::finish` |
| `Close` | `StderrOutput::close` |
| `Settle` | the wait sees the writer thread end and joins it |
| `GiveUp` | `StderrOutput::give_up`: `Counters::close_admission` sets the closed flag and reads `pending` in one atomic operation |
| `Exit` | the process exits; producers act freely between `GiveUp` and `Exit` |

Each action is one atomic step: one try-lock, one compare-and-swap of the
state word, one send, one receive, or a pass without shared state. Lines,
pending entries and the closed flag are one atomic word in the product, so
the model treats a reservation and a close as single steps. The mutation
`half-reserve` models the first design (two separate atomic operations) and is
caught. A producer is the phase machine `Enqueue`; the model uses the same
phases, so the conformance test can compare them.

## Invariants

| Name | Meaning |
|---|---|
| `bound` | Lines reserved, queued or being written never exceed the cap. |
| `lines-conserved` | The line counter equals the lines producers hold, the channel holds and the writer holds. |
| `pending-conserved` | The pending counter equals the entries in those three places. |
| `drop-uncounted` | The drop counter equals refused plus failed plus given-up entries (the model keeps its own books). |
| `send-found-full` | `try_send` never meets a full channel: the reservation guarantees room. |
| `exit-silent-loss` | At exit, every accepted entry is written, failed (counted) or given up (counted). This includes entries accepted between `GiveUp` and `Exit`: a producer that reserves after the give-up snapshot is refused. A clean shutdown leaves none pending. |
| `stuck` | No state exists where nothing can act and the process has not exited: producers never wait, and shutdown ends with a stalled sink. |

## Mutations

`Mutation::ALL` lists each defect that the invariants must keep catching, with
the invariant that catches it (`Mutation::caught_by`). `tests/model.rs` fails
when a mutation is not caught. They include the first designs of the port
(`release-at-recv`, `shutdown-no-wait`, `half-reserve`) and the review findings
(`give-up-keeps-admission`).

## Profiles and expected results

`writable`, `flaky`, `stalled`, `tight`, `three` (see `profiles()`). Each ends
exhausted and clean at the head, well before its horizon.

```
cargo run --release -p hostlog-model -- --profile all-profiles
cargo run --release -p hostlog-model -- --profile all-profiles --mutation release-at-recv
```

## Abstractions

1. The channel holds the line count of each entry, not its text.
2. Time is not modelled. `GiveUp` is enabled whenever the writer has not
   ended: the deadline may pass at any point, even with a working sink, so
   the model proves the counted-loss contract for slow sinks too. The count
   is "at most": an entry given up may still be written, or may be counted
   twice if a producer that reserved before the snapshot then fails.
3. A stalled sink never recovers. After `Exit` nothing runs, as in a process
   that has ended.
4. The sender lock is held only inside one atomic step, so its contention is
   the nondeterministic `busy` outcome of `Acquire`.
5. An entry has one of three sizes: one line, two lines, or too many lines
   (refused whole at `Measure`). Sizes of 3 to 16 lines use the same
   `Reserve` step with a larger count.
