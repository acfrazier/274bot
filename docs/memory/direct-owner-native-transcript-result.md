# Native deterministic transcript regression result

Root completed native Linux x86_64 affected qualification for reviewed a3abbdd
with unchanged helper c14b1b56 and patch1e5ac15b. Root wrapper ed77289 reused the
existing owned Cargo target; no production binary was rebuilt or run. Initial
builder free space33 GiB and later31 GiB; no storage exhaustion.

Two original generated controls failed as expected with loc IDs0 and13. Four
corrected full-golden/peer comparisons passed under empty/prepopulated cache and
snapshot feature off/on. Both COMPLETE host-play profile suites then passed:
feature-off aggregate harness summaries197 passed/0 failed/7 ignored; owner
feature-on212/0/7. Full snapshot-dedup integration passed16/0/0. The existing
ignored live tests remain unrun; this is generated functional evidence.

All9 commands ended without timeout and with their owned groups absent. Root
independently verified all9 raw log hashes and recomputed the harness summaries.
The raw archive preserves all1124 source members. Every source byte and mode
matches the frozen manifest plus exactly7 reviewed test-derived members; no extra
source member. Full source contract before/after is identical and all3 locks
remain bound. Only non-source evidence was additionally extracted locally, to
avoid another redundant source/build tree on the Mac.

Evidence under diagnostics/direct-owner-native-preparation:
- root-transcript-native-01-audit.json: independent hashes/source/count audit.
- root-transcript-native-01/{result.json,steps.json,focused/,logs/}: exact commands,
  original failures, corrected results, source contracts and cleanup.
- root-transcript-native-01.tar.gz:5337021B, full source and raw results,
  SHA9c9b5c785b6cb065ad7f0d9562b159076c6e1beddc39575c99d3770a4f4437a8.

This closes the native transcript fixture failure in the affected suites. It
does not relabel the earlier failing coverage02/03 runs. Previously passed
unaffected API/host/script/TUI/client-unit suites remain separate evidence.
Complete client --no-fail-fast inventories and the original-client GPU failure
are in direct-owner-native-gpu-baseline-report.md; the GPU failure remains open.
The reviewed helper's native_linux_qualified flag stays false because its own
full19 mode was not executed. Root records the affected native proof explicitly,
not a blanket passing19-command matrix or live admission.

Independent review must assess this combined functional evidence and the proposed
original GPU defect disposition for the TUI diagnostic. Controller review/native
lifecycle proof, source/build/runtime/private-fixture admission and the single
live capture remain prerequisites. No performance saving is claimed.
