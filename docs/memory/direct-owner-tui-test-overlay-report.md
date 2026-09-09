Direct-owner TUI test-only overlay report

Scope

This report covers only the new test-only overlay under
`diagnostics/direct-owner-native-preparation/tui-test-overlay/`. It does not
modify the frozen source archive, its manifest, the production checkout, or
any client/submodule files. The derived tree is a temporary staging input for
unit tests only; it is not a production replacement and is not a native/live
qualification artifact.

Frozen inputs

- Archive: `diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz`
- Archive bytes: 5,075,876
- Archive SHA-256: `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`
- Manifest source-member count: 1,124
- Original TUI source: `crates/tui/src/bin.rs`, SHA-256
  `a859ca06b33066cbb55641facb7eea449317c9d3e7a6f93ff11c5787d01499d2`
- Cargo.lock SHA-256: `03059d02f8274c8fc9cd7e2f69a48f317e3f44417570ecf4730b31710f8d4b83`
- Client Cargo.lock SHA-256: `015530e3685e8175fe5ec12b41cf37a40ee116557d575cd38b6852926ec47cc7`

Overlay

`tui-test-only.patch` is exactly the requested `git diff c0709aba f9675b66
-- crates/tui/src/bin.rs`: 28 insertions and 4 deletions. Patch bytes are
3,091 and its SHA-256 is
`3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4`.

`stage_tui_test_overlay.py` performs the following fail-closed checks:

1. Verifies the immutable archive bytes and SHA-256 against the frozen
   manifest.
2. Extracts a fresh tree and verifies all 1,124 member paths, modes, sizes,
   and hashes.
3. Verifies the original `crates/tui/src/bin.rs` hash before patching.
4. Applies only the hash-bound overlay patch to that one path.
5. Verifies the derived TUI source hash
   `aee97345c86cff0eb3253ef56b3e64fd123991f57bebbe5fcb750d899839e28c`.
6. Verifies that the guard field, helper implementation, and spawn branch are
   all `cfg(test)`-excluded, and records `non_test_source_modified: false`.
7. Rechecks every manifest lockfile (including the independent shade-probe
   lock) after staging and, when tests run, after tests; each check asserts
   both bytes and SHA-256 identity. Linux/live qualification remains false.
8. Runs tests in an owned process group, streams output directly to the log,
   terminates and reaps only that group on timeout, and retains the derived
   tree and partial log for failed or timed-out runs.

The resulting receipt is
`tui-test-overlay/verification-receipt.json`. The original archive and
manifest remain unchanged.

Local verification

The helper was run from the frozen archive with:

    python3 diagnostics/direct-owner-native-preparation/tui-test-overlay/stage_tui_test_overlay.py --archive diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz --manifest diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-source-manifest.json --output-dir /private/tmp/direct-owner-tui-test-overlay-03 --run-tests --timeout 120

The helper staged a fresh source tree, applied the patch, and ran:

    cargo test --offline --locked -p tui --features memory-profile-no-alloc,memory-owner-capture -- --test-threads=1

Result: exit 0; 92 passed, 0 failed, 0 ignored, 0 filtered out; 4.71 seconds.

The two guard-covered preparation tests both executed and passed:

- `bin::tests::live_prepare_bone_burier_selects_the_rs2b0t_card_without_starting`
- `bin::tests::live_prepare_thiever_posts_guard_target_when_schema_empty`

The complete test log was 21,978 bytes with SHA-256
`70b07a82eaa1677c66e1a060108e84cf1711db7be238ae51abb67599a27675cf`.
The test process created no slot worker from either preparation fixture; the
new assertions verified the absent arm and staged pending script. The fixture
harness may construct synthetic temporary vault fixtures, but no operator/live
vault, account, cache, server, or network was accessed.

Native handoff

This macOS run makes no Linux claim. The helper's later native staging command
is the same command with a new output directory, and must be run only by the
root-owned native workflow after review:

    python3 diagnostics/direct-owner-native-preparation/tui-test-overlay/stage_tui_test_overlay.py --archive diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz --manifest diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-source-manifest.json --output-dir /path/to/new/native-overlay-output --run-tests --timeout 900

That command still uses a newly staged test-only tree. The live production
build must continue to use the original frozen archive, not this overlay.

Conclusion

The separate test-only overlay is reproducible, hash-bound, limited to the
reviewed TUI test guard, preserves all 92 TUI tests and the existing test
count, and passes the bounded offline/locked local suite. It does not repair
or replace the failed Linux qualification and does not qualify any native or
live workload.
