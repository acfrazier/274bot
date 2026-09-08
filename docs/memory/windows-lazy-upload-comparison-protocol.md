# Windows N16 lazy-upload paired comparison protocol

Status: prepared controls only. This task performs no Windows command, native
call, network operation, live launch, staging, or build.

## Purpose and fixed scope

This is a bounded diagnostic comparison of the reviewed lazy-upload candidate
against a fresh baseline. It is not a performance-acceptance run and it does
not claim that a diagnostic screen or a falling RSS trend is a saving. The
comparison has four unique cells, run sequentially:

| role | mode | required cell-id shape |
|---|---|---|
| baseline | focused-one | `baseline-focused-one-<unique>` |
| baseline | focused-plus-background | `baseline-focused-plus-background-<unique>` |
| candidate | focused-one | `candidate-focused-one-<unique>` |
| candidate | focused-plus-background | `candidate-focused-plus-background-<unique>` |

Use the same N=16 active workload, existing local server, reusable workload and
cache, Intel(R) Graphics request, 30 s warmup, 120 s observation, and 60 s
teardown for every cell. Use one fresh process and unique cell id per cell. Do
not overwrite an old run or archive. Baseline is a new run; the existing
`36825a9` N16 archives are reference evidence only and are not a baseline for
this comparison.

Both roles use these exact immutable identities:

- baseline host commit `36825a9f0a07a19610439c691f6dfe3d2e1cbf48`, frozen binary
  SHA-256 `87c24665563ef8a55751244a52f7d25c7edf68e3117e84f2ff464add596c71fb`;
- candidate host commit `3118e9661a89906589ee6e0239e475b36228bde3`, candidate
  binary SHA-256
  `1937663506e3f0b243f5a9a742de3ae8d86ded66867bf2db38f9f882ec76e2e3`;
- both use client `5ee9b6efb2342452ceeb7958f864fd68d0daadd1` and the same client
  source digest recorded in each generated manifest;
- candidate source evidence is 852 files before/after with aggregate digest
  `2a1a551539e254abbd22914c965eb451a347e409955d0e6e53a76bbd9fd04ff8`;
- the approved producer correction is used as reviewed (`bdb5e29`); strict
  receipt binding and the original reader are unchanged. No reconstructed host
  conditions artifact may be used as an acceptance substitute.

The generated `build-manifest.json`, `server-identity.json`,
`host-conditions.json`, and `spec.json` are part of each cell's lineage. The
server identity is checked by preflight and serialized into both conditions
and server identity. Do not replace it with a PID-only assertion.

## Exact CLI and mode mapping

The controls call the existing `docs/memory/run_diagnostic.py` without changing
it. The focused cell uses:

    panel 16 active --focused-one --nav-captures --binary B \
      --build-manifest M --build-role ROLE --sustain \
      --warmup 30 --observe 120 --render-profile --gpu-completion-profile \
      --scheduling-profile --responsiveness-profile --responsiveness-fine \
      --failure-capture

The background cell uses `--focused-background` in the same command and all
other flags, but does not pass `--nav-captures`: the current validator permits
panel navigation captures only with `--single-renderer` or `--focused-one`.
This is recorded as a capability limitation, not silently weakened. If a future
reviewed tool adds background capture support, add it only after its tests and
CLI mapping are reviewed; do not invent a flag here.

Focused-one means slot 0 is full-rate GPU and the other slots are
simulation-only. Focused-plus-background means slot 0 is full-rate GPU and the
other 15 use the existing configured 1 fps skip-paint path while simulation
cadence remains unchanged. Record requested and selected adapter names from
startup. A configured 1 fps value is not a measured cadence.

## Files and root staging instructions

Transfer this directory without renaming files. The six executable controls are:

- `run-panel-lazy-upload-focused-one.py`
- `run-panel-lazy-upload-focused-plus-background.py`
- `preflight-panel-lazy-upload.ps1`
- `launch-panel-lazy-upload.ps1`
- `poll-panel-lazy-upload.ps1`
- `archive-panel-lazy-upload.ps1`

Also transfer `test_controller_provenance.py`, which evaluates the actual controller
output expressions against the strict evidence reader and tests omitted-field
rejection. This seventh file is a local regression test, not a launch entry point.
Before any launch, run `validate-controls-ast.ps1` from the same directory. It uses
PowerShell's `Parser.ParseFile` over all four executable `.ps1` controls (preflight,
launch, poll, and archive) and fails closed on any syntax error.

