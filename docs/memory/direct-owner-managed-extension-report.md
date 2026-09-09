# Direct-owner managed extension implementation report

Status: implementation and generated functional validation complete for review. This is not native target, private-admission, or live acceptance evidence.

## Implemented scope

- Added the exact opt-in `--direct-owner-capture` contract and preserved the default/no-cache contract when the flag is absent.
- Bound the root-selected cell/spec/output paths, direct launcher arguments, `N=1`, direct hot-flag environment, source/build lineage, admission receipt paths, raw owner output, launch/exit handoffs, and guard artifacts.
- Added direct Linux/x86_64 preflight for source and runtime provenance stability, binary identity/size, clean source trees, feature/allocator evidence, server identity and non-zombie state, MemAvailable, swap use and raw VM counters, cgroup boot/path identity and OOM counters, free output space, and immutable admission receipt file bindings.
- Added direct launch signal discipline: parent SIGTERM/SIGINT blocking across `Popen`, child restoration before exec, durable spawn handoff before parent unblocks, registered-handle cleanup, bounded unique-owned partial-spawn discovery, and explicit orphan-risk receipts when identity cannot be proved.
- Added direct guard enforcement for first/subsequent sampling deadlines, frontend identity/parent continuity, RSS, MemAvailable, wall time, output bytes, output type/symlink/disappearance/replacement races, normal exit proof, and outer wall time.
- Kept process accounting through launcher exit; direct guard failure terminates only the managed launcher/frontend/collector ownership chain.
- Rechecked build/source and root receipt bindings after the run, validated the raw owner JSONL with the existing validator, and hashed raw guard/handoff/owner artifacts into the managed receipt.
- Preserved the default process/accounting/receipt path when direct mode is absent. Direct-only environment variables are scrubbed from inherited environments and re-emitted only after direct validation.

## Section 11 generated coverage

The generated tests exercise:

- exact opt-in/default contracts, forbidden combinations, path binding, `N=1`, environment scrubbing, and clean-tree handling;
- build/archive/materialization/receipt/binary lineage validation and mutation rejection;
- missing/malformed raw swap counters, exact preflight boundaries, server zombie rejection, cgroup boot identity, counter drift, and receipt mutation;
- exact handoff schemas, spawn immutability, deadlines, PID/start/parent identity, exit proof, replay/replacement, disappearance, symlink, hardlink, and type-race handling;
- equality versus over-threshold guard behavior, large overshoots, skipped grids, missing/invalid identity samples, RSS/MemAvailable/output/wall limits;
- partial-spawn uniqueness, child restoration failure, pre-spawn discovery failure, SIGTERM/SIGINT child response, and real pending parent stop delivery only after durable handoff on generated Linux/x86_64;
- raw artifact presence/hash binding and true child exit-code preservation;
- generated lifecycle timestamps proving the direct collector remains alive through observe-end, Stop, C, and launcher exit, while the default collector stops after its two-interval pad before Stop/C;
- macOS import/default compatibility paths and Linux-only direct-mode gating.

Generated tests do not substitute for the still-required root/native work: all five root-private admission receipts, exact target checkout/build preparation, native Linux/x86_64 qualification, Windows import/contract execution, and the live matrix. No private root artifact was fabricated and no live claim is made.

## Verification

### Green generated Linux functional subset

Environment: local pre-existing `python:3.11-slim` container, Linux/x86_64 under Docker Desktop emulation, Python 3.11.16, `--init` enabled.

Command loaded the five focused modules, flattened the suite, and excluded only:

`docs.memory.test_run_diagnostic.RunDiagnosticCli.test_resolve_rs2b0t_commit_real_git_when_path_ok`

The container has no `git` executable. That exact real-git test passed in the macOS non-collector run.

Result after the collector-lifecycle review correction: 141 tests, 0 failures, 0 errors, 2 macOS-only skips.

Log: `docker-linux-generated-unittest-review-fix.log`

SHA256: `224eff9280223e4130fdf7cb5c06c08e1f09eaf6fea2238893789e370c6be13e`

This is a subset and generated functional evidence, not a full-suite/native pass.

### Green macOS partitions

Host: macOS 15.7.9 arm64, Python 3.9.6.

- Four non-collector focused modules: 90 tests, 0 failures, 3 Linux-only skips. `macos-focused-noncollector-unittest.log`, SHA256 `100c8d1445fe7d62d47a5cc71a1d3a5189e2b877e1387bd424c0774ca4a17382`.
- Direct managed unit partition: 9 tests, 0 failures. `macos-managed-direct-unit.log`, SHA256 `827d3c7802fb99f1176cbc2a55e429e49f6316f15d272961bb52cdce04fd0979`.
- Review-correction lifecycle pair: 2 tests, 0 failures, 1 expected Linux-only skip. `macos-collector-lifecycle-review-fix.log`, SHA256 `71fc664e9feeb87b1e3dba1a4f5a3f55d2d4ca1211f5059c90a8ce5976555ceb`.
- Existing raw-owner validator from its required working directory: 3 tests, 0 failures. `macos-owner-validator-unittest-rerun.log`, SHA256 `1596143dec45d0aaeec522bd1069dbe8f3e43700c979bf223933eece644e2359`.
- Python compilation of all ten scoped implementation/test modules: pass.
- `git diff --check` for all ten scoped files: pass.

### Preserved pre-review macOS full-suite failure

The complete focused command before the review correction produced 141 tests, 9 failures, 4 Linux-only skips. It is retained as failure evidence; the added lifecycle test is covered by the green Linux suite and targeted macOS run above.

All nine failures are existing managed-cell success assertions whose reports show `collector_exit_code=1` and `sampler_premature_exit=1`. The preserved isolated collector output records the concrete cause as a strict process-accounting cadence deadline miss:

`FAIL: sample cadence missed required deadline at index 5 (lateness_s=0.14382991699999992)`

Final log: `macos-focused-unittest-final.log`, SHA256 `6f12bb09c308981b1e656f01b8b3fb916746ea00a6b36a0eaf452bd886765f57`.

Cadence sample: `macos-cadence-failure-sample.log`, SHA256 `47559635fd0ef6f9a633a2dafbe4b968cc6e9b2a7e16fb340acb326e8e57c21c`.

The initial root-preserved macOS log remains unchanged: 138 tests, 12 failures, 2 skips, SHA256 `18efaffb26f73f21f5c7a17afec3bd58a56d29bc49fc479e197c4e9fc5fa9aad`.

The generated tests and accounting policy were not relaxed to make this host pass. The same default managed-cell paths pass in the generated Linux run.

### Preserved initial Linux container failure

The initial full container command remains preserved as 138 tests, 2 failures, 1 error, 2 skips, SHA256 `6f01ef6584579d2df2a06e1f01bde6a823b1debf944387e8bcccd32c7e76298c`.

Its failures were container-environment artifacts: Python as PID 1 does not reap the orphan fixture, and the image has no `git`. The later green command added `--init` and excluded only the real-git test. It must be assessed as a 140-test subset, not a full-suite pass.

## Native/live handoff

Root must still:

1. prepare exact clean host/client checkouts and the final native binary/build manifest with the required direct-owner feature and counting allocator evidence;
2. supply and bind real root-private conflict, account, population, and server-health admission receipts;
3. run the complete focused suite on native Linux/x86_64 with `git` available;
4. run the unchanged Windows import/contract check;
5. run the required baseline and direct `N=1` live captures/matrix and retain raw evidence.

Container evidence does not release live work.
