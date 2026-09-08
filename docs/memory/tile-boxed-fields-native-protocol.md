# Private tile field boxing: bounded native validation protocol

Status: predeclared before candidate completion or any candidate native launch.
The implementation and source integration review remain prerequisites.

## Frozen reference and scope

Baseline host9268890217d968cfeb7c66ebb11dd5c3dd2c084f,
clientabb811bd0afa1acd99319ccd5bc36bfb241080f9; Windows binary
SHA e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5.
Use the existing verified native build receipts and frozen source package.
Candidate is the reviewed descendant on codex/windows-render-owner-census.
Before launch, record its exact host/client commits, all source hashes,
native pre/post identity, binary hash and review verdicts in a companion freeze.
Do not substitute a moving checkout or claim provenance from branch names.

The intended change is seven private Option<Box<SceneModel>> tile fields.
Public enum/API and sprite storage stay unchanged. Candidate accounting must
include every newly allocated SceneModel box in both ordinary and linked grids,
separately from containers, nested arrays and unique Arc pointees. Measure actual
native size/alignment; the design estimate of64B is not a native result.

## Release gates

1. Same-card per-task review of committed client and host source, affected crate
   tests, separate client integration tests and reported behavior coverage.
2. Freeze the combined source and obtain an integration milestone review before
   expensive native comparisons, per docs/execution.md. Final campaign review
   remains separate.
3. Native build and source/binary verification, native layout and meaningful
   linked/all-variant/COW regression evidence. Inspect actual captures for
   scene/overlay correctness; preserve last-FBO freeze, CPU fallback and
   focus/attach/detach behavior proofs as distinct gates.
4. Full native no-launch controller/parser/spec/fixture/server/source/argv
   validation with current managed-helper identities, then one declared launch
   per cell. Archive original receipts, raw run directories and failures.

## Screening order and controls

Begin N16 focused-plus-background at30s warmup/120s observation/60s teardown,
using baseline then candidate, followed by candidate then baseline. Use the same
verified server/cache/nav/catalog/loadout, desktop session, frontend geometry,
requested and actual Intel Vulkan adapter, render policy and accounting mode.
Fresh preflight is required before each cell. Build VM must be Off with zero
assigned RAM; no native compilation, other tests or profiling overlaps clean
cells. Runtime owner census stays off for CPU/latency screens. Any separate
owner-census observation is diagnostic and cannot become a clean cell.

After the background screen supports proceeding, check N16 focused-one with a
fresh baseline/candidate pair under the same settings. A proposed retention must
also address focused-mode regressions. Use reverse order or the single longer
confirmation to resolve a named order/readiness confounder; no indefinite reruns.
One longer confirmation stage is available only for a candidate worth retaining.
Final1/16, scaling and lifecycle schedule remains in performance-finish-plan.md.

## Evidence and decisions

Every completed cell must independently pass native binding and per-slot workload
qualification. Process exit0 alone is insufficient. Record actual ready/rendered
counts, paint and simulation cadence, per-slot progress and fixture identity.
Failed cells remain failures; do not average them into a passing comparison.

Report whole-observation metrics and the common harness-elapsed overlap for each
pair, with sample counts and explicit start/end ranges. This addresses the known
readiness/falling-RSS confounder from prior lazy-upload screens. If overlap is
absent or short, label comparison insufficient and investigate that named cause.
Do not equate matching observation lengths with matching time since startup.

Report median/peak current RSS, private commit separately, CPU seconds/wall time,
p99 simulation/script/input/frame latencies where instrumented, completed frame
cadence, and any unavailable measures. Do not subtract logical bytes, allocator
counts, private commit or GPU memory from RSS. Diagnostic owner changes establish
storage accounting only. Additional box headers/allocator rounding stay unmeasured
unless directly observed.

Retain only on intended improvement beyond observed variation or concrete
allocation removal, with clean CPU non-regression within5% and p99 latency within
2ms. Missing/ambiguous regression evidence remains pending. Review recomputed raw
results independently before accepting a measurement conclusion. Absolute budget,
all-mode lifecycle/scaling and final Grok4.6 requirements are unchanged.