Preparation does not copy or modify the candidate. The root BotTest machine
already has the approved candidate staged at
`C:\ProgramData\274bot-Test\panel-3118e96`; verify its SHA-256 against the
value above before candidate launch.
The frozen baseline remains in
`C:\ProgramData\274bot-Test\panel-36825a9`; never replace it. The existing
workload, cache, nav pack, nav flags, catalog, and local server are reused, not
rebuilt or regenerated. Keep the candidate build receipt/logs from
`docs/memory/diagnostics/windows-build-3118e96` with the root evidence; some
JSON files are UTF-16 with BOM and must be read with the matching encoding.

The controller's `LAZY_UPLOAD_HOST_ROOT` may point to the approved host checkout
on the root machine. It must not point to a different source tree. The launcher
requires an explicit role, mode, and unique cell id; it rejects an existing run
or scheduled task.

## Preflight and execution order

Run a fresh preflight for each cell from elevated PowerShell on the unlocked BotTest local
console. It checks the builder VM is off, no cargo/rustc/frontend/VM process is
running, quiet services remain stopped, the server is the expected `node.exe`
process and owns the expected listener, the console is active and unlocked,
and current Process Lasso/driver/process state. Power evidence includes the real
Windows AC-line status, computer-system power state as a separately named field,
active power scheme and AC display/lid settings, battery telemetry, and panel
brightness when available; do not interpret computer-system power state as AC
line status. The DxDiag display list is a saved console observation with source
path, SHA-256 and last-write time, explicitly labeled as not a fresh scan. Fresh
CIM driver/session data and actual startup adapter records remain required; the
saved list does not prove current display routing. Conditions retain the strict
reader purpose and panel/terminal fields, and the server sidecar retains the
three configuration hashes and existing bind-host/Node-version identity.
It writes a cell-specific checked record to the frozen baseline stage; never
reuse a prior cell's timestamped record. Candidate binary presence is
intentionally deferred until candidate staging.

For each cell, use the following order and wait for completion and cleanup
before proceeding:

    .\preflight-panel-lazy-upload.ps1 -CellId baseline-focused-one-<unique>
    .\launch-panel-lazy-upload.ps1 -BuildRole baseline -Mode focused-one `
      -CellId baseline-focused-one-<unique>
    .\poll-panel-lazy-upload.ps1 -CellId baseline-focused-one-<unique>
    .\poll-panel-lazy-upload.ps1 -CellId baseline-focused-one-<unique> -Final
    .\archive-panel-lazy-upload.ps1 -CellId baseline-focused-one-<unique> -Destination D:\274bot-lazy-upload-raw

Repeat for baseline background, then stage and hash-verify the candidate, then
repeat focused-one and background with `BuildRole candidate`. Inspect each
completion and archive before launching the next. Do not run builds, tests,
profilers, another panel/TUI, or a second cell concurrently.

A failed or incomplete cell is retained. Always run the archive helper when
possible, including without `completion.json`; it preserves managed output,
receipt-referenced raw trees, launcher logs, null completion/receipt status,
and hashes every listed file. Never reuse its id, relabel failure as qualified,
or average it into a result. The archive's receipt status is evidence, not a
qualification verdict.

## Evidence and interpretation

For every cell retain the preflight, transfer/stage hashes, full managed run,
all receipt-referenced raw trees, JSONL, logs, captures, startup adapter records,
`spec.json`, build manifest, server identity, host conditions, qualification
stdout/exit status, and archive manifest. Run the existing expected workload
qualifier against each completed archived run with no counting or diagnostics
override unless the cell explicitly declares otherwise:

    python3 docs/memory/qualify_control.py ARCHIVED_RUN --no-write

Qualification is separate from RSS, CPU, cadence, responsiveness, rendering,
visual, lifecycle, and provenance review. Read captures with a capable visual
tool; filenames are not visual proof. Report current RSS, sampled/lifetime peak,
CPU seconds and CPU cores, client cadence, focused callback/paint cadence,
background measured cadence, and tracked GPU bytes in their own accounting
domains. Do not subtract tracked GPU bytes from RSS or attribute the entire mode
gap to GPU memory.

The diagnostic screen is not accepted savings. A falling RSS series requires
elapsed-aligned trend analysis, the same sampling boundaries, and an explicit
statement that no plateau or causal ownership is proven unless the evidence
supports it. These GPU cells provide no CPU-fallback latency proof; do not infer
CPU-renderer behavior, responsiveness, or target-hardware feasibility from them.
The short 30/120/60 cells remain screening evidence and do not replace longer
repeated runs, lifecycle soak, visual review, or final whole-branch review.

`performance_acceptance` remains false in every generated artifact and archive.
