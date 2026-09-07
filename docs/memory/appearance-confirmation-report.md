# Appearance packet storage: longer resource observation

The single longer N16 pair observed 58.40625 MiB lower median host RSS and
0.39664% higher CPU with boxed appearance packets. Earlier short replication
remains inconclusive at N1 and N16. Retain the implementation provisionally for
its concrete allocation removal and reviewed behavior tests; park RSS and CPU
acceptance claims. No further appearance-only comparison is queued.

## Evidence and provenance

Batch `diagnostics/appearance-resource-confirmation-20260907T112124Z/batch.json`
predeclared two cells, reference then candidate, with 120s warmup, 600s observation
and the existing 60s normal teardown. Both completed exit 0 on the first attempt.
Raw runs are `20260907T112710Z_tui_n16_active` and
`20260907T114207Z_tui_n16_active`. Both independently bind workload, native
identity and continuous managed accounting with no qualification errors.

The frozen manifest is
`diagnostics/matched-appearance-build-20260907T103345Z/build-manifest.json`.
Its reference explicitly reuses the previous shared-navigation candidate;
both sides include the TUI input-origin correction. Reference TUI SHA256 is
`eaaa45a718ca340f808e3467c469ff804e6875d7425fe41edfc715a698dd53a1`;
candidate is `9eb187b7064e0b1bf10bc82722d2ed1210189ee0a87987399006591dcad8bdc7`.
Client commits are `451759f2` and `85266df`; host source content is identical.
The [short report](appearance-short-resource-report.md) records build lineage,
implementation review and affected-test results.

Both cells use real PTY TUI, 16 sustained active Thievers and input probes.
Hot profiles, allocation counting, verbose/native captures and diagnostic
sidecars are OFF. Immutable server/host-condition sidecars match the short
batch. Quiet-board snapshots precede each launch: no campaign builds, tests or
reviewers run during observation. This is not a machine-idle claim.
`preparation-environment.json` records unrelated desktop activity before the
pair; it neither quantifies its effect nor reconstructs earlier conditions.
The game server and helpers are accounted separately.

Reproduce with `python3 docs/memory/diagnostics/appearance-resource-confirmation-20260907T112124Z/analyze_pair.py`.
The helper freshly binds both receipts against explicit manifest roles and
sidecars, checks distinct chronological runs, profile settings, native/runtime
matching and process identities. Only the expected, already role-bound client
source hash differs across runtime keys. It writes
`descriptive-pair-results.json` and changes no existing acceptance gate.
Current profile-ON overhead calibration remains pending; this is a metric-scoped
descriptive comparison.

## Observed resources and progress

| Host metric | Reference | Candidate |
|:---|---:|---:|
| Sampled observation, seconds | 598.918583 | 599.132115 |
| Median resident memory, MiB | 877.953125 | 819.546875 |
| Maximum sampled resident memory in observation, MiB | 938.421875 | 863.390625 |
| Reported resident high-water at observation, MiB | 938.500000 | 863.421875 |
| Average process CPU cores | 0.299569270 | 0.300757492 |
| Fleet mean client iterations/slot/s | 41.190595 | 42.670367 |
| Per-slot steal gains, minimum–maximum | 44–78 | 50–89 |

All 16 slots qualify and make positive progress. The fleet iteration mean does
not establish every-slot cadence or p99 intervals. Each final teardown sample
has zero active scripts, live isolates and in-flight snapshot bytes/capacity.
Both frontends exit normally. This does not replace repeated lifecycle plateau
qualification. Observed maxima/high-water are not a separately qualified
startup/transition peak result.

Server median RSS is 1057.359375/1057.390625 MiB and average CPU is
0.040979787/0.039525490 core, reference/candidate respectively. The JSON retains
each helper's identity, median/peak resident memory and CPU interval; these are
not added to or substituted for host RSS.

## Decision and remaining work

This one pair has no repeat-variance estimate. The earlier N16 short range
overlap and 71.171875 MiB maximum within-role RSS spread remain visible; N1
short results also remain inconclusive. The longer CPU point difference is
inside 5%, but does not erase short CPU drift or prove non-regression. We do
not apportion measured RSS difference to the structural table-size change.

Under approved performance-finish-plan section 5, the 4.109375 MiB per-client
empty-table allocation removal and reviewed behavior tests support provisional
retention while regression evidence remains pending. Occupied boxes have their
own storage and overhead. Allocation removal is not accepted RSS savings.
The paired screens and one longer stage are complete; park performance claims
instead of repeating until favorable. Broader scheduling/desktop noise remains
an uncertainty, not an established causal explanation.

Candidate N16 median RSS remains 307.546875 MiB above the 512 MiB TUI target.
Current profile-ON overhead and latency non-regression, absolute resource and
responsiveness targets, Linux/reference hardware, both panel modes, rendering,
full lifecycle/scaling checks and whole-branch Grok4.6 remain pending.
`accepted_rss_saving`, `performance_acceptance` and `final_acceptance` are false.
