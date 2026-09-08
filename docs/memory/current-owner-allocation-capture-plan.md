# Current owner allocation capture plan — N1 Heaptrack

Status: design only. This is one diagnostic capture proposal; it authorizes no
live run, optimization, feature change, or performance acceptance. CPU profiling
is deferred until a separate reviewed discriminator step.

## Decision

The selected discriminator is allocation-stack families and their requested
bytes at the single common-time global allocation peak. Exact vector capacities
and snapshot publication-epoch overlap remain unmeasured.
Phase A cannot rank `Client`/`World`, snapshot families, shared cache/interface
owners, script buffers, or retained allocator pages from headers, serialized
lengths, V8 gauges, or RSS. Heaptrack is therefore the next bounded diagnostic.
It will report allocation call stacks and live/peak requested bytes for the
owned frontend process. It will not establish causal RSS ownership, V8
size, mmap/GPU ownership, or CPU cost.

The allocation-family discriminator is the first stack frame in the captured
allocation backtrace that resolves to a frozen Rust function or a named
allocator boundary, grouped by that symbol in the common-time global-peak
population. A family is useful only when it has positive peak bytes and can be
mapped to an owner/lifetime from source. Rust function symbols are readable
evidence, not source-line or inline-coverage proof. The frozen ELF is known to
retain symbols such as `script::isolate_fb::encode_snapshot_masked_into`;
source/inline coverage remains unproven and must be reported as such.

## Exactly one procedure: N1 native TUI allocation capture

Run exactly one root-owned capture on the current Concord host, with no N16
pair, rerun, shortened alternate, optimization, or CPU profile.

### Frozen identity and fixture

- `N=1`, frontend `tui`, workload `active`.
- Host commit `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`.
- Client submodule commit `3456edc8dabf7b25ada78110ffa56327af9f67a4`.
- Frontend binary: `/home/acfrazier/274bot-campaign/calibration-c0709ab-incoming/tui-play`.
- Binary SHA-256: `a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9`.
- Build manifest: `/home/acfrazier/274bot-campaign/calibration-c0709ab-incoming/build-manifest.json`, with `requested=memory-profile-no-alloc`, locked `true`, System allocator, allocation counting `false`, and `snapshot-dedup=false`.
- Nav inputs are the frozen manifest values: `/home/acfrazier/.274bot/274bot.navpack` SHA-256 `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`, and its declared nav-flags file.
- Cache/account fixture must be the current N1 identity, not a newly generated
  account or cache: cache provenance
  `/home/acfrazier/274bot-campaign/current-tui-n1-1727/result.json.cells/result/cache-provenance.json`, cache-provenance SHA-256 `d2b65fdd61565a32c439aeeb97041e0d87301d704c4f71b7e7e194668722112f`, and cache content identity SHA-256 `2a660d20eaccd1491da88e4b4d2e1ba0e0bdea7c73dbda954f8df4f97bb9d9a3`.
- Revalidate the fresh post-update Concord identity immediately before launch:
  boot ID `2217ec26-dc3e-47a3-a700-d3fafb34163f`, kernel
  `6.8.0-139-generic`, server PID `726`, start identity
  `linux_proc_start_ticks:595`, server enabled, and SSH socket active. Use
  `diagnostics/concord-post-updates-1823/post-updates-1823/receipt.json` as the
  provenance anchor; do not reuse the old start identity `1761` or the old
  `oldstart1761` receipt. Recheck the server identity sidecar, fixture manifest,
  world configuration, public key, and binary hashes before reserving capture
  output.

### Launch and ownership boundary

Use the existing `run_current_tui_calibration.py` and `run_diagnostic.py`
contract, preserving their source/build/server/cache validation and their
120-second warmup, 600-second observe, and 60-second teardown. The diagnostic
must remain `--no-diagnostics --sustain`, with scheduling, responsiveness,
render, GPU-completion, input-probe, allocation-counting, and snapshot-dedup
features off. Do not attach to an already running frontend.

