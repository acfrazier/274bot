# Current owner allocation capture plan — N1 Heaptrack

Status: design only. This is one diagnostic capture proposal; it authorizes no
live run, optimization, feature change, or performance acceptance. CPU profiling
is deferred until a separate reviewed discriminator step.

## Decision

The selected discriminator is native live allocation families and requested
capacities, with snapshot publication-epoch overlap as a secondary observation.
Phase A cannot rank `Client`/`World`, snapshot families, shared cache/interface
owners, script buffers, or retained allocator pages from headers, serialized
lengths, V8 gauges, or RSS. Heaptrack is therefore the next bounded diagnostic.
It will report allocation call stacks and live/peak requested bytes for the
owned frontend process tree. It will not establish causal RSS ownership, V8
size, mmap/GPU ownership, or CPU cost.

The allocation-family discriminator is the first stack frame in the captured
allocation backtrace that resolves to a frozen Rust function or a named
allocator boundary, grouped by that symbol and phase. A family is useful only
when it has live bytes at the declared capture point and can be mapped to an
owner/lifetime from source. Rust function symbols are readable evidence, not
source-line or inline-coverage proof. The frozen ELF is known to retain symbols
such as `script::isolate_fb::encode_snapshot_masked_into`; source/inline
coverage remains unproven and must be reported as such.

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
`build_child_env()`/`main()`, and launch the binary at the existing
`subprocess.Popen` sites (Unix PTY lines 373-381 and non-PTY lines 403-404)
through the profiler prefix. The controller seam is
`run_current_tui_calibration.py:diagnostic_argv` and `build_spec` (lines
170-220): it must pass the unique output path and record the same argv in the
spec before `run_managed_cell.run_managed_cell` is called at lines 304-306.
The effective child command must be:

    /usr/bin/heaptrack -o <capture-dir>/alloc /home/acfrazier/274bot-campaign/calibration-c0709ab-incoming/tui-play

with the existing PTY, cwd, and validated child environment passed through. The
real frontend ELF remains the debuggee; do not replace it with the Python
controller, a shell wrapper, or a copied profiler binary. The installed
Heaptrack 1.5 script uses per-process `LD_PRELOAD` and an output FIFO, then
creates interpreter and compressor children. Its documented direct-attach path
is unstable and is prohibited here. The launch seam must record, separately,
the Heaptrack wrapper PID, real frontend PID, interpreter PID, compressor PID,
controller PID, and game-server PID/start identity. Existing process accounting
must include the profiler/helper roles for the full run and cleanup; it must not
label the profiler wrapper as the frontend or omit its overhead.

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
2. Warmup: run the existing N1 active workload for exactly 120 seconds. Do not
   interpret warmup allocations as the selected live population; retain them in
   the profile so initialization families can be distinguished from steady work.
3. Observe: run exactly 600 seconds with the normal active workload. Identify
   the observe-start boundary after warmup and select one bounded analysis slice
   at the existing observe-end boundary. The Heaptrack data is continuous, but
   report allocation families separately for initialization/warmup, observe,
   and teardown when the report supports phase timestamps; never silently merge
   phases.
4. Teardown: invoke the existing normal Stop path for 60 seconds. Record the
   after-script-Stop population while the client and snapshots are still alive.
   Do not call it post-join. If the managed lifecycle reaches a slot join
   before the capture point, record the explicit barrier and keep that population
   separate; do not manufacture a post-join sample from process exit.
5. Cleanup: stop the frontend through the existing managed path, wait for the
   frontend and every Heaptrack interpreter/compressor child, close and analyze
   the FIFO, verify output completeness, remove only per-attempt temporary FIFO
   and process files, and preserve the compressed allocation artifact and report.
   A surviving profiler/helper after the frontend exits is a failure, not a
   successful cleanup.

