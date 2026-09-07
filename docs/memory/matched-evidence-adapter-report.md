# Artifact-bound matched evidence reader

The reader binds explicit receipts to immutable saved binaries and their raw
run artifacts before invoking the existing qualifier and metric analyzer.
It does not establish performance acceptance. Overhead validation and stable
ordinal consumption remain unavailable pending their separate integration.

## Required artifact chain

- Receipt id/index/kind, typed frontend exit, exact run directory and binary,
  explicit UTC launch/completion envelope, and the recorded effective CLI.
- Persisted manifest path/SHA256, raw metadata/sample/qualification hashes,
  server-identity path/hash, and host-conditions path/hash when external.
- The launcher actual parser validates both enabled and disabled flags, panel
  render mode, headless terminal selection, stack mode, timing and canonical
  binary. Duplicate options follow real argparse last-wins behavior.
- The reviewed `build_provenance.verify_build` verifies canonical named binary
  and navpack/navflags/catalog paths against real file hashes, successful build
  exit, typed features/allocator, stable source digests and build/client commits.
  Metadata must carry matching nested `build_provenance` with completion_status
  unchanged. Missing runtime hash/configuration fields are never filled in.
- Top-level legacy source/client labels describe the checkout. The actual saved
  binary client/source identity comes from the separately recorded and verified
  build object. Reference/candidate host sources and binary hashes may differ;
  built client source identity remains an equality-checked configuration key.
- Qualification and analysis are recomputed independently. Raw files, manifest,
  binary/assets, receipt and external identity/host sidecars are rechecked after
  consumption. Malformed/unreadable artifacts return unavailable rather than a
  false binding or unhandled type/parse exception.

Explicit launcher/sampler failure, receipt binding_errors, missing/changed
artifacts and inconsistent time/CLI/configuration are rejected. A binding is
not eligibility: missing runtime match keys, qualification failure, overlapping
observation windows, missing ordinal mapping, endpoint mismatch or unknown
helper overhead still prevent a pair from being eligible. Configuration
comparison retains every non-side key in the metric adapter. Server identity
includes PID plus process start identity; its non-sensitive configuration is
an explicit separate required match key.

## Validation

`python3 -m unittest test_matched_evidence_adapter
 test_reference_metrics.ResourceAdapterTests -q` passed 34 tests. Fixtures use
real binary/asset bytes and persisted hashes, UTC envelopes matching their
metadata timestamps, completed nested build provenance and unchanged sidecars.
The positive fixture proves binding and qualification only, then reaches
`overhead_unavailable`.

Eighteen added production-path negative cases cover invalid/mismatched UTC,
missing/extra profile flags, duplicate timing arguments, missing manifest
fixtures, invalid source digests, wrong manifest hashes, corrupt assets, swapped
server sidecars, nonfinite metadata, bool PID/feature confusion, absent/changed
build completion, launcher/sampler failure and explicit binding errors.
Additional tests cover changed checkout labels remaining distinct from saved
builds, file mutation during independent analysis and host-sidecar mutation.
The six repeat root probes are saved in
`diagnostics/adapter-contract-review-20260907T040246Z/root-round3-probes.json`;
all now return unavailable with binding_ok false. No live run or binary build
was performed for this Python correction.

## Remaining work

The legacy N1 fixtures remain unavailable; no raw artifacts are enriched or
rewritten. New managed receipts must be produced at actual launch/completion.
Actual renderer/cache settings, full host/server/helper provenance and cache
content hashes still need independent capture. New native ordinal rows and
schema-2 continuous process accounting are not yet consumed here. The approved
OFF/ON/ON/OFF overhead sequence and matched repeated performance/lifecycle
matrix have not been accepted. No caller overhead/qualified label unlocks a
pass, and no confidence or latency margin is invented.
