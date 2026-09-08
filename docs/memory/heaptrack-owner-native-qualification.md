# Native Heaptrack owner replay qualification

Status: failed and unreleased.

## Scope and identity

One finite fixture-only Linux qualification was launched on Concord using the reviewed staged tree at `/home/acfrazier/owner-native-f097fdc`. The reviewed driver was run unchanged, without `--portable-probe`, against the exact reviewed executable hashes and a fresh absent output directory. No production trace, production manifest, previous capture/replay directory, rebuild, source edit, retry, cap increase, or alternate executor was used.

- Source/report correction commit: `f097fdcd6e0565dd332b3d68940317d48b3a3664`
- Stage manifest SHA-256: `56025edaa3d7d958ab4a00692c80f3480a3a9aaa289c6228453688bdb717be9b`
- Child SHA-256: `12eb47064705221514b2c638211c77cefb2a14fb89d2df7c61210ad787469221`
- Fixture SHA-256: `6822a2beaca8ab6c74152c3a1d96bd6e78e1ab978b238c5901edae4eddaee19d`
- Core-test SHA-256: `b76b840233cd19adf6c42cb91a377c861418e39ea5b940af7d7cbb691db10559`
- Host: Ubuntu 24.04, x86_64

Admission at `2026-09-08T23:01:03Z` recorded 1,001,304,064 available bytes, 11,827,228,672 free disk bytes, and no conflicts. The qualification output path was absent before launch.

## Actual execution

The exact driver exited 1 at `2026-09-08T23:01:05Z`. Wrapper wall time was 1.42387 seconds; the driver recorded 1.1421050770004513 seconds. The driver stopped at the native core-test gate before the Python/native suite or any fixture replay.

Core tests: 11 total, 9 passed, 2 failed, 0 skipped.

Failed tests:

1. `core::migration_tests::frozen_identity_hash_suffix_and_midpass_mutation`
   - `src/migration_tests.rs:68`
   - Assertion failed: `stream.finish(&mut g).is_err()`.
2. `core::migration_tests::interpreted_mutation_between_passes`
   - `src/migration_tests.rs:75`
   - `PoisonError` after the preceding test failure.

The qualification JSON records the exact native-test output and failure reason. The remote output is 2,220 bytes with SHA-256 `47ab6b8515b244b82d3fbda7bd2e4365090abc27cb216f9d811a11d0da9e7f74`. The owned native child/core-test processes and qualification process were absent after completion.

## Required measurements and coverage

No Python/native tests ran, so there is no native test count beyond the 11 core tests above. No original-case mapping, migration suite, saved-smoke replay, generated churn case, negative guard cell, phase measurement, sample interval, population, capacity, or throughput result exists.

The required necessary rates remain the reviewed values: raw `11.938945 MB/CPU-s`, interpreted pass 1 `4.503128 MB/CPU-s`, and interpreted pass 2/selected `4.503128 MB/CPU-s`. No phase CPU/byte rows were emitted, so there are no rates or medians to independently recompute. The earlier erroneous `3.175` value was not used.

Because the failure occurred in correctness/identity migration coverage, this qualification is blocked. It is not readiness evidence for throughput and does not establish production capacity, owner attribution, accepted memory savings, or permission for a production retry. Root must make a new explicit one-attempt decision after resolving and reviewing this failure; this result does not authorize that decision.

## Evidence files

- `diagnostics/replay-native-f097fdc-qualification-1/qualification.json`
- `diagnostics/replay-native-f097fdc-qualification-1/admission.json`
- `diagnostics/replay-native-f097fdc-qualification-1/identity.json`
- `diagnostics/replay-native-f097fdc-qualification-1/completion.json`
- `docs/memory/heaptrack-owner-native-qualification.json`

Earlier failure evidence and campaign state were not modified.
