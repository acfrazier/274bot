# Coordinate-refined navigation native admission and budget continuity

Status: DESIGN / ADMISSION PLAN ONLY for the reviewed coordinate refinement at
`ebf0f30f0422229ea75de39db06944091d6497a5`. This document authorizes no tool
change, build, SSH/native action, private-pack read, real probe, acceptance run,
cap increase, retry, cleanup, remote operation, or production/default change.
Root owns every later freeze and release. Same-card independent review of this
plan is required before a tooling card.

## 1. Decision and unchanged boundary

Keep the coordinate refinement as a candidate for a new, separately identified
native qualification. Do not continue the old tiled candidate's `F2` under a new
source, and do not relabel any old source, admission, executable, authorization,
receipt, or result. The generated experiment and its review establish behavior
checksums, equal storage, a 30.828% synthetic direct-read reduction, and a 2.936%
synthetic route reduction. They do not establish native or real-pack performance,
all-59 coverage, the 5% CPU gate, the 2 ms p99 gate, RSS savings, or acceptance.

The original Stage A semantics remain fixed:

- original 73,438,581-byte pack, SHA256
  `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`;
- original ordered 59-row selector TSV, SHA256
  `49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125`;
- every row remains a distinct 1-based ordinal even when tuple bytes repeat;
- all ten selector fields, facts/state families, radius, model, bank
  `[(995,10),(1712,1)]`, essence wizard 553, result consumption/drop, graph,
  node budget, tie order, errors, and host expansion behavior remain unchanged;
- all three existing lanes remain distinct: `find_with_model`,
  `find_allow_teleports_with_model`, and the original
  `ScriptRouteRequest.calculate` path;
- one row still means one local three-call warmup followed by eight timed sweeps
  in lane order 0,1,2. No search cache, resumed search, lane split, reduced sweep,
  successful-route filtering, or alternate router is allowed.

The operator's startup decision remains narrow: it accepts only the previously
measured 343.919 ms cold-load tradeoff for the tiled representation. It is not a
new waiver for the refined source and does not change CPU, p99, +8 MiB peak,
noise, resident, per-bot, lifecycle, absolute, Stage B, or final-review gates.
A materially different later startup observation returns to root.

## 2. Exact effective source, not an `ebf0f30` tree relabel

The native candidate must be a composite source binding. Using the complete
`ebf0f30` tree as the tiled arm would silently import unrelated API/host owner
capture changes accumulated after `8385`; that is forbidden. The effective
binding is:

| Component | Exact identity |
| --- | --- |
| Dense production base | `29b7aea779322c8611f83dc193e939ca7d756f75` |
| Refined tiled production base | `8385babb23fd15b876506d4a3f6154984a6b2df1` |
| Shared client | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| Sole effective overlay | `crates/nav/src/collision.rs` from `ebf0f30f0422229ea75de39db06944091d6497a5` |
| Replaced 8385 blob / SHA256 | `7446a044e12cddf4bd2d8ff0f7093cbaea4d7af1` / `688713da89546ab61755f6cdb95714f466ff270657b5a936e53a5d2e4702b8ef` |
| Effective overlay blob / SHA256 | `07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7` / `760ae7c88c35fac6183f2db51af1f69af8d5e3225110889bc84a5a4944720b0b` |

No `collision_tiled_oracle.rs` test module, experiment artifact, API source,
host-play source, manifest, lockfile, feature, or client byte from `ebf0f30` is an
effective production overlay. All non-overlay nav/api/host helper bytes come from
the named dense or 8385 base exactly as the existing tools require.

A future reviewed tool must introduce one externally hash-pinned source-binding
record, for example schema `nav-coordinate-source-v1`, with candidate id
`coordinate-ebf0f30`, the three complete commit ids above, and the sole overlay's
path, source commit, old/new Git blob ids, old/new SHA256 values, and lengths.
The record's own SHA256 is required on every prepare/build/qualification/release
entry point. Reject an absent hash, unknown field, second overlay, wrong base
blob, whole-`ebf0f30` materialization, uncommitted path bytes, client movement, or
any effective tree mismatch.

Admissions must describe the refined arm as `base=8385 + collision overlay ebf0`
and bind the effective full source tree. They must not use `commit=ebf0f30` as if
that commit were the whole arm. A new tool hash, binding hash, source-tree hash,
lock hash, compiler identity, admission hash, or executable hash is a new object.
It never updates an existing 8385 admission in place.

## 3. Reuse the reviewed machinery by parameterization

Use the existing probes, parsers, bounded process runner, and schedule machinery;
do not fork a router, decoder, probe protocol, or second scheduler. The smallest
implementation changes are source/admission/schedule parameterization around
unchanged workload code.

### 3.1 Full-byte differential path

