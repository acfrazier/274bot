# Managed diagnostic cell runner

`python3 docs/memory/run_managed_cell.py SPEC.json CELLS_ROOT` executes one
predeclared cell with no retry. It reserves the output directory exclusively,
validates the frozen build, checks the explicit server PID/start identity, and
writes launch and completion receipts. It never starts/stops the game server.

The spec names binary/build manifest/role, frontend, exact launcher and diagnostic
argument lists, server and host sidecars, nav/catalog paths, explicit server and
ambient PIDs, sampling interval, and a finite external maximum wall time. IDs are
safe single path components. Reserved roles and duplicate PIDs fail before launch.
The actual argv must invoke this `run_diagnostic.py` with the declared arguments;
optional observation/warmup values must agree. Arbitrary test launchers and
accounting scripts are programmatic test injection only, unavailable in the CLI.

`process_backend` defaults to `system`; `libproc` explicitly selects the reviewed
macOS native sampler for preflight, frontend start identity, and collection. No
silent fallback occurs. The launch receipt records the selected backend and the
actual accounting script path/hash. Explicit server/controller/launcher/ambient
roles are sampled; the collector includes itself. Host resource samples remain
in the frontend's own raw series.

The runner parses native `observe-start` and `observe-end` qualification rows.
It requires one ordered pair with increasing finite elapsed values and slot
records, and requests collector stop at least two intervals after receiving the
actual end — including when the launcher has already exited with a short
teardown. Login/setup delays cannot trigger an early time-estimated stop.
Missing, malformed, partial, duplicate, or out-of-order boundaries fail the
durable receipt through `runner_errors`, preserving actual child exit codes.
The pad-stop runner_error is emitted only when observe-end arrived but the
two-interval hold did not complete; missing end uses the boundary failure only.

Contract note on short teardown vs completed receipts: production frontend
teardown is long (order of 60s) relative to the default 1s sampler interval, so
the launcher normally remains a live required role through the post-end pad.
If teardown is shorter than two intervals, the launcher PID dies while the
collector still must sample it. Holding the pad after launcher exit is correct
for wall-clock stop bookkeeping, but continuous required-role evidence then
fails honestly (collector exit non-zero / sampler incomplete). The runner
records `launcher exited before post-observation collector coverage` and keeps
a failed measurement receipt with no retry. It does not drop the launcher role,
substitute another PID, extend launcher lifetime, or rewrite collector failure
into success. A completed receipt in that situation would falsify continuous
required-role coverage. The matched reader independently validates full
workload and slot contents.

Cleanup reaps owned launcher/collector children and independently checks the
captured frontend even if its launcher already exited. Frontend ownership is
established from the explicit child's parent PID while it is alive; its start
identity is rechecked immediately before TERM and KILL. Each signal stage has a
bounded 15-second wait. Identity mismatch/unavailability prevents signaling and
reports unresolved cleanup. Server/ambient/controller PIDs are never adopted as
frontends. Exceptions after launch, including output collisions, clean up owned
children, record the same `cleanup` structure as the happy path, and preserve a
failed receipt where writable. Existing output is never overwritten.

Validation: the full dummy-process suite (18 tests) covers real delayed
qualification, short-teardown early-launcher-exit failure with pad hold,
absent/duplicate boundaries, launcher exit leaving a TERM-ignoring frontend,
identity mismatch without foreign signaling, native backend identity
consistency, finite timing/reserved roles/actual argv, exclusive output, failed
launch/collector, distinct child exits, and preflight failures. No live
bot/server, release build, or performance measurement in these tests.

Cache fingerprint and continuous process-evidence consumption remain separate
integration work. A completed receipt does not establish overhead or performance
acceptance. The collector's controlled stop is not itself full window coverage.
