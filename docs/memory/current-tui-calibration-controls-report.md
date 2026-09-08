# Current native TUI N16 calibration controls

Scope

`run_current_tui_calibration.py` prepares exactly one current-source Linux native
`tui-play` N16 active diagnostic. It is a thin controller over the reviewed
`run_managed_cell.py` and `run_diagnostic.py`; it does not build, launch, stop,
or mutate the game server, and it never retries a cell. No native run or remote
mutation was performed for this preparation task.

Frozen contract

- `tui 16 active`, real PTY path, `--no-diagnostics`, `--sustain`, warmup 120 s,
  observation 600 s.
- `memory-profile-no-alloc` is verified through the supplied build manifest;
  `snapshot-dedup` is not enabled.
- Scheduling, responsiveness, fine responsiveness, render, GPU completion,
  stack logging, debug, CPU fallback, and TUI input probes are disabled.
- All 16 active Thiever fixture/progress rules remain owned by the existing Rust
  harness; this controller does not adapt or rewrite schemas or fixtures.
- External roles are separate: explicit server PID/start identity, SSH parent,
  managed launcher, controller, and collector. Server identity must declare
  loopback port 43594 and real 64-hex config, fixture, and public-key hashes.
- The predeclared memory guard is `MemAvailable < 128 MiB`, polled every 0.5 s.
  A trigger marks the durable result failed and records the owned-cleanup path;
  no server or ambient PID is signaled.
- Missing cadence, p99, and responsiveness data remain unavailable by design;
  unmeasured collector/instrumentation overhead is a limitation, not an RSS
  qualification failure. Every report sets `performance_acceptance: false`.

CLI inputs

The runner requires explicit `--host-checkout`, `--binary`, `--build-manifest`,
`--build-role`, `--server-root`, `--rs2b0t`, `--nav-pack`, `--nav-flags`,
`--catalog`, `--cache-dir`, `--unpack-root`, `--host-conditions`, `--server-identity`, `--server-pid`,
`--server-start-identity`, `--ssh-parent-pid`, and unique `--output`. Expected
host/client commits default to the current declaration and can be overridden
only explicitly. The real operator paths described by the plan should be passed
by root; no future paths, hashes, receipts, or credentials are fabricated.

`--preflight-only` validates source cleanliness/commits, server declaration,
manifest and runtime fixture bindings, and emits a JSON no-launch contract. It
creates no spec, cell directory, or output reservation. A real run refuses any
existing output/spec/cell path and writes a failed or completed artifact without
turning it into acceptance evidence.

Verification

`python3 -m unittest docs/memory/test_current_tui_calibration.py -v` — 7 tests
passed. `python3 -m py_compile` passed for both Python files. Tests cover exact
flags/spec, hot-flag scrubbing, source mismatch before launch, server identity
hash/port validation, no-launch preflight behavior, memory-guard cleanup
callback, output collision behavior, and incomplete native state remaining
failed/unaccepted.

Root-owned next steps

Root must supply and independently freeze the actual current host checkout,
immutable binary/build manifest, cache/catalog/nav paths, server identity and
host conditions, SSH parent identity, and unique output. Root then performs the
native preflight and sole live attempt after review. This control set does not
claim that the native screen has run or qualified.
