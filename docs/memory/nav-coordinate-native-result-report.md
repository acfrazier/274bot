# Coordinate navigation native generated qualification

The builder completed all eight stages for reviewed `a44a930`: guards, source
preparation, fresh clean/counting builds, generated clean/counting execution,
integration and scheduler qualification. The scheduler exercised four generated
children with native address-space enforcement. This is generated functional
evidence, not private-pack performance or CF1/CF2/CA acceptance.

Root independently verified all 771 members of the 5,928,420-byte relocation
archive (SHA256 `18e574254751f4307965bb78e91282ec545af701ae5d1d8cc60ecf07d17afe6c`).
The audit checked four admitted executable hashes, admitted source hashes,
effective source manifests against actual host/client Git blobs, exact reviewed
tool bytes, and all twelve historical/native raw comparisons. Each comparison
has 840 calls; behavior aggregates, cell/layout values, lookup and narrow
checksums match. Counting runs retain zero narrow allocations/requested bytes.
Dense stays at29b7aea; refined stays at8385 with only collision blob07e0b215.
The full raw archive is retained locally; compiler intermediates were excluded.
Audit code and result: diagnostics/nav-stage-a-native-preparation/
`audit_coordinate_native_01.py` and `root-coordinate-native-audit-01.json`.

## Concord dependency correction

The same admitted binaries were staged in
`/home/acfrazier/274bot-campaign/nav-coordinate-a44a930-concord-01`.
The first guard command failed: 21 tests, three errors, all from missing rustup
in source-binding preparation tests. The failed archive and raw hashes remain
preserved (SHA256 `c20effa705e0deb1ce71e1bb0099f68038879465ca1d6c2463d979d9e835b42d`).
No private-pack child ran.

The operator installed apt rustup1.26.0. Root streamed the existing builder Rust
1.98.0 toolchain to a dedicated Concord campaign folder and verified all165 files.
Rustup uses a directory override for this navigation task. Initial safe extraction
removed group-write from nine metadata files; root restored exactly their
manifest modes after verifying bytes. No native probe was rebuilt and no test or
limit was weakened. The toolchain archive/manifest hashes and installation receipt
are recorded under diagnostics/nav-stage-a-native-preparation/concord-toolchain*.

Fresh guards in coordinate-concord-native-guards-02 passed after the dependency
correction. The remaining generated clean/counting/integration/scheduler stages
are currently running with new continuation logs. The first failure is unchanged.
Root must audit their final results before any later private-input phase.

## Fresh native full-byte differential

The separate builder workflow for the same reviewed `a44a930` tooling passed
all six steps: guards, prepare, build, generated comparison, audit, and 13
admission/extension tests. It exercised 12,954 generated inputs and compared
328,120,731 complete output bytes per arm, plus 80,987 bytes per arm for the
generated protocol fixture. Both pairs are identical. No private input was read.

The reviewed archiver produced an 18,765,670-byte archive, SHA256
`baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524`.
Root downloaded it and independently checked all 13,436 members (681,424,323
uncompressed bytes), every corpus hash, output hashes, reviewed launch tools,
admission source bindings, all 209 dense and 210 refined effective source files
against actual Git objects, the sole collision overlay, and frozen host spans.
The first root audit helper retained only files below 2 MiB and therefore omitted
the larger corpus manifest; correcting that readback-only retention threshold
to 16 MiB completed verification. No production tool, test, or evidence changed.

Evidence is in diagnostics/native-nav-differential-preparation/
`coordinate-a44a930-native-01/`: progress.json, audit.log,
admission-extension-tests.log, audit_archive.py, root-archive-audit.json, and the
retained raw archive and archive manifest. Native binaries remain on the builder;
the reviewed evidence archive excludes compiler targets. This is generated
correctness qualification, not exact-59 private validation or performance.

## Concord scheduler failure and bounded diagnosis

Concord continuation passed generated clean (203.182 s), generated counting
(204.518 s), and integration (18.105 s). Scheduler qualification then failed:
the guard subprocess exited -9 after302.849 s with300CPU/360wall limits,
46,628,864-byte sampled group peak, and native hard-AS enforcement active.
It passed the coordinate full-matrix test and was running the legacy full-matrix
test when killed, consistent with its hard CPU limit. No generated native smoke
or private child followed this failure. The overall Concord qualification fails.

Root archived and independently verified all5770 retained files, including the
six generated fixtures and passing earlier stages. Archive SHA256
`7c500748ace8bae33fd449452df5f0b9e8377d3ec4c3a70abe8925d2728a68de`,
429,242 compressed bytes and3,956,521 uncompressed bytes. Artifacts and audit
receipt are under diagnostics/nav-stage-a-native-preparation/ with prefix
coordinate-concord-scheduler-failure-01 and root-coordinate-concord-scheduler-
failure-audit-01.json. All original failed and passing outputs remain unchanged.

A separate builder-only cProfile diagnosis of the legacy full-matrix fixture
passed in67.747 s under90CPU/100wall diagnostic limits. It made198,078,546
function calls; bind_entries consumed63.282 s cumulative, including2,849,698
repeated admission checks. These profile timings identify test cost and are not
production or native qualification evidence. No private input was involved.

Corrective card t_481b2857 uses profile luna, limited to generated matrix
fixtures and explicit final real integrity checks, preserving all826 slots per
matrix and existing negative mutation coverage. Production tools, limits and
binary/source admissions remain outside its scope. Same-card independent review
and root fresh native qualification remain required before any real release.
