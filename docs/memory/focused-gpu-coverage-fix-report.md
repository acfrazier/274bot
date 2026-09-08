Focused-one GPU coverage reader correction

Status: implementation and offline unit verification only. This report makes no
native, visual, performance, or final acceptance claim.

Scope

The public evaluate_gpu path now honors an explicit render_policy declaration
while preserving the complete (slot_id, generation) identity set. Declared
focused-one requires one stable focused_full_rate GPU row and N-1 explicit
expected_headless rows. Declared focused-plus-background requires one focused
GPU row and N-1 background GPU rows. Headless rows are reported as a separate
role and do not receive FPS or p99 metrics.

The role contract is checked at every contained observe sample, including
freshness, slot identity/generation, focused identity, renderer shape, and
headless zero-counter/coverage shape. Required GPU rows additionally validate
enabled/backend state, all counters, pending/loss/drop/coverage flags, and both
histograms at every epoch; counters and histograms must be monotonic across
the complete contained sequence. Historical metadata without a render-policy
declaration retains the prior all-slots behavior.

Evidence and provenance

The approved audit (tile-focused-gpu-coverage-audit.md, audit id 4af1348)
records read-only recomputation of these six short native-bound archives:

- baseline-focused-one-nativecheck-20260908-0103
- candidate-focused-one-nativecheck-20260908-0103
- baseline-focused-plus-background-nativecheck-20260908-0103
- candidate-focused-plus-background-nativecheck-20260908-0103
- baseline-focused-plus-background-reverse-20260908-0116
- candidate-focused-plus-background-reverse-20260908-0116

The approved primary diagnostics directory was checked read-only at
`/Users/acfrazier/experiments/274bot/docs/memory/diagnostics/windows-tile-boxed-clean-20260908`
and the named archive directory was not present in this checkout or its
available worktrees. Therefore no before/after archive recomputation is
asserted here: the audit's original native-bound provenance and limits are
retained, with no local Windows path binding or acceptance result manufactured.

Reader version: focused-gpu-role-contract-v2
Reader source SHA-256 after change:
ee9e5589ae7d8b6d900a838b477fb51b4a0d7923c547b1ef52f18c798983c10b

Verification

- FocusedGpuRoleCoverageTests: 7 tests passed, including public evaluate_gpu
  adversarial interior epoch and headless-shape cases.
- Targeted metrics/adapter/overhead suites: 82 passed, 2 skipped.
- Full memory unittest discovery: 429 tests; 1 pre-existing missing diagnostic
  fixture error, 1 pre-existing missing reviewed-cell failure, and 8 skips.
- git diff --check: passed.

Changed files are limited to reference_metrics.py, test_reference_metrics.py,
and this report.
