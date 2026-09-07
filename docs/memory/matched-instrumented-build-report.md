# Matched instrumented release builds

Four system-allocator release binaries were frozen under
`diagnostics/matched-instrumented-build-20260907T054019Z/`; the authoritative
`build-manifest.json` in that directory binds paths, bytes, fixture hashes,
features, client source, and pre/post host source digests. Both builds exited 0
and all four entries passed `build_provenance.verify_build`.

| Side | Build commit | Host source SHA256 |
| --- | --- | --- |
| Duplicate-nav control | `8741399d853bbff76c49bbfb347391fe32dd9578` | `c82379eba218865e169f7978713f87b19ba2dddc7d952e564a13673ea52b93ec` |
| Shared-nav candidate | `c6ce0da67003e55b5cb3ed1b796d2ab3cd79a61e` | `04cc03c46a5249507d269a8eeb0d63dbf5430c40d18bdf433908ea84012b59cf` |

The control retains its original duplicate navigation construction. Root applied
the same reviewed clock/fine-histogram/capture/ordinal/runtime-settings/script
instrumentation to both sides. The only Rust differences are the original
navigation-sharing change in memory.rs, panel app/session, scenario runner, and
TUI startup. The single application conflict was resolved by retaining the
control's absence of three tests for candidate-only ownership helpers.

Control validation: host-play `memory-profile-no-alloc` 172 passed; script
`memory-profile` 300 passed, 2 ignored across 19 suites. Candidate host-play has
175 passing tests, including the three sharing tests. No client code changed.

Each checkout used its own Cargo target directory. Saved pre/post snapshots and
depfile hashes show stable source and no cross-checkout build artifact reuse.
Release fingerprints on both sides include memory-profile and no-alloc; the
allocator path is System. Binary hashes and sizes are in the manifest, and old
frozen builds remain untouched.

Grok-4.5 review `t_e596bc02` approved the source comparison and frozen artifacts.
This is build provenance only. Live qualification, matched resource/latency
measurements, overhead experiments, lifecycle and final Grok-4.6 remain pending.