Parameterize these existing owned paths to consume the hash-pinned source binding:

- `docs/memory/nav-tiled-differential/harness.py`;
- `docs/memory/nav-tiled-differential/test_admission.py` and the existing guard
  tests needed to prove source mutation/relabel rejection;
- the adjacent README/audit source only where the new binding must be reported or
  independently checked.

Keep `probe.rs`, `generate.py`, the original router access bridge, exact host
source-span extraction, framing, byte comparator, limits, and one-input real
wrapper semantics unchanged. Materialization reads dense and 8385 base Git blobs,
verifies all original blob identities, then replaces only the admitted collision
blob before adding the existing probe-only suffixes. The manifest records base,
overlay, effective bytes, and adapter-only additions separately.

Fresh generated dense-versus-refined execution is required because candidate
production bytes changed. Historical generated output may remain a reference and
lineage check, but is not the new candidate's generated qualification. Before
performance measurement, a separate root release must also compare complete
fresh dense/refined output bytes for the exact ordered 59-row TSV and original
pack. The old 40-row and 19-row results remain valid 8385 evidence but may not be
concatenated, relabeled, or reported as a refined exact-59 run. No private pack is
read during tool development or generated qualification.

### 3.2 Stage A source/build/admission path

Parameterize the current reviewed singleton tool rather than changing the fixed
`frozen_support.py` constants and pretending the old arm moved. Owned code paths
for the future tooling card are limited to the necessary subset of:

- `docs/memory/nav-tiled-stage-a/frozen_support.py` — accept and verify an
  explicit structured arm binding during materialization; retain the existing
  bounded runner and Git-blob checks;
- `docs/memory/nav-tiled-stage-a/stage_a.py` — carry the binding through prepare,
  original/effective source manifests, build admissions, `verify_arm`, generated
  qualification, and release prerequisites;
- `docs/memory/nav-tiled-stage-a/sharded.py` — bind candidate generation, new
  phase namespace, prior campaign ledger, and child accounting;
- `docs/memory/nav-tiled-stage-a/qualify_sharded.py` — report and verify the new
  binding/admissions while retaining the same generated smoke and historical
  Linux-reference comparisons;
- `test_stage_a.py`, `test_sharded.py`, `test_sharded_guards.py`,
  `README-sharded.md`, and `sharded-proposed-manifest.json` only as required to
  specify and test those changes.

Keep `stage_probe.rs`, `allocator.rs`, `frozen_probe.rs`, `frozen_generate.py`,
all three route-call spans, the timed endpoint, phase meanings, raw schema/order,
layout probe, counting allocator, and selector normalization byte-identical. If
implementation discovers one of those bytes must change, stop for a new design;
it is not an incidental binding edit.

Use a new immutable protocol schema such as `stage-a-coordinate-v1`, candidate
prefix `coordinate-ebf0f30`, arms `dense` and `refined`, and phases:

- `CF1`: rows 1 and 2, full fresh dense/refined pairs in the old AB/BA order;
- `CF2`: rows 3 through 59, full fresh pairs with the old ordinal parity order;
- `CA`: the future six-pair clean acceptance schedule, not released here.

Every destination, claim, checkpoint, record, result, authorization, and admission
must contain that candidate id, full source-binding reference/hash, and phase.
Use new destinations such as `coordinate-ebf0f30-CF1-*`; never reuse an `F1`,
`F2`, `root-singleton-release-01`, or 8385 admission path. The new schema refuses
old v2 authorizations and old admissions rather than upgrading them in place.

### 3.3 Native packaging and root helpers

Parameterize, do not duplicate, the applicable root-owned helpers:

- `diagnostics/nav-stage-a-native-preparation/package_reviewed_singleton_tool.py`;
- `build_native_singleton.py`;
- `package_native_singleton_for_concord.py`;
- `qualify_native_singleton_concord.py`;
- the F1 authorization-preparation logic currently in `prepare_singleton_f1.py`.

The source package must contain the existing exact dense/8385/client Git objects
plus independently verified access to the sole refined collision blob. It must
not make the rest of the `ebf0f30` tree available as candidate production source.
Package manifests bind the reviewed tool commit separately from the effective
source binding. Builder and Concord use new roots and numbered qualification
paths; no admitted directory is rebuilt and no old builder/Concord result is
renamed.

## 4. What may and may not be reused

