# Stage A routing: bounded single-row qualification proposal

Status: DESIGN ONLY, card `t_4e422f9a`. No implementation, build, probe, SSH,
capture, retry or resource-cap increase is released by this document or its
review. Root owns subsequent freezes and releases. This proposes a new
**single-row warmed schedule**, not a completion or reconstruction of the failed
original full-corpus process.

## 1. Decision and evidence boundary

Keep the frozen tiled candidate provisionally. The operator decision recorded
at `0036143` in `nav-tiled-storage-design.md` accepts specifically the measured
343.919 ms cold-load increase. The cold report/review's original-contract park
verdict remains historical evidence; that disposition is superseded, not the
measurements. This is no CPU, latency, RSS, lifecycle or general startup waiver.

The original 59-row dense attempt remains failed/incomplete: returncode -9,
90.230536282 s wall under 90 CPU / 120 wall limits, no completed route marker or
summary and no tiled process. Final CPU and an active row/lane were not captured.
The evidence is consistent with a CPU-limit kill, not proof of the exact cause,
a tiled regression, or which warmup/timed call was running. Do not divide the
90 seconds by a presumed completed call count or use it as full-batch runtime.

The completed one-row cold screen supplies a cost warning, not route acceptance:
its 12-process clean wrapper took 98.085719729 s; counting separately took
17.929182196 s. In clean dense process 00, the receipt is 5.622581776 s wall,
summary CPU 5.408707 s and hot-route CPU 4.648116 s for 24 calls. Even the
outside-destination row is not an entry short-circuit. These local receipts were
read directly from the preserved archives, along with `root-cold-screen-audit.json`.
The reviewed original 40 + extension 19 full-byte correctness comparisons and
cold/layout/narrow-allocation results need not be repeated as broad experiments.

Frozen provenance remains:

- Tool ancestor `f24de7cbcaaa3f419fc5485b77c4a117543a6afc`.
- Dense `29b7aea779322c8611f83dc193e939ca7d756f75`, tiled
  `8385babb23fd15b876506d4a3f6154984a6b2df1`, client
  `3456edc8dabf7b25ada78110ffa56327af9f67a4`.
- Original pack 73438581 bytes, SHA256
  `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`.
- Original ordered selector file
  `diagnostics/nav-stage-a-real-proposal-01/routes.tsv`: 59 rows, SHA256
  `49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125`
  (locally rechecked). Row identity is original 1-based ordinal, not deduped tuple.
- Four existing admitted executable hashes and original source/lock bindings are
  in `nav-stage-a-first-attempt-report.md`; they remain immutable historical
  artifacts. A changed probe requires new admissions, never an edited old receipt.

Primary evidence: `nav-stage-a-first-attempt-report.md` and its review;
`nav-stage-a-cold-screen-report.md` and its review; archives
`diagnostics/nav-stage-a-native-preparation/concord-stage-a-attempt-01.tar.gz`
and `concord-cold-screen-01.tar.gz`. Readable cold audit is in the same directory.
Archive members used here include `root-real-progress.json`,
`root-cold-progress.json`, and each archive's
`host/docs/memory/nav-tiled-stage-a/<run>/real-release/00-dense.receipt.json`
(with `<run>` respectively `concord-run-01` and `concord-cold-screen-01`), plus
the latter's `00-dense.out`. Prior independent archive/source audits remain the
basis for broad provenance; this design did not rerun those audits or probes.

## 2. What can be reused, and what cannot

`stage_probe.rs:293-325` defines three distinct calls. Preserve them verbatim:

1. `find_with_model`: selector model; original default options/empty facts,
   ignoring host radius/options/bank by API definition.
2. `find_allow_teleports_with_model`: selector model and frozen facts; original
   teleport-enabled policy, not the host option bits.
3. Original `ScriptRouteRequest.calculate`: running model, selector bits,
   facts and radius, bank `[(995,10),(1712,1)]`, original essence wizard 553.

Preserve all ten fields, facts(0..6), route/result consumption and drop, graph,
node budget, tie order, errors, bank diagnosis and host expansion behavior.
The frozen router creates search-local maps/heap/set in `find_bounded_impl`;
there is no permission to cache or resume a search. A host call may perform
multiple searches. Neither search continuation nor splitting a single call is
an admissible measurement-only change.