The missing runner integration is a small reviewed launch seam in
`docs/memory/run_diagnostic.py`: add an explicit `--heaptrack-output` option
accepted only with the N1 TUI diagnostic, thread it through
`build_child_env()`/`main()`, and set these variables only in the frontend child
environment at the existing `subprocess.Popen` sites (Unix PTY lines 373-381
and non-PTY lines 403-404):

    LD_PRELOAD=/usr/lib/heaptrack/libheaptrack_preload.so
    DUMP_HEAPTRACK_OUTPUT=<unique-0700-capture-dir>/alloc.raw

The controller seam is `run_current_tui_calibration.py:diagnostic_argv` and
`build_spec` (lines 170-220): it must pass the unique output path and record the
same argv and direct-preload metadata in the spec before
`run_managed_cell.run_managed_cell` is called at lines 304-306. The effective
debuggee command remains the real binary, with the existing PTY, cwd, and
validated child environment passed through; there is no `/usr/bin/heaptrack`
wrapper, shell, FIFO, copied binary, or attach operation. This preserves the
frontend's real PID and binary identity. Record the exact installed preload
library, version, and SHA-256 (verified Concord path:
`/usr/lib/heaptrack/libheaptrack_preload.so`, Heaptrack 1.5.0).

After the frontend exits, and only then, interpret the raw artifact with:

    /usr/lib/heaptrack/libexec/heaptrack_interpret < <capture-dir>/alloc.raw > <capture-dir>/alloc.interpreted

Then run the proven `heaptrack_print` command below against the interpreted
artifact. Direct raw mode has no profiler wrapper, interpreter, or compressor
child during collection. Account for the controller, real frontend, game
server, and SSH parent separately; record the frontend PID/start identity, and
record the post-exit interpreter PID/exit status as analysis metadata rather
than as captured frontend ownership. The capture directory is unique, owned, and mode0700. The raw file is created
inside it, never reused or symlinked, and restricted to mode0600 via the owned
child umask077. The directory and file permissions are different facts.

This seam is instrumentation-only and needs its own reviewed behavior contract
before launch. No change to the Rust/client behavior or to account/cache/server
state is part of this card. If the seam cannot preserve the direct frontend
identity and explicit process accounting, fail closed without starting a
capture.

### Phase boundaries and capture windows

1. Preflight: verify source relationship against the frozen host/client commits,
   exact ELF and manifest hashes, current cache identity, current server PID and
   start identity, fixture/config hashes, `heaptrack 1.5.0`, available memory,
   free disk, and output directory emptiness. Record the command and all hashes.
2. Warmup: run the existing N1 active workload for exactly 120 seconds. Record
   the warmup-end controller timestamp, but do not treat it as a Heaptrack
   boundary or claim that the report separates initialization from steady work.
3. Observe: run exactly 600 seconds with the normal active workload. Record the
   observe-start and observe-end controller timestamps as lifecycle metadata only.
   The continuous Heaptrack profile is analyzed as one common-time global-peak
   population; it does not provide a phase-separated observe-end live set.
4. Teardown: invoke the existing normal Stop path for 60 seconds and record the
   `script_stop` invocation and teardown-end timestamps. These are external
   barriers only, not an after-Stop allocation population. If the managed
   lifecycle reaches a slot join before teardown ends, record that fact as
   metadata; do not manufacture a post-join sample from process exit.
5. Cleanup: stop the frontend through the existing managed path, close the raw
   artifact, run the interpreter and analysis commands, verify output
   completeness, remove only per-attempt process files, and preserve the
   raw/interpreted allocation artifacts and report. An interpreter or analysis
   failure is a failed attempt, not a successful cleanup.

The capture population is the owned frontend process only. The game server, SSH
parent, controller, and unrelated services are outside the allocation population
and are separately accounted roles. The post-exit interpreter's memory and CPU
are analysis overhead, not frontend ownership. Heaptrack observes ordinary
malloc/new-family allocations; custom pools, mmap, V8-native allocations, GPU
resources, and page retention may be absent or incomplete.

