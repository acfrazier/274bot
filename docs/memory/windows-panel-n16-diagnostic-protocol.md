# Native Windows panel N16 Intel diagnostic protocol

Status: prepared controls only; no Windows command, native call, workload launch,
or source/binary build was executed by this task.

## Purpose and fixed evidence

This is a bounded feasibility diagnostic for the approved panel N=16 cells. It
uses the existing frozen Windows panel binary `36825a9`, client `5ee9b6e`, local
BotTest unlocked console, and existing local game server. It does not alter
production adapter selection and does not grant performance acceptance.

The two cells are sequential and must use the same conditions:

1. `n16-focused-one-console-intel`: `panel 16 active --focused-one`; slot 0 is
   the focused full-rate GPU renderer and other slots are simulation-only.
2. `n16-focused-plus-background-console-intel`: `panel 16 active
   --focused-background`; slot 0 remains focused and the other 15 render with
   the existing configured 1 fps skip-paint path while simulation cadence is
   unchanged.

The actual mode flags are proven in `docs/memory/run_diagnostic.py` and the
accepted mode mapping in `docs/memory/reference-mode-report.md`. Do not replace
these with an invented flag or infer the mode from a filename.

Both controllers declare `count: 16`, `requested_adapter: Intel(R) Graphics`,
the mode, binary SHA, source/client manifest hashes, server identity, nav/cache/
catalog hashes, and `performance_acceptance: false`. The background controller
also declares `configured_background_fps: 1`. The explicit Intel adapter is a
diagnostic environment variable only; startup records must prove both requested
and selected adapter names.

## Files and transfer

Transfer this entire directory without renaming files:

- `run-panel-n16-focused-one-36825a9-intel.py`
- `run-panel-n16-focused-plus-background-36825a9-intel.py`
- `preflight-panel-n16-36825a9-intel.ps1`
- `launch-panel-n16-36825a9-intel.ps1`
- `poll-panel-n16-36825a9-intel.ps1`
- `archive-panel-n16-36825a9-intel.ps1`

On the root workstation, transfer to a new, empty staging directory and record
hashes before any use. Example root commands (replace only the source and
destination transport paths; do not use a shell-expanded wildcard):

```powershell
$src = 'C:\Users\<operator>\n16-controls'
$dst = 'C:\Users\BotTest\274bot-n16-controls-20260907'
New-Item -ItemType Directory -Path $dst
Get-ChildItem $src -File | Get-FileHash -Algorithm SHA256
Copy-Item (Join-Path $src 'run-panel-n16-focused-one-36825a9-intel.py') $dst
Copy-Item (Join-Path $src 'run-panel-n16-focused-plus-background-36825a9-intel.py') $dst
Copy-Item (Join-Path $src 'preflight-panel-n16-36825a9-intel.ps1') $dst
Copy-Item (Join-Path $src 'launch-panel-n16-36825a9-intel.ps1') $dst
Copy-Item (Join-Path $src 'poll-panel-n16-36825a9-intel.ps1') $dst
Copy-Item (Join-Path $src 'archive-panel-n16-36825a9-intel.ps1') $dst
Get-ChildItem $dst -File | Get-FileHash -Algorithm SHA256
```

Compare the two hash listings and preserve both as `controls-transfer-hashes`
in the root evidence bundle. Do not copy into or overwrite the frozen binary
stage. The launch script stages a uniquely named copy under the existing stage;
that is an execution-time root action, not performed during preparation.

## Root preflight and exact execution order

Run from an elevated PowerShell only on the BotTest machine and local console.
The preflight is a gate, not a best effort:

```powershell
Set-Location 'C:\Users\BotTest\274bot-n16-controls-20260907'
.\preflight-panel-n16-36825a9-intel.ps1
```

It requires the builder VM off, no cargo/rustc/panel/tui/VM workload, quiet
services still stopped, the existing server PID/listener identity, active local
BotTest console, and no lock screen. It samples Process Lasso state and files
rather than assuming a configured state. Preserve the resulting
`C:\ProgramData\274bot-Test\panel-36825a9\preflight-panel-n16-intel.json`.
Do not run with another panel, TUI, VM, build, benchmark, or native profiler.
Do not stop or restart the server or ambient helpers.

Launch exactly one cell, poll it to completion, and archive it before launching
the next. The 18-minute scheduled-task limit and 900-second controller max wall
allow preparation for N=16 while preserving the per-cell 30/120/60 timings:

```powershell
.\launch-panel-n16-36825a9-intel.ps1 -Mode focused-one
.\poll-panel-n16-36825a9-intel.ps1 -Mode focused-one
.\poll-panel-n16-36825a9-intel.ps1 -Mode focused-one -Final
.\archive-panel-n16-36825a9-intel.ps1 -Mode focused-one -Destination 'D:\274bot-n16-raw'

# Inspect focused-one completion/archive and confirm cleanup before continuing.
.\launch-panel-n16-36825a9-intel.ps1 -Mode focused-plus-background
.\poll-panel-n16-36825a9-intel.ps1 -Mode focused-plus-background
.\poll-panel-n16-36825a9-intel.ps1 -Mode focused-plus-background -Final
.\archive-panel-n16-36825a9-intel.ps1 -Mode focused-plus-background -Destination 'D:\274bot-n16-raw'
```

If a cell fails, preserve its output and scheduled-task state; do not reuse its
name, retry until favorable, or overwrite the archive. Run the archive helper
even when the controller, frontend, sampler, or qualification step failed, and
also when a controller crash leaves the run directory without
`completion.json`. In that incomplete case the helper archives all available
managed contents, discoverable receipt-referenced `run_dir` raw trees, and the
matching launcher log; it records `completionPresent=false` with null receipt
exit/status rather than inventing a verdict. It copies the managed receipt tree,
every receipt-referenced `run_dir` raw tree, and the matching launcher log into
immutable `raw-run-NN`/`launcher.log` destinations (or records a missing
referenced directory), then hashes every archived file listed in
`archive-manifest.json`.
The receipt exit/status remains evidence, not a qualification verdict; failed or
incomplete bundles must be retained and must never be relabeled as qualified.
The managed runner owns
only its frontend/collector children and performs its normal owned-process
cleanup. It does not signal the game server or ambient helper PIDs.

## Qualification and raw evidence bundle

A process exit of zero is not qualification. For each archived cell retain the
full run directory, all receipts, JSONL, logs, captures, `spec.json`,
`build-manifest.json`, `server-identity.json`, `host-conditions.json`, startup
adapter records, preflight JSON, transfer hashes, and archive manifest. Record
controller/launcher/PowerShell exit codes and exact UTC intervals.

For completed active cells, run the existing expected qualification command from
the host checkout against the archived run (the exact run path is shown by the
receipt):

```powershell
python3 docs/memory/qualify_control.py '<ARCHIVED-RUN>' --no-write
```

Use no `--counting` and no `--diagnostics` override for these system-allocator,
non-sidecar controls. Record stdout JSON and exit status. A qualified workload
still needs separate cadence, resource, rendering, lifecycle, provenance, and
visual review. Read the actual captures with a capable visual tool; filenames
are not proof. These are diagnostic cells, not final acceptance runs.

The raw bundle must explicitly state whether each cell had all requested slots
ready/active, progress, final GPU pending/lost/dropped faults, and cleanup. Keep
failed and incomplete evidence alongside successful evidence. Do not average a
failed cell into a pass.

## Limits and interpretation

Use the approved targets only as limits, not as implied results:

- panel N=16 steady median RSS <=768 MiB;
- panel N=16 startup/transition peak RSS <=1 GiB;
- CPU <=1 average process core in each panel mode;
- simulation >=40 iterations/slot/s, 20 ms target, p99 start interval <=40 ms;
- focused completed/presented frames >=40/s and p99 frame interval <=40 ms;
- responsiveness endpoints <=100 ms p99;
- no swapping/OOM/persistent backlog and lifecycle cleanup without owner growth.

Report current frontend RSS samples separately from peak/`ru_maxrss` evidence;
current RSS is not peak RSS. Report CPU seconds/wall seconds separately, and
report GPU renderer residency and completed/presented cadence separately from
host RSS. For focused-plus-background, record actual background renderer count
and observed skip-paint cadence; configured 1 fps is not measured 1 fps.

These short 30/120/60 cells are screening evidence under approved section 5.
They are not the final schedule of three fresh 120/600 runs per mode, do not
establish target-hardware feasibility, and do not change the existing Intel or
Process Lasso policy. Preserve all earlier Intel/NVIDIA failures and the prior
Intel N1 evidence; this protocol adds only uniquely named N16 diagnostic output.
