# Optional launcher frozen-build verification

The launcher previously labeled a saved binary with source hashes and commits
from the current checkout. It now accepts `--build-manifest PATH --build-role
reference|candidate` with an explicit `--binary` and records verified build
provenance separately from those checkout labels. Existing launches without a
manifest explicitly report build provenance unavailable.

`build_provenance.py` requires an explicit manifest side, successful build exit,
stable pre/post source digests, declared stability, valid commit/digest formats,
client provenance and typed feature/allocator fields. It verifies the canonical
named binary path and SHA256, runtime navpack/navflags paths and hashes, and the
runtime HOME catalog path/hash. A same-hash copy at another path is rejected;
a symlink resolving to the declared path is accepted. All five files, including
the manifest itself, are rechecked before the verifier returns and after the
frontend exits. A completion mismatch preserves the frontend exit in metadata,
marks provenance invalid, and makes the launcher exit 1.

The manifest remains evidence of build provenance; this does not reproduce a
build or cryptographically prove its compiler flags. The current checkout
`host_sources_sha256`, `client_sources_sha256`, `host_commit` and `client_commit`
remain historical field names with `checkout_source_labels_only=true`. The
binary's source/commit values are in `build_provenance`. Its feature flags and
allocator/counting labels are also exposed as metadata fields only after
manifest validation. No runtime client/settings value is silently substituted
from the current checkout. Runtime metadata adds navflags/catalog hashes and
the launcher CLI. This step does not create a managed receipt.

Validation: `python3 -m unittest test_build_provenance test_run_diagnostic -q`
passed 20 tests. The tests use real temporary files and cover every file's
corruption, completion mutation, copied versus symlinked binary, invalid source
stability/features/commit formats, runtime catalog mismatch, CLI prerequisites,
and the production main path rejecting a missing manifest before mkdir/Popen.
Read-only verification of the two TUI entries in
`shared-nav-build-manifest.json` succeeded (control `53ddeda0…`, candidate
`769a9367…`). No frontend, server, build, or performance experiment was run.

Limits: renderer/cache settings, script source dependencies beyond the catalog,
host/helper conditions and overhead accounting are not established by this
step. No existing receipt/raw artifact is rewritten. The old frozen binaries
retain their original instrumentation; new matched builds still need the same
reviewed instrumentation on both sides. This is file/provenance validation,
not workload qualification, overhead proof or performance acceptance.
