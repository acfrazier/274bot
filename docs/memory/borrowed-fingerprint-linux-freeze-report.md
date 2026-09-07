# Borrowed fingerprint Linux x86_64 freeze (plan 4B)

Captured: 2026-09-07 UTC
Status: matched artifacts frozen; performance acceptance false.

## Exact inputs

- Baseline host: `e3188e2522c4681a47e0127c8202caaa0c3b81a8`
- Candidate host: `188a52095f3a8a6db9b2567f5278993b80876852` (`script: borrow fingerprint compare; retain only changed fields`)
- Client submodule for both: `3456edc8dabf7b25ada78110ffa56327af9f67a4`
- Actual Rust diff between the pair: `crates/script/src/isolate_fb.rs` and `crates/script/src/slot.rs`; the candidate commit also carries the reviewed report. No client diff.
- Independent Git source snapshots: `sources/baseline/` and `sources/candidate/`, with inventories in `sources/*-snapshot-hashes.json`.
- Complete pre/post input records: `input-hashes-before.json` and `input-hashes-after.json`.

The snapshots were exported from Git before building, so later checkout activity cannot change build inputs. The pinned image was `274bot-memory-ref:1.98.0-bookworm-amd64`, image ID `sha256:51e9ecf025a9ac89e2ae14be24008827771a25d9792370fa2724c4ae5387d637`, platform `linux/amd64`. Toolchain output is in `toolchain.txt` and the successful Cargo version probe is in `cargo-version.txt`. Cargo was locked, feature `memory-profile-no-alloc` was enabled, the normal System allocator was used, and no native CPU tuning was requested.

## Frozen binaries

| Role | Path | SHA-256 | Size | Mode |
|---|---|---:|---:|---:|
| Baseline | `artifact/tui-play-linux-x86_64-baseline-system-allocator` | `019c35779271f150bb83fb7cf6fcb755a4abee8abd1baa2378dcf9e76c932b76` | 95,280,840 | 0555 |
| Candidate | `artifact/tui-play-linux-x86_64-candidate-system-allocator` | `10ddc915c7b0f5ef9f3fbb9443c95bc74249303d9a048fcdc4fb4e4fc69af008` | 95,292,216 | 0555 |

`build-manifest.json` is the machine-readable root. `elf-deps-baseline.txt` and `elf-deps-candidate.txt` preserve `file`, `readelf -lW`, and `ldd` output from the pinned image. The artifact hashes and modes were rechecked after finalization.

## Verification

The successful isolated run was container `borrowed-fingerprint-linux-freeze-20260907-container-6`, with `CARGO_BUILD_JOBS=1` to keep the pinned 4 GiB environment from linker OOM. Every required command exited 0:

- Baseline release TUI build: 0
- Candidate release TUI build: 0
- Baseline and candidate `cargo test --locked -p script --features load`: 0
- Baseline and candidate `cargo test --locked -p host-play --features memory-profile-no-alloc`: 0
- Baseline and candidate `cargo test --locked -p tui --features memory-profile-no-alloc`: 0

The first attempt is preserved in `container-4` logs: the baseline script test reached the linker and exited 101 after `ld` was killed with signal 9. It was an environment-memory failure, not rewritten; the sequential rerun passed both script suites. No existing container was stopped or mutated.

These builds and tests establish reproducible matched artifacts and functional test results only. They do not establish RSS, CPU, latency, renderer cadence, or any savings claim.
