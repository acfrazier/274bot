# Direct Heaptrack capture seam

Status: tooling implementation only. No frontend, server, cache, profile, or native run was started by this task.

## Changed semantics

`run_diagnostic.py` now accepts `--heaptrack-output` only for the exact `tui 1 active --no-diagnostics --sustain` real-terminal mode. It rejects N16/panel/idle/headless and all other diagnostic probes before launch. The option validates the exact Linux preload path, version metadata, and SHA-256, then creates one unused operator-owned mode-0700 directory. The raw path is fixed to `alloc.raw`; it is never reused or followed through a symlink and is checked as an owned mode-0600 regular file after exit.

The frontend remains a direct `Popen([binary])` child. Only that child receives `LD_PRELOAD` and `DUMP_HEAPTRACK_OUTPUT`; no wrapper, FIFO, shell, attach, or collection-time interpreter is used. The child pre-exec setup applies umask 077 and an inherited-limit-preserving `RLIMIT_FSIZE` cap of at most 3 GiB. A 0.5-second guard records and enforces frontend total RSS >512 MiB, MemAvailable <128 MiB, or raw size >=2 GiB, terminating only the owned frontend and preserving partial output.

After frontend exit, the helper runs exactly two sequential, shell-free commands: `heaptrack_interpret` with raw stdin to `alloc.interpreted`, then `heaptrack_print --merge-backtraces=0 --flamegraph-cost-type=peak --print-flamegraph peak-stacks.txt alloc.interpreted` to `peak-analysis.log`. Each child has a 180-second timeout, RSS ceiling 512 MiB, child-only address-space cap 768 MiB, and output file cap (3 GiB interpreter, 512 MiB printer). Nonzero exit, timeout, bound breach, missing/corrupt output, or cleanup failure remains a failed attempt.

`run_current_tui_calibration.py` adds the explicit output option to the predeclared diagnostic argv/spec and records preload metadata. `run_managed_cell.py` accepts an explicit child environment, so the controller no longer clears or mutates its own process environment. Existing disabled/normal, Windows, N16, and panel paths retain their prior argv/environment behavior.

## Verification

- `python3 -m unittest test_heaptrack_capture.py test_run_diagnostic.py`: 30 passed.
- `python3 -m unittest test_current_tui_calibration.py`: 24 passed.
- `python3 -m unittest test_run_managed_cell.py`: 41 passed.
- `python3 -m py_compile heaptrack_capture.py run_diagnostic.py run_current_tui_calibration.py run_managed_cell.py`: passed.
- Full memory Python discovery: 525 passed, 3 skipped, 1 pre-existing failure in `test_resource_screen.py` expecting the historical reason `resource_side_unavailable` while current code returns `overhead_resource_screen_unavailable`; this task does not touch that resource-screen code.
- `git diff --check`: passed.

The native smoke under `diagnostics/heaptrack-direct-smoke-1857` remains the provenance anchor for the exact preload and printer procedure. Root must still stage this reviewed tooling bundle separately on Concord, recheck frozen identity and installed tool facts, and perform the single authorized N1 capture. No performance, RSS ownership, phase-slice, snapshot-equality, or CPU claim is made here.
