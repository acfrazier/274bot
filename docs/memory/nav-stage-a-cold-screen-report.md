# Tiled navigation: cold-load gate failed

**Park the candidate under the current experiment contract.** The reviewed
startup/load-only screen completed all six pairs and both separate counting
runs. Median cold load rose from **108.637 ms to 452.555 ms**: **343.919 ms extra,
+316.6%**, against the predeclared +10% gate. Baseline repeat range was 6.96%,
so the failure is not masked by the measured baseline variation. The startup
peak gate passed. Full route performance and Stage B remain unqualified.

This is a completed necessary-gate result, not the earlier interrupted59-case
comparison. No longer routing or resident matrix follows from this result.

## Frozen screen and qualification

Method approved by `t_eb56e9b6`, actual Grok4.5/xai session
`20260909_095544_27c777`; see `nav-stage-a-first-attempt-review.md`.
Root release STATE `f0965f3`, new authorization SHA
`c967039a1ad535d0d830bb4eb0b0057177ec9517b0b2e3ce73cf7d2a4624bf65`.
Same reviewed tool `f24de7cbcaaa3f419fc5485b77c4a117543a6afc`, same original
baseline/tiled/client pins and four admitted Linux binaries as
`nav-stage-a-first-attempt-report.md`. No source change, rebuild or re-admission.

Artifacts were copied to a new `concord-cold-screen-01` directory with original
source/admission/qualification hashes intact and explicit relocation receipt.
The prior failed `concord-run-01/real-release` was not reused.

Full original274 pack:73438581 bytes, SHA
`2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`.
Only earlier-reviewed row40 was selected:
`3222 3218 0 1855 1280 0 0 0 0 0`, TSV SHA
`0694cd204e005868d3c772ccf65f389d9c57b8fcee0166279ac175c2c75c9516`.
Both arms use identical selectors/facts/options and all three original API lanes.
The observed aggregate is24 NoPath calls, no successful route/bank/transport,
in every clean and counting process. These route values are **diagnostic only**.

Concord: Linux6.8.0-139/glibc2.39,2CPUs,2014852KiB RAM, no swap,
same boot2217ec26-dc3e-47a3-a700-d3fafb34163f. Fresh14:01:03Z preflight:
972984KiB available,98.32% idle/0steal over3s, no competing campaign process.
Same six-per-arm AB/BA order; unchanged120s wall/90s CPU/1GiB sampledRSS/
4GiB AS/256KiB output caps per process. All12 clean and2 counting processes
returned0, hard-AS active, with complete phases, summary and world-drop checks.
Clean wrapper98.086s; separate counting wrapper17.929s. These wrapper times are
not the load metric. No cache flush or allocator purge was used: cold **ownership**
means fresh-process file read plus original decode/conversion/world construction.

## Matched necessary gates

| Metric | Dense median | Tiled median | Change | Gate | Verdict |
|---|---:|---:|---:|---:|---|
| Cold input-read + decode/world construction |108.637 ms|452.555 ms|+343.919 ms / +316.6%|≤+10%|FAIL|
| Process high-water RSS at completed construction, input retained |142.625 MiB|146.500 MiB|+3.875 MiB|≤+8 MiB|PASS|

Baseline cold range7.566ms (6.964% of median); baseline construction-peak range0.
Independent recomputation matches the unchanged tool's baseline-noise policy
and classifications. Every pair is retained, including the slowest candidate:

| Pair | Order | Dense load ms | Tiled load ms |
|---|---|---:|---:|
|1|dense then tiled|109.755|453.579|
|2|tiled then dense|111.226|456.611|
|3|dense then tiled|106.448|449.501|
|4|tiled then dense|103.660|448.445|
|5|dense then tiled|107.519|558.814|
|6|tiled then dense|110.046|451.532|

The raw tool also emits route CPU/p99 classifications. They are outside this
screen's approved acceptance scope and are **not** the59-case route gates.
Do not promote their labels to a routing verdict or a full Stage A verdict.

## Allocation and resident observations, separately scoped

Separate counting runs, never included in clean timing/RSS comparisons:

| Collision storage | Dense | Tiled |
|---|---:|---:|
| Face vector capacity |65,142,784 B|0|
| Blocked vector capacity |8,142,848 B|0|
| Tile directory |0|63,616 entries /508,928 B|
| Dense tile pool |0|3,090 entries /3,559,680 B|
| Total requested elements |73,285,632 B|4,068,608 B|
| Allocator usable bytes, both buffers |73,293,792 B|4,075,488 B|
| WorldCollision header |104 B|120 B|

Requested element reduction69,217,024 B (**66.0105 MiB**) matches the counted
layout. DenseTile alignment8. Both real counting runs observe0 allocations and
0 requested bytes in the narrow warmed collision-read window. This does not
claim allocation-free routing or that the clean allocator counters are active.

The clean input-dropped/world-live milestone has median current RSS76,273,664 B
(dense) and7,020,544 B (tiled). This is a direct **microprobe phase** observation;
it is not steady deployed-host RSS, a Stage B resident win, or a per-bot saving.
Graph/bank state remains present; storage is one shared world, not multiplied by N.
No claim about the full frontend/client workload follows from these samples.

## Independent audit and evidence

Local `diagnostics/nav-stage-a-native-preparation/concord-cold-screen-01.tar.gz`:
5,169,413 bytes,720 files, SHA
`eb545cb73f725cc07e307d24853be935614d1aea0c312ca1a617a823b87f5ba6`.
Companion manifest includes every member length/hash, complete real input copies,
all14 raw process outputs/receipts, authorization/preflight, transferred source
and executables, and original qualification receipts. Root independently
streamed and verified every member, all output phases/aggregates, caps, four
admitted binaries, unchanged tool hashes, original source prefixes, and cold/peak
arithmetic. `root-cold-screen-audit.json` retains the complete raw samples.
`root-cold-original-git-audit.json` independently binds209 dense/210 tiled source
files and host helper provenance to the original Git commits. Readable small
artifacts are in `concord-cold-screen-01-metadata`.

One operational caveat is preserved: while investigating control runtime, root
checked for an owned process to stop, but all14 runs had already completed and
no signal was sent. The outside-destination control does not short-circuit at
router entry; it is not an immediate-return fixture. Its completed bounded runs
remain valid for the approved cold/peak metrics. `root-cold-interruption.json`
and `root-cold-control-readback.json` record the no-op and clarification.

The earlier failed59-case run remains separately archived/reviewed. Concord
redundant transfer/archive and failed input copies were offloaded only after
local verification; removal receipt records limited global FD visibility for
protected unrelated services. The staged original pack and admitted binaries
remain available. No failed evidence was erased or counted as a successful pair.

## Disposition

The candidate fails an existing necessary cold-load gate. Park it for campaign
performance acceptance and obtain independent review of this evidence/disposition.
Do not launch Stage B, extend the routing budget, waive +10%, or run another
sample seeking a favorable result. Any decision to accept the measured startup
tradeoff needs an explicit change to the experiment contract; none is made here.
Full59-case route CPU/p99, deployed resident savings, lifecycle and final campaign
budgets are still unresolved. Source reconciliation and the required final
whole-branch Grok4.6 review remain root responsibilities.
