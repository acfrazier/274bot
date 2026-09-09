Scheduler matrix fixture correction report

Scope

Only docs/memory/nav-tiled-stage-a/test_sharded.py and the owned scheduler-fixture-correction diagnostics were changed. No production scheduler, Rust, frozen helper, qualification limit, timeout, resource cap, matrix row, sweep, lane, or native input was changed.

Correction

matrix_fixture_doubles now keys its read_ref cache by the complete bounded reference identity: path, expected SHA-256, and admission cap. It still performs the real admission on first observation. The double remains confined to the two generated full-matrix tests and substitutes only their repeated fixture JSON/storage work.

After each full matrix exits the context, the tests assert that read_ref, write_json, storage_guard, and stage.admit are restored to their original callables. The fixture admission double caches only the bounded identity (path, expected SHA-256, cap), performs the real admission on first observation, and records only a scalar cache-hit count plus bounded first-check identities. The structural evidence was 5,032 underlying admission calls with 2,844,655 cache hits for the legacy matrix and 5,032 underlying calls with 3,013,143 cache hits for the coordinate matrix.

`audit_generated_matrix` then runs the restored real read_ref, bind_entries, collect, and stage.admit over every retained phase result: F1/F2/acceptance and CF1/CF2/CA. The coordinate audit also reloads and rebinds the source binding. Each audit records real admissions and asserts that the count covers the three complete generated phase schedules. A post-cache mutation of both the first final output and its record is then rejected by the real SHA-256 admission path.

Preserved invariants

- Legacy launches remain 826 in full ordinal/ABBA order.
- Coordinate launches remain 826 in full ordinal/ABBA order.
- Legacy final completed count remains 708.
- Coordinate final completed count remains 708 with cumulative children 830.
- Existing scheduler state machine, aggregation, budget/deadline, identity, source-binding, and focused real admission/mutation/TOCTOU tests remain exercised.

Verification

Command:

  python3 -m unittest -v test_sharded test_sharded_guards

Result: OK; 29 tests passed in 30.085 seconds. Raw output:

  diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction/full-generated-suite.log
  SHA-256: fa9489543ecb21629ae7da260e4b4163224d117c13c39ba55083576de196564c

The two full matrix tests were also run directly before the full suite:

  python3 -m unittest -v test_sharded.Metrics.test_full_generated_protocol test_sharded.Metrics.test_full_coordinate_generated_protocol

Result: OK; 2 tests passed in 27.479 seconds. The output included the structural counts above; the scalar hit counter does not retain one object per hit.

The same full affected suite was exercised through `frozen_support.bounded` with the unchanged `QUAL_LIMITS` (`wall=360`, `cpu=300`, `rss=512 MiB`, `address=4 GiB`, `output=1 MiB`): return code 0, 29 tests, wall 31.0476 seconds, sampled group peak RSS 98,795,520 bytes. Receipt:

  diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction/bounded/runner-result.json

Source hashes at verification:

  test_sharded.py         b39ee71d0778488c09ea6e4e97a08eeb995ab195296436a20160f433cfd3ec4d
  test_sharded_guards.py  45dd91f94b7c9128647e26d243666fd7ea658a82dce56c4dd19b6d2b01cae15b
  sharded.py              f4b7cc0bd2ca0135f84f9ad2f0d11829b39218bc7b12acc5b3925978f1fa53bf
  qualify_sharded.py      38692e38c7174cdc5e13f31048a80aa0dfe026362a7fb1de8a4e391c3e894e41

The full generated suite is local fixture verification only. It does not qualify or retry the failed native Concord scheduler run.