The current probe (`stage_probe.rs:352-403`) constructs all selected requests,
runs the fixed lookup warm/timed/narrow passes, warms every selected row in lane
order 0,1,2 once, then runs eight sweeps in row/lane order. It stores call elapsed
samples but sorts them and emits only pooled p99, plus three lane CPU sums.
`stage_a.py:321-363` releases twelve processes for ONE selector file and compares
whole `hot_routes.cpu_ns`, not the sum of lane CPU measurements. Its output
parser requires the exact phase stream and summary. It cannot run a globally
bounded shard schedule or reconstruct an exact full-corpus p99 from its outputs.

Consequently, merely feeding 59 one-row files to the current `real` command is
NOT the proposal: that accidentally spends six pairs per row before feasibility,
lacks a global work cap and loses the distributions needed for pooling. The
existing bounded child runner, source/admission checks, phase ownership, native
guard tests and `paired()` classification should be reused. Do not alter the
frozen support helper or production source to add a second router/runner.

### Selected partition and changed experimental meaning

Use exactly 59 fixed singleton shards, row 1 through row 59 in original order.
Each child loads the same FULL pack and executes all three lanes: one local
three-call warmup, then eight local three-call timed sweeps. A complete logical
replicate thus has 177 warm calls and 1416 timed calls, 472 timed per lane.
There is no speed-based grouping, skipping, successful-route-only filtering or
smaller repetition count. A failed singleton stops this plan; do not automatically
split lanes, warm only the fast calls, or restart it with fewer sweeps.

This retains call multiplicity and lane sequence **within a row**, but not the
original global warmup/sweep semantics. The old schedule warms all 59 rows before
any timed call, then interleaves rows on each sweep. The new schedule repeatedly
visits a single row, with no preceding rows in that child's allocator/cache state.
World/input/request lifetimes reset per row; fewer request owners and a smaller
sample buffer are live. Page faults, allocator reuse/fragmentation, hash seeds,
CPU caches, branch predictors and repeated collision locality can differ. The
OS file cache is not flushed; neither mode is cold-filesystem evidence. Repeated
loads/lookup passes cost extra CPU and do not represent deployed per-route work.

Fresh per-call search collections do not prove equivalence of allocator or cache
state. Keeping the entire old warmup in every child would repeat the potentially
failing interval and cannot guarantee bounded progress. Fork/checkpoint, shared
world daemons, purging and warmup replay subsets introduce larger confounders;
they are rejected here. Adjacent multi-row shards would reduce reload overhead
but retain unknown expensive combinations and require another partition choice;
do not choose them after seeing favorable timings.

Root must explicitly accept this revised schedule as the Stage A routing
experiment before acceptance release. A passing result may be called only
“all-59, three-lane, single-row-warmed Stage A routing comparison.” The failed
original all-row schedule remains unqualified. If exact old temporal semantics
are required, this plan cannot deliver them within demonstrated bounds: stop for
root decision rather than claiming sharding is identical.

## 3. Smallest separately reviewed tooling change

One bounded tooling card, followed by same-card independent review, should:

- Extend the diagnostic probe only to preserve ordered raw elapsed samples in
  the summary before sorting/dropping. Use a compact fixed-capacity record for
  each timed call: implicit execution index maps to `(sweep,row,lane)`; include
  a schema and schedule identifier. Capture elapsed at precisely the current
  endpoint (including result consume/drop and existing CPU reads). Store into
  preallocated memory; no formatting, file write, flush, per-call RSS or new CPU
  syscall inside the timed interval. Keep the existing three lane CPU sums and
  `hot_routes.cpu_ns` unchanged in meaning. Output raw arrays after measurement;
  fail if the aggregate stream would exceed 256 KiB, never truncate silently.
- Add a versioned scheduler/authorization and aggregator beside the existing
  driver, sharing its guard and admission functions. Do not silently reinterpret
  old `real` authorizations or emit their generic acceptance label for diagnostics.
  A diagnostic authorization schedules one pair per row; a later acceptance
  authorization schedules all six pairs. Both enforce global bounds below.
- Bind original row ordinal, full selector-file digest, normalized shard bytes,
  shard digest, pair/arm and exact expected calls before every launch. Consume
  raw durations with exact cardinality/order checks; retain the ordinary numeric
  result aggregate per row for cross-arm/repetition equality. Aggregate checksums
  are not a replacement for the already completed full-byte correctness proof.