## What to classify and what remains unknown

Analyze the preserved artifact with the exact Heaptrack 1.5 command proven by
the owned smoke:

    heaptrack_print --merge-backtraces=0 --flamegraph-cost-type=peak --print-flamegraph <capture-dir>/peak-stacks.txt <capture-dir>/alloc.interpreted > <capture-dir>/peak-analysis.log

Parse the flamegraph rows as one common-time global-peak population; retain
the raw text, row count, positive-row count, and the sum of positive costs.
The smoke produced 2,617 rows, 2,087 positive rows, a 5,435,240-byte sum, and
the printed 5.44M global peak; the largest 4,196,352-byte family resolved to
`PyByteArray_Resize` and matched its known request calculation. The default
merged-backtrace peak is not valid for ranking because merged independent peaks
are not a common-time population. Never sum independent per-stack peak bytes.

Do not invent an observe-end or after-Stop Heaptrack slice. Heaptrack 1.5's
available printer modes (`--print-flamegraph`, `--print-massif`, peaks,
allocators, and leaks) do not expose a lifecycle-bound live-set API. The
continuous capture records external controller timestamps for warmup end,
observe end, `script_stop` invocation, and teardown end, but those timestamps
are metadata only and are not join keys for a per-phase allocation population.
`--print-massif` may be retained as an aggregate time-series diagnostic, not as
an owner tree at either boundary. Therefore this one procedure ranks only
families in the proven common-time global peak. It can select a material
source-mappable private candidate; temporal retention at observe end or after
script Stop, snapshot equality, and post-join state remain unknown.
For each top peak family, retain symbolized stack, allocation count, requested
live bytes, peak bytes, the external lifecycle labels (without claiming a
phase-specific population), PID/role, and whether the first useful symbol is a
frozen Rust function, a dependency, libc allocator, V8/native code, or
unresolved. Map Rust families only to the ledger domains already established:
shared cache/interface template, per-client Client/World/entity storage,
GameSnapshot and its family vectors, nav snapshot/publication shell, script
isolate/fingerprint/encoded buffers, or nav/status queues. A family that cannot
be source-mapped stays `unresolved`; do not guess from a serialized snapshot or
identical stack.

The supported question is whether a material stack family occurs in the
common-time global peak and is source-mappable to a shared or per-bot owner. A
material private family can select a specific candidate for a later reviewed
ownership change, but does not prove that it survives either lifecycle
boundary. The two publication-epoch shells and snapshot equality remain
unknown: stacks alone cannot prove equality or coexistence. If no family is
both symbolized and source-mappable, or if Heaptrack coverage is materially
incomplete, ownership/RSS causality remains unresolved.

Not covered by this capture: V8 heap totals, V8/native allocations not seen by
Heaptrack, mmap/GPU allocations, allocator page retention, RSS causality,
process CPU, elapsed-span attribution, server/helper cost as client cost, exact
snapshot equality inferred from stacks, and a post-join resident value. These
remain explicit unknowns. CPU attribution is a separate future design step.

## Bounds, provenance recheck, and failure rule

Before launch, require a fresh `/proc/meminfo` and server sample. Capture
admission is **MemAvailable >=768MiB**. The unprofiled N1 peak is about177MiB;
256MiB is an additional planning allowance, with the rest a safety margin.
The server's already-used memory is outside MemAvailable. The preload lives
inside the frontend; its overhead cannot be independently measured by PID.
Do not report the allowance as observed overhead or subtract177MiB from a
profiled RSS to invent it.

