# Builder storage expansion — 2026-09-09

The operator authorized expansion of the builder VHD after the first native
coverage matrix exhausted its 64 GiB disk. The builder now uses a separate
128 GiB dynamic VHDX and a grown ext4 root filesystem. Guest free space immediately
after growth was 86,298,832,896 bytes (about 80.4 GiB).

Root shut down only the idle `274bot-builder` VM after confirming no cargo,
rustc, or TUI process. The Windows host and Concord measurement host remained
running. `Convert-VHD` copied the active differencing disk into
`ubuntu-builder-os-expanded-128.vhdx`; `Resize-VHD` grew that independent copy
and the VM was attached to it. `growpart` preserved the root partition's start
sector and UUID, then `resize2fs` grew ext4. The guest returned through its existing
SSH connection configuration.

The original three-member disk chain remains present with unchanged lengths and
write timestamps across conversion. Both checkpoint IDs remain registered:

- Automatic checkpoint: `53428eb2-3632-4182-9962-c9fbcb33da42`.
- `pre-campaign-toolchain`: `e369d938-017e-4206-ac99-166522d2f5d1`.

The old chain is retained for rollback; it was not merged or deleted. The new disk
has no parent. Both preserved release binaries still match their original full
hashes: `8e400e2d622af2c00280bfb91c741753e0da3df1736bb7d6fe7d0f8251394194`
and `392dbecc7a86f2fcd7e3f6b515aedceb3f3de3b1d4e140b95463bf20438c0c95`.
Host/guest receipts and the exact expansion script are bound by
`diagnostics/builder-storage-expansion-20260909/manifest.json`.

## Failed qualification retained

`root-coverage-native-01` remains failed. Eight commands completed with exit zero
before host-play linking failed; the following TUI compile explicitly reported
ENOSPC. The wrapper then could not write its summary, leaving `steps.json` and
`result.json` empty. No complete matrix or native admission is inferred.

The 5,326,072-byte archive has SHA-256
`46d7076954362c3eed75fab9757e8c2c97251e068de5d082bccbed6226ecfcf9`.
Root independently checked all 1,124 derived test-source members against the
frozen manifest plus reviewed six-file test overlay and preserved ten command
logs. The audit is
`diagnostics/direct-owner-native-preparation/root-coverage-native-01-failure-audit.json`.
The original source and failed directory remain on the builder.

Before expansion, root reclaimed disposable incremental compiler state and 47
uniquely-linked debug/test executables in the owned Cargo cache. Inventories and
release-hash checks are in the two `root-coverage-*-cleanup-01.json` receipts.
Hardlinked frontend binaries, release copies, source, and raw evidence were
retained. No product or test assertion was changed.

The second attempt uses fresh `root-coverage-native-02` output and the original
nineteen-command matrix/build settings. Root summary writes are atomic and the
wrapper stops on the first failed command. The capacity correction does not turn
attempt 01 into a pass. Attempt 02 still requires completion, source/lock checks,
raw-artifact review, and the separate live-readiness decisions.
