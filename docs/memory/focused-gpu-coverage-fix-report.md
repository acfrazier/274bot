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
headless zero-counter/coverage shape. Counter, pending, loss/drop, histogram,
coverage, backend, and existing callback-delivery cadence gates remain on the
required GPU rows. Historical metadata without a render-policy declaration
retains the prior all-slots behavior.

Evidence and provenance

The approved audit (tile-focused-gpu-coverage-audit.md, audit id 4af1348)
records read-only recomputation of these six short native-bound archives:

- baseline-focused-one-nativecheck-20260908-0103
- candidate-focused-one-nativecheck-20260908-0103
- baseline-focused-plus-background-nativecheck-20260908-0103
- candidate-focused-plus-background-nativecheck-20260908-0103
- baseline-focused-plus-background-reverse-20260908-0116
- candidate-focused-plus-background-reverse-20260908-0116

The raw archive directory is not present in this isolated checkout, so this
implementation does not reopen, alter, or manufacture archive data. The audit's
original native-bound provenance and limits are retained; no local Windows path
binding or acceptance result is asserted.

Reader version: focused-gpu-role-contract-v1
Reader source SHA-256 after change:
1513552a59f73601c5084706d8b264a9189bd90caa5993d5a8d1dd319926ca4d

Verification

- FocusedGpuRoleCoverageTests: 4 tests passed.
- Full test_reference_metrics discovery: 89 passed; 1 pre-existing missing
  diagnostic fixture error and 1 pre-existing missing reviewed-cell failure.
- git diff --check: passed.

Changed files are limited to reference_metrics.py, test_reference_metrics.py,
and this report.