During collection sample at0.5s and abort if frontend **total RSS >512MiB**,
MemAvailable <128MiB, or the existing controller guard fails. This is a
process-total operational ceiling, not an owner or overhead measurement.
Require free disk >=8GiB and a unique empty owned capture directory. At0.5s
poll the owned raw output size; >=2GiB requests Stop and marks the capture
failed. Set **RLIMIT_FSIZE soft=hard=3GiB only on the frontend**, before exec,
for an enforced per-file ceiling. Preserve the inherited limit if it is more
restrictive and report that restriction rather than raise it. No filesystem
quota or privileged configuration is involved. Keep the existing960s maximum
for120s warmup +600s observe +60s teardown +180s launch/cleanup allowance.
Use existing identity-checked Stop/TERM/KILL cleanup on a guard breach; do not
signal the server or an unrelated process, and preserve partial raw bytes.

After the frontend exits, run the interpreter and printer sequentially on
Concord. Each analysis process has wall timeout180s, total RSS ceiling512MiB
(sample at0.5s), and child-only RLIMIT_AS768MiB. Before each command require
MemAvailable >=768MiB. The interpreter has RLIMIT_FSIZE3GiB; the printer has
RLIMIT_FSIZE512MiB, bounding both peak-stacks and its stdout report. Set limits
only in these owned child processes. Capture their PID/start identity, argv,
limits, exit status and logs separately. Timeout, RSS/limit breach, missing
output, or nonzero exit fails the analysis; terminate and reap only that owned
process. These two180s analysis allowances are outside the live960s budget.
Raw3GiB + interpreted3GiB + two512MiB printer files fit within the8GiB disk
admission with margin; continue checking free disk and abort below1GiB.
The interpreter is never part of the live frontend allocation population.

The source/build/cache/server provenance must be rechecked immediately before
launch and recorded in the capture manifest. The fresh Concord receipt is
`diagnostics/concord-post-updates-1823/post-updates-1823/receipt.json`; its
recorded binary SHA, fixture manifest SHA, world SHA, package/tool versions,
boot ID, kernel, server PID, and start identity are the required baseline. The
capture manifest must also record the Heaptrack version, exact profiler argv,
actual frontend PID/start identity, separate analysis PIDs/start identities, exit codes,
artifact sizes and hashes, and the post-cleanup process check.

One attempt only. Failure means any provenance mismatch, stale/reused server
identity, wrong binary/feature/cache/account, missing direct frontend identity,
wrong preload library or output ownership, memory/disk bound breach, raw
interpretation failure, incomplete/corrupt output, abnormal frontend exit, or
cleanup leak.
Preserve partial artifacts and logs with a failed receipt; do not rerun, change
N, shorten the observation, switch profiler mode, or reinterpret a failed run
as evidence. A failed attempt leaves the discriminator unresolved and requires a
new reviewed card for any different procedure.

## Evidence anchors and interpretation limits

The chosen direct-preload path is proved by
`diagnostics/heaptrack-direct-smoke-1857/receipt.json`: record, interpretation
and printer each exit0; raw332128bytes; peak costs sum5435240bytes; largest
4196352-byte family resolves to PyByteArray_Resize and matches2048*2049requests.
Preload `/usr/lib/heaptrack/libheaptrack_preload.so` SHA256 is
`134760dbba8d9a2cd1b45f639c119f5c0cd779674a496fb7ed2b6baacac15ca9`.
Recheck the installed library/version/hash before every owned frontend launch.
No wrapper, FIFO or collection-time helper was used in this smoke.

The earlier `diagnostics/heaptrack-smoke-1837` wrapper smoke and offline peak
and Massif replays are supporting historical tool evidence. The99-sample perf
smoke proves CPU-tool access only. Neither supplies client allocation, phase
retention, CPU attribution or accepted savings. The approved owner ledger,
fresh Concord post-update receipt and existing N1 managed contract remain the
source, lifecycle and provenance anchors.

This document is the reviewed-scope candidate for the next executable
diagnostic design. Root must first verify the launch seam, output paths,
process-role discovery, and bound enforcement on Concord; this card does not
execute the capture, release a new live cell, or authorize instrumentation
before review.