The capture population is the owned frontend plus its Heaptrack helper process
family. The game server, SSH parent, controller, and unrelated services are
outside the allocation population and are only separately accounted roles.
Heaptrack's interpreter/compressor bytes are helper overhead and must be
reported, not attributed to a Rust owner. Heaptrack observes ordinary malloc/
new-family allocations; custom pools, mmap, V8-native allocations, GPU
resources, and page retention may be absent or incomplete.

## What to classify and what remains unknown

Analyze the preserved artifact with the exact Heaptrack 1.5 command proven by
the owned smoke:

    heaptrack_print --merge-backtraces=0 --flamegraph-cost-type=peak --print-flamegraph <capture-dir>/alloc.zst > <capture-dir>/peak-stacks.txt

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

Before launch, require a fresh `/proc/meminfo` and server sample. The existing
server is about 630 MiB RSS and the unprofiled N1 frontend peak is about 177
MiB. Capture admission is **MemAvailable >= 768 MiB**, which reserves the
known frontend peak plus a predeclared 256 MiB profiler/helper allowance and
an additional safety margin; it is separate from the server's already-used
RSS. Record the measured value and the rationale in the manifest and fail
closed below it. The expected profiler/helper overhead is bounded separately
at 256 MiB for admission; exceeding that budget is a failed diagnostic, not a
performance result. During the run retain the existing 128 MiB
`MEM_AVAILABLE_GUARD_BYTES` as a last-ditch runtime abort, not as admission.

Also require fresh free disk >= 4 GiB and an empty unique owned capture
directory. Poll `du -sk` for the directory (allocation artifact and temps)
every 0.5 s: 2 GiB is a soft limit that requests orderly frontend/profiler
stop and marks the attempt failed; 3 GiB is a hard limit that terminates the
owned profiler/frontend process group, then waits for all children. These are
owned-runner checks, not a filesystem quota or privileged configuration. Bound
the attempt to one 120s warmup + 600s observe + 60s teardown, plus the
controller's existing 180s launch/cleanup allowance. Preserve partial output
and logs on either bound breach, and never retry or shorten the window.
Keep profiler/helper process accounting at 0.5-second sampling.

The source/build/cache/server provenance must be rechecked immediately before
launch and recorded in the capture manifest. The fresh Concord receipt is
`diagnostics/concord-post-updates-1823/post-updates-1823/receipt.json`; its
recorded binary SHA, fixture manifest SHA, world SHA, package/tool versions,
boot ID, kernel, server PID, and start identity are the required baseline. The
capture manifest must also record the Heaptrack version, exact profiler argv,
actual frontend PID/start identity, helper PIDs/start identities, exit codes,
artifact sizes and hashes, and the post-cleanup process check.

One attempt only. Failure means any provenance mismatch, stale/reused server
identity, wrong binary/feature/cache/account, missing direct frontend identity,
missing helper accounting, memory/disk bound breach, FIFO/interpreter/compressor
failure, incomplete/corrupt output, abnormal frontend exit, or cleanup leak.
Preserve partial artifacts and logs with a failed receipt; do not rerun, change
N, shorten the observation, switch profiler mode, or reinterpret a failed run
as evidence. A failed attempt leaves the discriminator unresolved and requires a
new reviewed card for any different procedure.

## Evidence anchors and interpretation limits

This plan is based on the approved Phase A ledger and attribution plan, the
Heaptrack 1.5 owned Python smoke (`diagnostics/heaptrack-smoke-1837`, record and
report exit 0, 5,902 allocation calls, expected `PyByteArray_Resize` stack), the
99-sample perf smoke (`diagnostics/perf-smoke-1836`, tool access only), the
fresh Concord post-update receipt, and the existing N1 managed launch contract.
Those smokes establish tool collection and cleanup only; they are not client
allocation or CPU evidence. Heaptrack's own help explicitly warns that runtime
attach is unstable, so the capture is launch-wrapped, not attached.

This document is the reviewed-scope candidate for the next executable
diagnostic design. Root must first verify the launch seam, output paths,
process-role discovery, and bound enforcement on Concord; this card does not
execute the capture, release a new live cell, or authorize instrumentation
before review.
