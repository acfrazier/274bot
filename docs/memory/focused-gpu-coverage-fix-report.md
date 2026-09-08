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
enabled/backend state, all counters, loss/drop/coverage flags, renderer backend
identity, residency epoch counters, and both histograms at every epoch; counters
and histograms must be monotonic across the complete contained sequence. A
normal interior in-flight pending callback may temporarily make completion
coverage incomplete, but endpoint pending remains unavailable. Historical
metadata without a render-policy declaration retains the prior all-slots behavior.

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
`/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f/diagnostics/windows-tile-boxed-clean-20260908`.
All six named raw-run-01 archives were reopened with the public `evaluate_gpu`
entry point using the parent reader (`e852262^`, source SHA-256
`ee9e5589ae7d8b6d900a838b477fb51b4a0d7923c547b1ef52f18c798983c10b`) and the
candidate reader (source SHA-256
`33fcbad168a3d68cb99d4989b7b4878c6da8468d1348698fc93afc1e405b238d`). The six
windows contained 118–120 observe samples (see the integration check below). The exact old-status/reason to
new-status/verdict results were:

- baseline-focused-one-nativecheck-20260908-0103: unavailable / focused_one_role_contract_failed -> available / meet (1 required GPU row)
- candidate-focused-one-nativecheck-20260908-0103: unavailable / focused_one_role_contract_failed -> available / meet (1 required GPU row)
- baseline-focused-plus-background-nativecheck-20260908-0103: unavailable / coverage_lost_or_incomplete -> available / meet (16 required GPU rows)
- candidate-focused-plus-background-nativecheck-20260908-0103: unavailable / coverage_lost_or_incomplete -> available / meet (16 required GPU rows)
- baseline-focused-plus-background-reverse-20260908-0116: unavailable / coverage_lost_or_incomplete -> available / meet (16 required GPU rows)
- candidate-focused-plus-background-reverse-20260908-0116: unavailable / coverage_lost_or_incomplete -> available / meet (16 required GPU rows)

These are reader recomputations only. They retain the archives' original
native-bound provenance and do not manufacture a local Windows path binding or
make a native, visual, performance, or final acceptance claim.

Reader version: focused-gpu-role-contract-v2
Reader source SHA-256 after change:
33fcbad168a3d68cb99d4989b7b4878c6da8468d1348698fc93afc1e405b238d

Verification

- Targeted metrics/adapter/overhead suites: 87 passed, 2 skipped.
- Full memory unittest discovery: 434 tests; 1 pre-existing missing diagnostic
  fixture error, 1 pre-existing missing reviewed-cell failure, and 8 skips.
- git diff --check: passed.

Changed files are limited to reference_metrics.py, test_reference_metrics.py,
and this report.

Primary integration check (root, 2026-09-08)

Integrated the four reviewed source commits as 0f25930, d22d733, b49fcae,
and ec2d716, excluding the isolated setup/STATE commit. Grok 4.5/xai-oauth
review session 20260908_041158_98ce2b approved final source 8100b8f. The
integrated reader SHA-256 remains 33fcbad168a3d68cb99d4989b7b4878c6da8468d1348698fc93afc1e405b238d.

The earlier comparison above used an intermediate rejected reader as its old
side. Root additionally compared the actual primary reader at aca71ac
(SHA-256 8ec0e48ae54324dc26ea5b07d6a87747a4568018c5f182d206881d05fac4bb17)
with the final reader, using the same six untouched native raw directories:

| Cell | Observe samples | Original primary | Integrated reader |
| --- | ---: | --- | --- |
| baseline focused-one nativecheck 0103 | 118 | unavailable / no_complete_qualified_stable_interval | available / meet |
| candidate focused-one nativecheck 0103 | 119 | unavailable / no_complete_qualified_stable_interval | available / meet |
| baseline focused-plus-background nativecheck 0103 | 119 | available / meet | available / meet |
| candidate focused-plus-background nativecheck 0103 | 119 | available / meet | available / meet |
| baseline focused-plus-background reverse 0116 | 119 | available / meet | available / meet |
| candidate focused-plus-background reverse 0116 | 120 | available / meet | available / meet |

Full old/new public evaluate_gpu results and original metadata/sample hashes
are retained in diagnostics/focused-gpu-coverage-integration-8100b8f/original-vs-final.json.
These are offline callback-delivery cadence results, not scanout, instrumentation
calibration, native visual proof, paired latency regression acceptance or an RSS
saving. The focused RSS claim remains parked.

Root ran the complete test_reference_metrics, test_matched_evidence_adapter and
test_instrumentation_overhead modules **on primary after integration**: 175 tests
passed, with no failures or skips. The isolated checkout's two absent-fixture
failures did not reproduce with the primary checkout's available fixtures.
No Rust or client source changed in this reader integration.
