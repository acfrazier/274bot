# Offline nav uniform-tile census implementation

Scope

This directory is an isolated Cargo workspace. It contains the native
`nav-uniform-census` diagnostic, the actual-decoder `nav-fixture` generator, a
Python owned-process supervisor, and compact fixture tests. It does not edit
host/client runtime code, root Cargo files, production pack validation, routes,
or public API, and it never reads the real navpack.

Source and build identity

- Current host checkout: `e16931ba075440aad65360166cb2dbcd1ad5956d`.
- Frozen host provenance required by the diagnostic: `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`.
- Frozen client provenance: `3456edc8dabf7b25ada78110ffa56327af9f67a4`.
- `build.rs` compares the byte contents of every frozen `crates/nav` and
  `crates/api` Rust/Cargo/build input and every compiled client-package
  (`vendor/fr-client-rust/crates/client`) Rust/Cargo/build input
  plus both workspace manifests/lockfiles before compiling. It rejects
  relevant dirty source and has rerun triggers for every verified tree; it does
  not require the enclosing checkout HEAD to equal the frozen host commit.
- The executable reports both actual checkout refs and the deterministic frozen
  source-manifest digest (`c97223eef5030d60dca26250976e78f41aa94416e22f63708fd9cb96788fa207`
  for this local build).
- `cargo check --offline --locked` and `cargo build --offline --locked` pass.
  The isolated `Cargo.lock` is included.
- An initial `cargo fmt --all` temporarily formatted 25 host/runtime files
  outside this diagnostic (including dependency files). Root preserved and
  independently verified that spill under
  `diagnostics/nav-census-format-recovery`, then restored the exact HEAD bytes;
  final tracked runtime source status was clean. Future formatting used
  `rustfmt` only on files in this diagnostic.

Native diagnostic

The input is admitted once as a regular non-symlink, capped at 128 MiB, hashed
before `nav::pack::decode`, and the decoded instance is retained. The report
contains actual vector lengths/capacities/element sizes, dimensions/origin,
flags presence, graph/teleport/bank counts, four level counts, the bounded 512
pair histogram, and checked A/T/D accounting. Tile comparisons use the full
face-byte plus blocked-bit pair; boundary tiles inspect only valid map cells.
The hypothetical estimate includes deterministic boundary padding and labels
hypothetical storage, not an actual allocation, not RSS, and not acceptance.
Omitted header/allocator/conversion/input/graph-bank/page costs are explicit.

Supervisor and fixtures

`supervisor.py` uses direct subprocess execution, lexical no-symlink admission,
input/executable/source hashes and an explicit expected source-manifest
digest (the `--source` file hash identifies the staging artifact separately),
unique output directories, fixed CPU 30 s, wall
60 s, RSS 384 MiB, AS 512 MiB, and one combined 1 MiB stdout+stderr+receipt
budget. REAL runs fail
closed off Linux and require expected identities; Linux uses `/proc` samples.
Portable Mac fixtures explicitly mark Linux supervision as skipped. Process
groups are owned and killed/reaped on failure or interruption, and pre-launch
and child failures retain receipts. Sampled RSS is explicitly not presented as
a cumulative peak. The runner validates the child input identity before
accepting a result.

`test_nav_uniform.py` covers uniform/all-pair, boundary, per-plane, malformed
and oversized dimensions, hash mismatch, portable owned success, nonzero child,
output overflow, and timeout receipt retention. The actual decoder encoder is
used for valid fixtures; no foreign decoder is duplicated.

Verification evidence

- Six-test suite: `python3 -m unittest discover -s docs/memory/nav-uniform-census -p 'test_nav_uniform.py' -v` — 6 passed.
- Follow-up smoke after supervisor identity/cleanup changes: 2 targeted tests —
  2 passed.
- No real navpack, remote host, native Linux executor, server, live capture,
  or acceptance run was performed.

Remaining gates

A root-controlled Linux build must reverify staged source bytes, executable and
manifest identities, fresh memory/disk admission, and all owned-process guard
fixtures. Root must separately qualify the reviewed tool and authorize exactly
one real-input census. This card does not release either action.