- Preserve all original phase and owner-drop checks. Feasibility failures may
  identify the selected singleton but not an exact active lane from a missing
  summary. Do not add per-call progress output to clean timing to improve this
  attribution. No new route optimization, alternate allocator or counting-based
  timing. Existing counting evidence stays separate; no real counting matrix here.

Both newly built clean arms must use the same reviewed instrumentation/compiler
settings and independently materialized frozen production bytes. Existing four
binaries and their qualification are not silently relabeled as the new tool.
Requalify affected generated clean/counting and guard paths, exact helper spans,
locks and new binary admissions; report which historical evidence is reused and
why. No new native real input access until root's separate authorization. Tool
build/test cost is a separately bounded tooling task, not hidden capture time.

## 4. Diagnostic feasibility, with a hard total stop

These are proposed ceilings, not current launch permission. No concurrency:
one owned probe at a time, no other campaign build/profiler/capture alongside it.
Every child retains 120 s wall, 90 s CPU, 1 GiB sampled group RSS, 4 GiB hard AS,
256 KiB combined output/file bound. A fresh native preflight must qualify hard AS
and confirm hardware identity, entitlement, available memory, swap/idle/steal and
no owned competing work; leave the resident unrelated service/settings alone.

F1: after reviewed tooling and a root freeze, run rows 1 and 2 only, once each
per arm. Row 1 order dense/tiled, row 2 tiled/dense. At most four children,
480 s supervisor elapsed and 360 CPU seconds across supervisor plus children.
Keep the first prefix because it is deterministic, not because it is a purported
fast or representative sample. Stop for evidence review even if all four finish.
F1 is a smoke/resource check; no percentile/5% verdict from one pair per row.

F2: only after F1 evidence review and another explicit root release, continue
rows 3..59 once each per arm, odd rows dense/tiled and even rows tiled/dense.
Do not redo F1 or omit its costs. F1+F2 together: at most 118 children, 1800 s
active supervisor elapsed, 1500 total CPU seconds. Review waiting time is not
active measurement time; each release records remaining cumulative budget.
The authorization carries F1 receipt hashes and consumed budgets, not a fresh
allowance. A partial prefix is an incomplete feasibility inventory, never an
all-row result. No extrapolation from that prefix can release acceptance.

For every release reserve the full next child's wall/CPU allowance from the
remaining global allowance before launch; refund unused allowance only from a
validated completed receipt. Record supervisor CPU too. An unknown final child
CPU is charged its full allowance and stops the sequence, not charged zero.
A supervisor monotonic deadline includes setup/hash/aggregation overhead and
terminates/reaps its owned active group if exhausted. Preserve output/receipts
and refuse subsequent launches. Test global accounting and cleanup independently
of the unchanged per-child guard. No automatic restart after wrapper interruption.

Stop immediately on the first material failure: nonzero exit/signal; timeout;
resource/output bound; missing/malformed phase/sample; source/input/schedule
mutation; aggregate mismatch; owner-drop failure; preflight conflict; global
budget exhaustion. Also stop after a completed child above 60 CPU seconds or
80 wall seconds: insufficient feasibility headroom, even though hard caps were
not hit. No second attempt to confirm failure and no candidate launch after a
failed dense member. Retain incomplete pairs and all prior completed rows.
One diagnostic pair cannot resolve noise or candidate acceptance; a visible
performance concern is reported for review, not averaged into a pass.

F2 success means all 59 pairs completed, every child below those headroom limits,
and every required receipt/sample validated within total bounds. It establishes
feasibility of the selected schedule only. Review the full row costs and any
candidate slowdown before spending on six-pair clean acceptance.

## 5. Later clean acceptance release and cost envelope

Only root, after F2 evidence review, may freeze a fresh clean release. Diagnostic
samples never fill acceptance slots. Pair-major schedule: for pair j=1..6 visit
rows 1..59 in order; each row runs dense/tiled for odd j and tiled/dense for even j.
Thus every row has the original six-pair AB/BA ordering, with 708 fresh children
in total. This temporal blocking differs from the original twelve processes;
retain timestamps/preflight context and all six pairs, including the slowest.

