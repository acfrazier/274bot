# Bounded game-server resource sidecar

This deliverable adds `server_resources.py`, a standalone finite-duration sampler for a game-server PID supplied by the caller. It does not hunt for a process, start/reset/stop one, or capture command-line arguments or environment variables. The target is the explicit root PID only; child processes are not silently included.

## Interface

```text
python3 docs/memory/server_resources.py SERVER_PID OUTPUT.jsonl \
  --interval 1 --duration 120
```

The default interval is one second. Duration is required and must be finite and positive. Output is created exclusively and is never overwritten unless the operator explicitly passes `--force`. A failure exits 1, prints `FAIL: ...` to stderr, and leaves metadata plus any samples and an error record in the JSONL output.

Each successful sample records:

- UTC wall timestamp and `time.monotonic()` timestamp/elapsed time;
- scheduled deadline, acquisition start/end/duration, and lateness;
- current resident bytes (not a peak);
- cumulative user/system CPU seconds;
- interval CPU deltas and `cores_delta = delta CPU seconds / monotonic elapsed seconds`;
- process identity, state where available, and exact OS/process source descriptions;
- host pressure in a separate field.

Linux process data uses `/proc/PID/stat` (utime, stime, starttime, and field-24 RSS pages) and `SC_PAGE_SIZE`, with `SC_CLK_TCK` conversion. Field-24 RSS is a current resident-set approximation read from the same stat record as process identity, avoiding a cross-file PID-reuse mix-up. Linux host pressure uses `/proc/pressure/memory`: PSI `avg10`, `avg60`, and `avg300` are percentages of wall time stalled over their named windows, while `total` is microseconds. Missing/unreadable required process files, CPU counter resets, PID start-identity changes, and invalid PIDs fail the run. A changed start identity is treated as PID exit/reuse, not as a continuation.

macOS process data uses `ps` against the explicit PID only: RSS KiB, cumulative `utime`/`stime`, and `lstart` as the process start identity. Each `ps` read is bounded by the remaining finite run duration; a timeout fails the run while retaining partial JSONL output. `lstart` has one-second resolution, so PID reuse within the same second cannot be ruled out. macOS pressure is currently reported as `unavailable` rather than a fabricated zero/healthy value because this sidecar does not select a portable counter with equivalent PSI semantics.

Sampling uses fixed monotonic deadlines rather than sleeping after acquisition. The default one-second interval permits 0.25 seconds of cadence tolerance (bounded to 0.05--0.5 seconds for other intervals); an overdue or overlong acquisition fails instead of claiming complete sampling. The finite duration remains the hard subprocess timeout budget.

Sampling overhead is **unmeasured**. Sidecar output is diagnostic evidence and does not establish memory savings, a server budget, non-starvation, or performance acceptance.

## Verification

```text
python3 docs/memory/test_server_resources.py
```

The test suite covers parenthesized Linux `/proc` command parsing, same-record RSS parsing, PSI semantics and missing fields, invalid PID, PID identity change, CPU counter reset, missing process samples, unsafe overwrite, bounded macOS `ps`, stalled cadence, and a short controlled self-process smoke. The next clean integration use is to attach to the existing engine PID supplied by the existing bounded harness and keep the sidecar records separate from client resource records; do not start, reset, or discover a server here, and do not run acceptance measurements concurrently with builds, tests, or other profilers.
