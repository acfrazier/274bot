# Concord native Linux environment

Prepared and verified 2026-09-07. This is native x86_64 VPS execution, not the
Mac's emulated Linux container. Concord reports Ubuntu 24.04.4, two logical CPUs,
1,967 MiB RAM and no swap. Its RAM is below the approved 4 GiB TUI reference
profile. The game server is co-located and must be measured separately.

## Isolation and provenance

The operator applied dedicated-key SSH setup and disabled the old nightly timer.
Fresh key login was verified. The prior nightly repository, engine and data were
preserved. The campaign uses a separate server directory and source checkout.

The server fixture is based on engine commit
`4c95f87efe00b068cadbd229d94736626907bd1a`, Node 24.19.0, with the already qualified
operator seed handler, wordenc and runtime-content map additions. The uploaded
archive SHA-256 is `188955b66ee0f8d3ebb3cbcb5ded632a22efd9fe66d7bc5c2712526f9361cf2c`;
688 fixture files were hash-checked. Locked dependencies were installed. Fresh
local RSA keys and SQLite were initialized; production players were not copied.

The operator started a static systemd unit as the ordinary campaign user, with
only the low-port bind capability and a restricted writable server directory.
It is not enabled at boot. Readiness logs report 7,322 static NPCs and World
ready; ports 80, 43594 and 8898 listen only on loopback. Server PID 152004 has
`linux_proc_start_ticks:241214967`. The operator corrected the initial root-only
log ownership without restarting the server. This PID is an evidence snapshot;
future runs must verify its start identity again.

## Client and measurement setup

The native Linux binary is the reviewed system-allocator artifact documented in
[concord-linux-build-report.md](concord-linux-build-report.md), SHA-256
`f00e7fb18e28c013fc173e78d956bd4db4b819fd28a3a39b8784de71a7ed2546`.
Its glibc 2.34 floor and dynamic dependencies resolve on this host. The executable
is hash-checked and staged read-only. Real Git checkouts record host `b4b686f`,
client `3456edc`, and script repository `100adccc`; the build report explains the
separate build-time source identity and documentation-only HEAD transition.

The corrected native Linux N1B and first N16A diagnostics completed and
qualified their workloads. See
[native-platform-resource-screen-report.md](native-platform-resource-screen-report.md)
for the bounded resource screen. Both on-host `bind_side` results report
`status: bound`, `qualified: true`, no missing match keys, and native plus
managed resource inputs available. The receipts remain artifact-bound
diagnostic evidence, not final performance or lifecycle acceptance.

The N1B and N16A runs use the same frozen system-allocator binary and source
labels. N1B recorded median frontend RSS 175,255,552 bytes, 0.06651113 CPU
cores, and 49.6703 client ticks/slot/s. N16A recorded median frontend RSS
640,348,160 bytes, 0.72958343 CPU cores, and 48.3294 client ticks/slot/s
across 16 qualified slots. The diagnostic N+15 arithmetic is 443.546875 MiB
of median RSS increase, or 29.5697917 MiB per additional slot; it is not a
matched saving or capacity model. N16A is above its diagnostic median-RSS and
CPU budgets. Its predeclared 128 MiB `MemAvailable` guard polled at 0.5 s and
did not trigger.

The server remains separately accounted as PID 152004 with its recorded start
identity and native host conditions. The server, terminal launcher,
controller, SSH helper, and collector remain explicit roles; this is not a
claim of complete descendant-tree sampling. No accepted memory saving,
target-budget pass, latency pass, or final platform qualification follows.