Proposed acceptance maximum: 7200 s active supervisor elapsed, 6000 total CPU
seconds, at most 708 children, unchanged per-child bounds and the same stop rules.
These are global abort ceilings, not a promise to spend two hours. No many-hour
matrix is authorized by the design or by F1 success. At the F2 review calculate
`S_wall` as the complete diagnostic schedule's active elapsed including controller
work and `S_cpu` as its supervisor+child CPU. Acceptance may be proposed only if
`1.25 * 6 * S_wall <= 7200` and `1.25 * 6 * S_cpu <= 6000`, with every row's
headroom check passing. This 25% planning margin is a falsifiable heuristic, not
a confidence bound or a noise waiver. Record uncertainty and stop at actual
bounds if the projection was optimistic. If it fails, return to root with costs;
do not raise caps, regroup rows or release just the cheap portion.

Across F1/F2 and that one acceptance release the total ceiling is 9000 active
wall seconds and 7500 CPU seconds; no unlisted pilot, retry or counting schedule.
Tool qualification and review are separately scoped prerequisites, not real
capture authorizations. Subsequent Stage B is entirely outside this envelope.

Cost expectation is uncertain rather than a fabricated precise estimate. Scaling
only the old one-row clean wrapper as an illustration gives 98.086/12 = 8.174 s
per child; 118 diagnostic children would be about 964.512 s and 708 acceptance
children about 5787.074 s (96.451 min). This assumes all rows cost like row40,
which is NOT established and may be very wrong, especially for bank/radius and
long-range cases. It includes that wrapper's overhead but not new-tool overhead.
The failed 90 s process supplies no full-corpus estimate. Multiplying only child
wall caps would permit 23.6 hours for acceptance: explicitly prohibited by the
global ceiling. Singletons bound accumulation, not an individual expensive call;
a row may still fail. F1/F2 decide whether any realistic full schedule fits.

## 6. Metrics, denominators and fail/inconclusive decisions

For each acceptance pair j and arm a, require all 59 shards first:

- `CPU[a,j] = sum(row hot_routes.cpu_ns)`. Compare total timed batch CPU (or divide
  both arms by the identical 1416 calls for presentation). Do not average row
  percentage changes, include construction/warmup/lookup CPU, subtract an estimated
  observer cost, or substitute lane CPU sums for the original batch CPU metric.
- `P99[a,j]` is nearest-rank p99 of all 1416 raw timed elapsed samples. Sort and
  take rank 1402 (1-based). Each row/lane contributes exactly eight calls; every
  lane has weight one third. No averaging or taking the maximum of shard p99s,
  no success-only denominator, no pooling across six replicates for the gate.
- Report all three lanes separately: sum their emitted CPU per pair; compute
  per-lane p99 from 472 raw calls (rank 468). Host bank internal searches are
  still ONE outer API call, not extra weighted samples. Include row-level
  durations/costs to reveal masking. Lane CPU/p99 are diagnostic breakdowns of
  the unchanged primary pooled gates, not three interchangeable implementations
  or an invented combined API. Material lane concerns require root review before
  declaring the representation worth further qualification even if pooling passes.
- Keep peak as the existing construction-complete process HWM metric, never sum
  RSS over processes. Report each row's six paired construction peaks using the
  +8 MiB gate/noise rule; no average over a failing row. End-of-routing peaks are
  additional diagnostics. Repeated cold metrics are diagnostic for stability,
  not 59 new startup waivers or an opportunity to erase the accepted old failure.
  A materially different startup result returns to root under the narrow exception.

Use the existing `stage_a.py:285-295` baseline-repeat policy on six complete
replicates: baseline center is its median; delta is candidate median minus that
center (divide by center for CPU); noise is baseline max-minus-min with the same
scale. Inconclusive if noise >= threshold or `delta-noise <= threshold <
delta+noise`; otherwise fail if delta > threshold, else pass. Thresholds remain
CPU .05, p99 2000000 ns, peak 8388608 bytes. Preserve all paired deltas and raw
values; zero relative baseline, missing data or invalid values reject the metric.
No confidence claim from eight calls per row/lane or from one feasibility pair.

A process/provenance/coverage failure stops at once. Metric decisions requiring
six pairs wait for all six rather than inventing early significance. At the
first evaluable clear gate failure stop further work and park/review acceptance;
inconclusive remains unresolved, with no automatic rerun. Never discard an
outlier or completed pair. A missing shard invalidates that whole replicate;
publish partial results as partial, never use the remaining calls' denominator.

## 7. Exact tooling validation fixtures before release

