# Optional macOS native process counters

The continuous collector accepts `--process-backend libproc` (API
`process_backend='libproc'`) to read explicit PIDs without spawning `ps`.
Default `system` sampling and the older `server_resources.py` remain unchanged.
There is no silent fallback on failure. Metadata records the selected backend;
injected test samplers are labeled `injected`.

`native_process_sample.py` uses the installed macOS SDK's `rusage_info_v0`
layout: 16 UUID bytes followed by ten uint64 fields, 96 bytes total. The native
call reads current resident bytes separately from physical footprint and uses
process start absolute time as identity; an exited process or missing start
time is rejected. PID validation prevents signed-int truncation. The native
call is not cancellable; its elapsed time is checked against the acquisition
budget after return, and the outer collector retains its watchdog/cadence
checks.

CPU units were verified rather than assumed. Apple XNU
[`fill_task_rusage`](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c)
copies task CPU totals; [`task_power_info_locked`](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/task.c)
uses Mach-time counters. The reader converts with `mach_timebase_info` numer /
denom / 1e9. On this Mac the ratio is 125/3; a direct /1e9 conversion would
undercount CPU by about 41.7 times. A live self-process test agrees with
`getrusage(RUSAGE_SELF)` within 5ms while confirming no waited-child CPU change.

Six native tests plus 36 collector tests passed. They cover ABI offsets,
conversion, distinct RSS/footprint, exited/unknown identity, invalid PID/budget,
native failure and the self-process check. A four-sample collector integration
artifact is `diagnostics/native-collector-self-20260907T052051Z.jsonl`:
exit0, libproc selected, `required_grid_complete`, actual shared sample span
0.310006s for a requested 0.35s grid, and no sampling subprocesses. This is a
self-process diagnostic, not a bot/server run or performance result.

Future managed use must select the same backend for its identity preflight and
continuous samples; native start identities are not interchangeable with the
legacy `macos_lstart` strings. Current RSS accounting for arbitrary descendants,
macOS pressure, observation coverage and profile perturbation remain separate
requirements. No RSS saving or overhead acceptance is claimed here.
