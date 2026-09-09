# Scheduler matrix fixture cost report

## Change

`test_sharded.Metrics.test_full_generated_protocol` now uses a test-local
`matrix_fixture_doubles` context. It caches repeated reads of immutable,
hash-bound authorization/checkpoint history, replaces repeated generated
checkpoint durability (`fsync` + rename) with a bounded direct write, and
replaces repeated whole-release storage traversal with a constant-time seam
that records calls and validates the generated destination shape.

The double is limited to the 826-slot generated arithmetic fixture. The test
still invokes the real `run_phase` implementation for all 59 ordinal rows and
all phase counts (F1=4, F2=114, acceptance=708), preserves launch ordering,
continuation authorization, output aggregation, and budget/deadline logic.
It does not claim native enforcement or real-I/O performance.

## Focused unmocked coverage retained

The two scheduler test modules continue to exercise the actual fail-closed
implementations outside the full matrix: atomic `write_json` and `read_ref`
admission, storage size/free-space and symlink checks, output/hash mutation,
wrong shard and repeated ordinals, native idle preflight, inherited address
binding, global wall/CPU deadlines, owned descendant cleanup, reservation and
continuation budget exhaustion, and no-restart behavior. The full matrix also
asserts all 826 launch tuples and records that the storage seam was called.

## Observed fixture cost

| Run | Result | Time |
| --- | --- | ---: |
| Existing pre-change full matrix log (`16-full-matrix.log`) | pass | 86.732 s unittest elapsed |
| Current local full matrix | pass | 51.728 s unittest elapsed; 51.78 s wall |

The current `/usr/bin/time -p` sample used 23.00 s user CPU and 28.58 s
system CPU. These are macOS-local observations, not proof of Concord's
2-CPU/360-wall/300-CPU qualification limits. The pre-change sample is retained
as historical evidence; no failed result or native cap was changed.

## Verification

- `python3 -m unittest -v test_sharded.Metrics.test_full_generated_protocol` — pass.
- `python3 -m unittest -v test_sharded test_sharded_guards` — 24 tests pass.
- No production scheduler/source file, qualification limit, ordinal, schema,
or frozen helper was changed.

Evidence: `diagnostics/nav-scheduler-matrix-fixture-cost/fixture-cost.json`.