These are required future tests, not tests claimed executed on this design card:

1. Existing 17 guard tests and generated all-uniform/all-dense/gated fixtures,
   native Linux hard-AS proof, clean/System vs separate counting self-tests and
   generated complete release integration. No skipped test counts as qualification.
2. Scheduler stand-in with 59 distinct ordinal rows: assert exactly one warm
   call and eight timed calls for each `(row,lane)`, original lane order, and
   the entire F1/F2/acceptance order. Check repeated identical tuple rows remain
   separate ordinals. Reject row deletion, duplication, reordering, lane swap,
   changed facts/bits/radius/model/bank and raw-length mismatch.
3. Raw quantile fixture with 1416 values: 1401 values of 1 ns and 15 of 100 ns
   must yield 100 ns at rank 1402. Place the 15 slow values in one shard versus
   spread across shards and verify the pooled result is unchanged despite
   different shard p99s. Per-lane vector 467 ones + five hundreds yields 100 ns
   at rank 468. Retain the actual arrays and independently recomputed output.
4. CPU weighting fixture: baseline row CPU [100,1], candidate [100,2] gives
   total delta 1/101, not a mean of [0%,100%]. Pad the stand-in's other rows
   identically to test the complete denominator. Compare row/lane mapping,
   returned-error calls and bank-session outer-call counts, not successes alone.
5. Six-pair noise fixtures: baseline [100,200,100,200,100,200], candidate each
   +1 must be inconclusive at 5%; baseline six 100s with candidate six 105s
   passes at the exact boundary; six 106s fails. Exercise exact p99 +2 ms and
   peak +8 MiB boundaries and straddles; preserve the sixth slow pair.
6. Extend the existing between-launch mutation test matrix: original full TSV,
   normalized singleton, original pack/copy, schedule/authorization, tool/source,
   lock, executable and admission mutation after child, between rows, between
   pairs, across F1/F2 continuation and during final aggregation. Reject before
   the next spawn and retain the failure. Test a valid but wrong shard digest:
   fresh per-run consistency cannot replace root's original authorization.
7. Mock scheduler clock/accounting: F1 max four launches, total F1+F2 118,
   acceptance 708; exhausted wall/CPU reservations refuse the next launch;
   supervisor overhead counts; restart cannot reset spent allowance; unknown CPU
   never refunds. Test failure in first dense arm prevents its tiled partner,
   later failure preserves prefix, and global deadline reaps pipe-holding owned
   descendants. Validate output-cap overflow, truncated arrays and missing final
   summary all fail closed. Completed headroom breach must stop the next launch.
8. Generated old/new probe comparison: unchanged phase definitions, workload
   aggregates, helper spans and raw-derived summary p99 agree for both arms;
   no production function bytes differ. A new sample buffer/instrumentation
   overhead is disclosed, not “corrected” by subtracting favorable estimates.

## 8. Root freeze and remaining conclusions

Freeze new schema/method/version and reviewed tool commit; dense/tiled/client
original objects, explicit diagnostic suffixes and helper spans; resolved locks,
compiler and binary/admission hashes; pack size/hash; original TSV bytes/hash
and complete ordinal-to-normalized-shard manifest; allowed phase, exact schedule,
counts, all hard/global budgets, headroom/projection checks, metric scope and
noise method; native guard/generated receipts; clean allocator; hardware/boot/
preflight and no-concurrency context; unique owned destinations and prior-phase
receipt chain. Verify these before launch, after every child and at aggregation.
Archive every output/receipt and manifest including failures and unlaunched slots;
independently read back hashes/counts before any evidence is removed remotely.
Do not overwrite the failed or cold evidence or reuse their authorizations.

The next action after design approval is separately reviewed tooling preparation,
NOT a real run. After tool review root may release F1; after F1 review root may
release F2; after complete F2 review root may release the clean schedule if its
new-method contract and budget projection are accepted. None is automatic.

Still unqualified: original all-row temporal routing schedule; new all-59 routing
CPU/p99 until complete clean evidence review; deployed resident savings, per-bot
slope, Stage B matrix, lifecycle/owner release in the actual host, target hardware,
absolute campaign budgets and final whole-branch Grok review. The +8 MiB startup
peak, Stage B/lifecycle/absolute gates remain unchanged. A sharded microbench
cannot close them or authorize a new candidate, optimization or live capture.
