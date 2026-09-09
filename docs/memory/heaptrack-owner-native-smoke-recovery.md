# Retained native saved-smoke proof after launcher failures

Root evidence for t_276a9ed9, 2026-09-09 UTC. Frozen native source is
a55972aa5cce2902c2b09aa098ee825d84a5957a, approved by actual Grok4.5/xai
session20260908_203046_6a6bdc. This does not authorize production replay.

## Preserved failures and bounded recovery

The qualification driver was invoked twice. Driver3492 failed at qualify.py:109
with FileExistsError because its wrapper precreated qualification-1. That is
before executable checks, core tests or timing cells; it remains a failed
invocation. The wrapper artifacts were moved to qualification-launch-wrapper-error-1.
Driver3510 subsequently completed all9 cases/27 repetitions,16 core and42 native
tests without Linux skips, in33.954859 seconds. Root independently recomputed
81 rates and27 medians, all108 matching. Successful qualification JSON SHA256:
26185ab784edebeec9a667e2a83bfc8d63454463df5b07cffa72ee4d0025d786.
This is not a claim that the original one-invocation instruction was followed.

The worker then attempted saved-smoke-1. Its wrapper called smoke_manifest from
/home/acfrazier instead of the staged repository root, causing FileNotFoundError
before creating the manifest or launching either engine. Root recovered both
exact tool tracebacks, not reconstructed stderr, into the evidence directory.
The failed directory and its admission receipt remain unchanged.

After confirming that cause, root explicitly authorized and executed one
corrected pair at fresh saved-smoke-2. No qualification rerun, production input,
interpreter, capture, guard relaxation or source change occurred. The recorded
root script sets cwd before importing/calling the unchanged fixture helper.

## Identity, admission and results

Evidence: diagnostics/root-native-a55972a-qualification/. The saved_smoke_2.py
script hashes all41 staged files against the pinned manifest, requires the
completed qualification, fresh memory/disk admission, no listed competing game,
profiling or build processes, and an absent output directory. It validates every
allowlisted fixture entry and its actual bytes (each <=2MB), then runs Python
once followed by native once with lower-only CPU30/wall60 limits. The controlled
fixture requires --portable-fixture; on Linux, both supervisors still record
linux_proc_monitor=true. Root supplies the explicit memory admission that the
portable manifest mode does not perform itself. This is not production mode.

The native executable SHA256 is
98b5a2a20b0daf66057024f4618972fb34219b34568a09fa3be89a674d265b7f.
Manifest SHA256 is
98d73772eca4222f0511ef637cedb455cb04bca303e93f4619c8fdf0f7b0c87e.

Python runner3928/child3929 and native runner3930/child3931 exited0; both parent
receipts record child reaping. Root wrapper3927 also exited0. A separate /proc
readback at Unix1788914947.3318958 found all five PIDs absent. Root retained the
exact commands, stdout/stderr, both runner receipts and all result files.

All17 filenames match. TSV bytes are exact; structured JSON matches after only
the established resources and tables.high_charged_bytes exclusions. No semantic,
definition, hash, graph or cutoff fields are excluded. Root repeated the17
comparisons after export and rehashed all44 payload files against the export
manifest. The archive also contains that manifest as an additional file:
saved-smoke-2-export.tar.gz,245268 bytes, SHA256
5badbfa6af786bff88b617d12053466327df363eaf8cc0b9e33abed2103128fc.

Both receipts prove full positive canonical multiset equality:2951 canonical
rows/2212 positive rows, peak5730024 requested bytes/4962 allocations. Requested
500ms is bracketed by actual497/507ms marks. EOF is407233 requested bytes/34
allocations. The exact graph, descriptor and stack outputs remain in the archive.

Native supervisor wall time was0.169862s; its three phase CPU times were
0.017335083/0.024497203/0.124946683s. The supervisor observed4,104,192 RSS and
5,349,376 address bytes, while the child's cumulative peak RSS was20,578,304.
These different measurements are retained separately. Short sampling does not
establish an exact peak or sustained cadence, and cumulative peak can include
earlier process lifetime. This pair is functional proof, not a matched memory
saving or production throughput measurement.

## Remaining boundary

The current fixture correctness and necessary-rate gates have positive evidence;
the reviewed report must still account for launcher deviations and this separate
root recovery. Small warm fixtures do not bound production populations, tables,
symbols, cold I/O or output. The failed original capture, missing observe-end,
Stop/post-join evidence, original production replay failure and all campaign
acceptance gates remain unresolved. Root must make a new explicit admission and
one-attempt decision before any production replay. Final whole-branch Grok4.6
review remains required.
