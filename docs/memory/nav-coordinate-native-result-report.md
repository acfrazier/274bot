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