| Evidence or artifact | Refined-candidate use |
| --- | --- |
| Dense/8385 full-byte and native correctness results | Historical lineage and expected-behavior evidence only; not a refined exact-59 result. |
| Existing generated corpus, selector generator, probe framing, host/router spans | Reuse bytes after exact hash/span verification because their semantics do not change. Run fresh candidate outputs. |
| Linux f24 generated reference outputs | Reuse as immutable comparison references for unchanged aggregate/layout/read fields. New dense/refined clean and counting outputs must be produced and matched to them. |
| Existing Stage A bounded runner, parser, `paired()` method, raw schema, lane order | Reuse implementation and behavior after parameterization tests; new tool hashes and qualification are mandatory. |
| Old four clean/counting admissions and binaries | Do not relabel. A changed tool/source binding requires new effective manifests and all four fresh admissions; byte coincidence alone is not an admission. |
| Old F1 refined-arm positions | Never reuse: they executed 8385, not the refined collision source. |
| Old F1 dense rows 1/2 | May inform diagnostic planning and show historical dense headroom/checksum on the same pack/selectors. They are not temporally fresh pairs with a refined arm and do not fill `CF1`, all-59, or acceptance slots. |
| Generated coordinate experiment samples | Support the hypothesis and source-review decision only; never fill native, exact-59, `CF1`/`CF2`, or clean acceptance slots. |

In particular, pairing a later refined row 1/2 child with an old dense child may
be shown as an explicitly historical, unpaired feasibility comparison. It cannot
be fed into the pair aggregator, noise method, p99 population, all-59 completion,
or a fresh clean result. All 708 eventual `CA` children must be fresh under one
refined binding; no old dense or tiled measurement fills an acceptance slot.

## 5. Required generated and independent validation

A future tooling implementation must complete and retain all of the following
before any native/private input release:

1. Source-binding fixtures: exact dense/8385/client materialization; sole collision
   overlay; rejection of whole-ebf source, extra/missing overlay, wrong old/new
   blob, moved client/base, path traversal/symlink, source mutation, and admission
   relabel. Independent readback compares every effective file to either its base
   Git blob or the one admitted overlay blob.
2. Existing coordinate source tests: complete nav library and nav-pack tests,
   dense oracle, all 256 faces and both blocked values, edges/partial tiles,
   planes/origins/unknown levels, raw-flags variants, exact panic payloads,
   layout equality, and no added direct-read allocation. These generated results
   are prerequisites, not native performance evidence.
3. Fresh full-byte generated differential: dense versus refined full output bytes,
   all current generated corpus/guard/admission/audit checks, and exact effective
   source manifests. No historical output hash substitutes for this run.
4. Stage A generated clean and counting qualification for all-uniform, all-dense,
   and gated fixtures, with equal aggregates, logical counts, checksums, layout,
   zero warmed narrow allocations, and exact raw cardinality/order.
5. Scheduler tests covering the complete `CF1`/`CF2`/`CA` order, 59 ordinals,
   candidate id on every record, old-schema/admission rejection, between-launch
   mutation, setup alarms, output bounds, cleanup, raw p99, weighted CPU, noise,
   headroom, publication, and immutable prior-ledger accounting.
6. An explicit budget fixture seeded with the accepted old F1 receipt must prove
   the old 40.090448 wall, 29.501340 CPU, and four children cannot reset. With the
   current 118-child ceiling it must refuse a fresh 118-child refined completion,
   not silently omit rows or consume old dense/tiled slots.
7. The existing four-child tiny generated scheduler smoke and the twelve Linux
   f24 generated-output comparisons, now bound to the refined source record and
   new admissions. Exact unchanged `route_calls`, `warm_timed`, and owner/drop
   helper spans remain required.
8. Same-card `reviewer` inspection of the minimal tool commit and all generated
   evidence. Review must independently reconstruct the composite source, verify
   that old admissions/results are unchanged, reproduce the child-gap assertion,
   and confirm no real-pack path was opened.

Only after that review may root package for Linux. Native generated qualification
then requires fresh source-package verification, four new clean/counting
admissions, all existing legacy and scheduler guards with zero skips, hard 4 GiB
AS proof, generated clean/counting/integration/scheduler checks, exact helper
spans, and independent archive readback. Reusing f24 reference outputs is labeled
reference reuse; every refined output and admission is fresh.

Before any private full-byte or performance run, root additionally verifies the
reviewed tool commit and source-binding hash, effective source and lock manifests,
all four admissions/binaries, native qualification receipts, exact pack and
59-row hashes, new destinations, and the applicable review/budget-decision hashes.
Authorization/source/native checks occur before stat/open of the private pack.
A fresh hardware/boot/memory/swap/one-second-idle/zero-steal/no-conflict preflight
is mandatory. Run one owned probe at a time, with no concurrent campaign build,
profiler, capture, or other benchmark. Preserve every failure and do not retry
or advance automatically.

## 6. Campaign ledger and the blocking child-count decision

Accepted old 8385 F1 spending is immutable campaign cost:

| Ledger item | Wall seconds | CPU seconds | Real children |
| --- | ---: | ---: | ---: |
| Already spent old F1 | 40.090448 | 29.501340 | 4 |
| Existing cumulative ceiling | 1800 | 1500 | 118 |
| Remaining now | 1759.909552 | 1470.498660 | 114 |

