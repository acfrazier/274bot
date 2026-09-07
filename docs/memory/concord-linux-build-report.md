# Concord Linux x86_64 TUI build report

Card: `t_136083b8`
Build batch: `concord-linux-build-20260907a`
Captured: 2026-09-07 UTC

## Result

A release `tui-play` executable was built for Linux x86_64 in the existing pinned image `274bot-memory-ref:1.98.0-bookworm-amd64`. The build used Rust 1.98.0, locked dependencies, feature `memory-profile-no-alloc`, the normal system allocator, and no CPU-native tuning. The binary is preserved in the immutable artifact directory:

`diagnostics/concord-linux-build-20260907a/artifact/tui-play-linux-x86_64-system-allocator`

- SHA-256: `f00e7fb18e28c013fc173e78d956bd4db4b819fd28a3a39b8784de71a7ed2546`
- Size: `95,280,792` bytes
- Mode: `0555` (`-r-xr-xr-x`)
- ELF: 64-bit LSB PIE, x86-64, dynamically linked, not stripped
- Interpreter: `/lib64/ld-linux-x86-64.so.2`
- Build ID: `1027985ac14b14dfabe0b1b2a69eec41764df7c1`
- Observed glibc symbol floor: `GLIBC_2.34`
- Dynamic dependencies: `libssl.so.3`, `libcrypto.so.3`, `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`, and `ld-linux-x86-64.so.2`

ELF and dependency evidence is in `elf-and-deps.txt`; the complete feature/metadata capture is in `feature-verification.log`. The feature graph explicitly includes `tui` `memory-profile-no-alloc` and `host-play/memory-profile-no-alloc`.

## Source and image provenance

- Host checkout at preflight: `8318368cf434e73faca8ec80450789a217b49aa2`
- Host checkout at postflight: `b4b686fd8765cc9d1aa880346f440bd6b781246e`
- Client submodule before/after: `3456edc8dabf7b25ada78110ffa56327af9f67a4`
- Image ID: `sha256:51e9ecf025a9ac89e2ae14be24008827771a25d9792370fa2724c4ae5387d637`
- Image platform: `linux/amd64`
- Toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)`, `cargo 1.98.0 (797e8a9bc 2026-08-05)`
- Cargo invocation: `cargo build --locked --release -p tui --bin tui-play --features memory-profile-no-alloc`
- Build container: `concord-linux-build-20260907a`; it was uniquely named and removed after exit.

The host HEAD changed during this run due to concurrent root activity. This is not reported as a clean checkout. The actual Rust/Cargo/client/build-input hash set was captured before and after across 351 files; `input-hash-comparison.json` records equal client HEAD and equal file hashes, while correctly recording the changed host HEAD. No production files were edited by this card.

Preflight also records the existing dirty Python worker files, including `docs/memory/run_diagnostic.py`, `run_managed_cell.py`, their tests, `tui_input_probe.py`, and `windows_process_sample.py`, plus existing untracked diagnostic material. These were not changed or treated as Rust build inputs.

## Verification

- Release build: PASS, exit 0 (`build.log`, `build-exit.txt`).
- `cargo test --locked -p host-play --features memory-profile-no-alloc`, non-root `memref`: PASS, 176 unit tests plus non-live integration/doc-test groups; live tests remained ignored (`test-host-play-nonroot.log`).
- `cargo test --locked -p tui --features memory-profile-no-alloc`, non-root `memref`: PASS, 92 tests, zero failures (`test-tui-nonroot.log`).
- Feature graph/locked metadata verification: PASS, exit 0 (`feature-verification.log`).
- ELF architecture, interpreter, dependencies, and version requirements: captured with `file`, `ldd`, and `readelf` (`elf-and-deps.txt`).

A root-user TUI test attempt is preserved in `test-tui.log`; it failed one DAC-sensitive test because root bypassed the intended permission boundary. The required non-root rerun passed. An initial wrapper invocation incorrectly passed `--name` through the container entrypoint and is preserved in the pre-existing `build.log`; the corrected uniquely named build is the successful receipt.

No panel build, client GPU pass, live run, remote SSH/server action, VM/reboot/config change, or performance/native VPS qualification was performed. The running `memory-ref-amd64-view` and `fr-vault-chroma` containers were left untouched.

## Artifact and evidence files

- `artifact/tui-play-linux-x86_64-system-allocator`
- `artifact.sha256`
- `artifact-stat.txt`
- `host-file.txt`
- `elf-and-deps.txt`
- `glibc-symbol-versions.txt`
- `build.log`, `build-exit.txt`
- `test-host-play-nonroot.log`, `test-host-play-nonroot-exit.txt`
- `test-tui-nonroot.log`, `test-tui-nonroot-exit.txt`
- `test-tui.log`, `test-tui-exit.txt` (preserved root-user failure)
- `feature-verification.log`, `feature-verification-exit.txt`
- `preflight.txt`, `input-hashes-before.json`, `input-hashes-after.json`, `input-hash-comparison.json`
- `hash-build-inputs.py`, `analyze-build.py`
