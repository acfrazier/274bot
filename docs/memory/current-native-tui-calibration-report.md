# Current native TUI N16 calibration

This is one diagnostic cell, not an accepted matched saving or final performance
result. All 16 Thiever slots qualified and progressed. Steady RSS and CPU exceed
the approved working targets; no specific allocation owner is established yet.

Runtime and test qualification are recorded in
`current-native-tui-build-report.md`. Runtime host c0709ab/client 3456edc8,
ELF SHA256 a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9,
System allocator, memory-profile-no-alloc, no scheduling/responsiveness/render
profiles, no diagnostic sidecar, no allocation counting, real 120x40 terminal.
The single run used 120 seconds warmup and 600 seconds observation on Concord
after its reboot. Frontend and controller both exited zero.

| Quantity | Measured diagnostic result | Working target |
|---|---:|---:|
| Steady median client RSS | 520.779 MiB | 512 MiB |
| Native process peak RSS | 597.957 MiB | 768 MiB |
| Client mean CPU | 0.561746 cores | 0.5 cores |
| Workload progression | 49–82 steals per slot, all 16 | All slots progress |
| Mean client tick rate | 49.479 ticks per slot per second | Not a per-slot p99 proof |

The qualification phase spans 600.042665854 seconds. The 580 resource samples
bracket 598.91186859 seconds; finite sampling endpoints do not shorten the actual
phase. Client CPU is 336.436373 seconds divided by that sampled bracket.
Sampled maximum resident memory is 626692096 bytes, distinct from the native
high-water reading of 627003392 bytes. Raw measurements and per-slot gains are
in `current-native-tui-calibration-evidence.json`.

The original 93-file archive is
`diagnostics/current-tui-n16-1616/current-tui-n16-1616-final`, archive SHA256
7612191cfe88f61303e1d0409952032a552c53ff87370f90b03fd4d17a2941c4.
Its original binding result rejected the explicit `frontend_processes: []`
observation. Reader correction 82057de, approved by actual Grok 4.5/xai-oauth
session 20260908_124352_8900f2, narrowly recognizes the Linux zero-population
observation. It changes no runtime or raw artifacts. Root's integrated adapter
suite passed all 49 tests.

Root replayed on the native VPS with original absolute paths and verified all
93 preserved files before and after. Replay is `bound`, `binding_ok=true`,
`qualified=true`, and managed cache/process resources are `available`.
`pair_eligible=false` and performance acceptance remains false. Original raw
resource analysis and `shaped_for_compare` still report
`missing_resource_provenance`; managed sidecar binding is a distinct result,
not permission to relabel that gate. Overhead remains unknown. Replay plus
reader sources are preserved beside the original in `binding-replay-82057de`
and `reader-replay-82057de`; replay archive SHA256
3a7f5bbe2dab42f61f967aac8034b7e2d18772662aac7a73bf5fb5f2c583fc87.

Separately bound server accounting is 654934016 bytes median RSS and about
0.05484 CPU cores over the enclosing interval. Controller, launcher, collector,
and SSH-parent identities and resource intervals are reported separately in the
evidence JSON. Those observations are not an OFF/ON overhead comparison and are
not subtracted from client RSS or CPU. No profiled latency, per-slot tail cadence,
input latency, or GPU completion result is supplied by this profile-off TUI cell.

After Stop, the last sample has zero active slots, zero live V8 isolates and V8
bytes, zero inflight snapshot bytes, and retained process RSS 433799168 bytes.
One teardown is not a repeated lifecycle plateau proof. V8 and snapshot counters
identify logical ownership only and cannot be equated to RSS savings.

Before selecting an implementation owner, independently review these artifacts
and recompute the important numbers. One N16 cell cannot separate fixed from
incremental per-bot cost. A bounded ownership experiment must establish that
distinction and evaluate CPU as well as RSS; no additional live run or speculative
optimization is approved by this report itself.