A scientifically clean refined all-59 feasibility inventory needs 59 fresh
pairs, or 118 new children. Cumulative child use would be `4 + 118 = 122`, which
is four above the existing 118-child ceiling. This is a mathematical admission
block independent of the remaining wall/CPU observations.

Reusing only the two unchanged old dense row-1/2 children would still require 116
new children: 59 refined arms plus fresh dense arms for rows 3..59. Cumulative use
would be 120, still two above the ceiling, and the two reused rows would not be
fresh pairs. Therefore neither four old children nor a mixed two-child reuse can
be combined into a refined all-59 success.

Generated/tool/native-qualification children are separately bounded tooling cost,
as in the reviewed protocol; they are not real measurement slots and cannot be
called `CF1`/`CF2`. All their wall/CPU/storage remains disclosed separately.

The next executable stage is thus bounded tooling parameterization plus generated
and independent review only. No real `CF1` should start before root resolves this
binary choice in a separate, explicit, reviewed decision:

1. Preserve the existing 118-child ceiling: issue no refined real-phase
   authorization and make no all-59 claim. Root may park refined native
   measurement or separately decide whether the unchanged old 8385 `F2` should
   use its remaining 114 slots under the old v2 chain. Refined qualification does
   not authorize or rename that continuation.
2. If the operator/root wants a fresh refined all-59 inventory, explicitly review
   and authorize a four-child-only cumulative exception from 118 to 122. That
   later decision must name `coordinate-ebf0f30`, retain the old four-child spend,
   leave 1800 wall / 1500 CPU, every per-child limit, headroom, stop rule, and all
   acceptance ceilings unchanged, and require a new schedule-tool review. This
   plan does not grant that exception or permit code to default to it.

Do not choose a passing prefix, two historical dense rows, candidate-only rows,
or a smaller selector set as campaign acceptance. If no exception is approved,
the refined real all-59 path is blocked by policy, not partially successful.

## 7. Real phases if and only if the decision permits them

After every correctness/native prerequisite and the explicit child decision,
root may separately release `CF1` and later `CF2`; neither is automatic.

`CF1` runs rows 1/2 as four fresh children: row 1 dense/refined, row 2
refined/dense. It starts its campaign budget at the old F1 totals, not zero, and
records both candidate-phase and campaign-cumulative wall, CPU, and child counts.
Stop for evidence review even if all four finish. There is no percentile or 5%
verdict from this phase.

Only after accepted `CF1` evidence may root release `CF2`: rows 3..59 once per
arm, odd dense/refined and even refined/dense. The authorization carries the old
F1 ledger, complete `CF1` receipt/authorization hashes, actual spent totals, the
reviewed source binding, and a fresh preflight. It never points to old `F1` as a
same-candidate parent. The existing 120 wall / 90 CPU / 1 GiB sampled group RSS /
4 GiB hard AS / 256 KiB output limits, 80 wall / 60 CPU headroom, reservation,
unknown-charge, setup/deadline, mutation, cleanup, and first-failure stop rules
remain exact. Wall and CPU have no exception: all new setup, idle samples,
supervision, children, hashing, aggregation, and publication are charged on top
of 40.090448 / 29.501340 and must stay within 1800 / 1500.

Complete `CF1+CF2` means 118 fresh refined-candidate children and all 59 pairs,
not an acceptance result. A partial prefix remains partial. Review full row/lane
costs and candidate concerns before any clean release.

For a later `CA` proposal, compute the 1.25×6 feasibility projection from the
complete refined `CF1+CF2` active wall and CPU because those are the relevant
candidate-method costs. Separately retain old F1 as spent campaign cost and
require:

- refined acceptance itself at most 7200 wall / 6000 CPU and exactly 708 fresh
  children under the existing pair-major six-pair schedule;
- old F1 + refined feasibility + refined acceptance at most 9000 active wall /
  7500 CPU;
- no child-count exception for diagnostic work to increase the 708 acceptance
  slots or any per-child cap.

A complete fresh `CA`, independent evidence review, and unchanged gate math are
still required for route CPU/p99/peak conclusions. Diagnostic or mixed-candidate
records never fill those slots.

## 8. Remaining campaign work

Even successful refined full-byte correctness, native qualification, `CF1`, and
`CF2` would leave clean six-pair Stage A acceptance open. The narrow startup
tradeoff remains the only accepted exception. Deployed resident RSS, absolute
RSS, per-bot slope/CPU, latency/responsiveness, Stage B, lifecycle/owner release,
target capacity, production/default decisions, and final whole-branch Grok 4.6
review all remain open. No result from this plan changes those boundaries.
