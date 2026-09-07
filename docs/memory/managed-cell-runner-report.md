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
actual end. Login/setup delays cannot trigger an early time-estimated stop.
Missing, malformed, partial, duplicate, or out-of-order boundaries fail the
durable receipt through `runner_errors`, preserving actual child exit codes.
The matched reader independently validates full workload and slot contents.

Cleanup reaps owned launcher/collector children and independently checks the
captured frontend even if its launcher already exited. Frontend ownership is
established from the explicit child's parent PID while it is alive; its start
identity is rechecked immediately before TERM and KILL. Each signal stage has a
bounded 15-second wait. Identity mismatch/unavailability prevents signaling and
reports unresolved cleanup. Server/ambient/controller PIDs are never adopted as
frontends. Exceptions after launch, including output collisions, clean up owned
children and preserve a failed receipt where writable. Existing output is never
overwritten.

Validation: the full 16-test dummy-process suite passed, then an additional
nonobject-server-sidecar preflight test passed (17 tests total). Coverage includes
real delayed qualification, absent/duplicate boundaries, launcher exit leaving
a TERM-ignoring frontend, identity mismatch without foreign signaling, native
backend identity consistency, finite timing/reserved roles/actual argv, exclusive
output, failed launch/collector, distinct child exits, and preflight failures.
No live bot/server, release build, or performance measurement in these tests.

Cache fingerprint and continuous process-evidence consumption remain separate
integration work. A completed receipt does not establish overhead or performance
acceptance. The collector's controlled stop is not itself full window coverage.
