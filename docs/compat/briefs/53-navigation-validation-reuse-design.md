# Build-time navigation identity for bundled release assets

Latest operator clarification (2026-09-11 01:28 UTC) controls this design:
"we precompute the hash at build time. In a release build we will be shipping
the navpack with it". Root initially interpreted the request as one runtime
hash around immutable loading; that interpretation is superseded. The user
wants build/package-time validation and a shipped navigation manifest, with
normal release startup loading the bundled pack once and using its build-time
identity without rehashing the whole file. Runtime full hashing belongs to
external/custom packs, development iteration, or explicit integrity checking.
Do not insist on a startup full-byte checksum for bundled release assets.

This new bounded scope supersedes the older startup brief's requirement to keep
three navigation rehash passes. Packaging is planned but publication/signing/
release creation is not authorized now. The current development path must stay
usable and fail closed for external revision/cache/manifest mismatches. There
must be an explicit packaged-asset boundary; release optimization must not be
selected merely by a filename, --release or the presence of an arbitrary
user-authored manifest. A manifest content hash identifies the packaged asset;
it is not a claim that startup freshly verified disk bytes. Signed bundle or
validated release artifact provenance is the intended distribution boundary.

Architecture review only using grok46 defaults. Read AGENTS.md, docs/execution.md,
this brief, and relevant code at an exact committed host plus gitlink. No source
edits or LIVE. Own only docs/compat/06h-navigation-validation-reuse-design.md.
Keep the report actionable and bounded (roughly 150 lines), not a re-audit of
all profile, protocol, gameplay or older campaign reports. Existing integration
review t_dde220ef continues against its own frozen source; this new change must
receive its own same-card source review and final branch review later.

Inspect exact ownership and runtime navigation consumers:
crates/host-play/src/{profile,lib,progress}.rs,
crates/nav/src/{world,pack,manifest}.rs,
crates/panel/src/{picker,session,app}.rs, focused profile tests and existing
release/build metadata helpers. NavWorld::load_pack currently reads bytes again
after hashing; legacy grid fallback rereads by path. SharedClientTemplate holds
Arc<NavWorld>; slots and picker share it. Raw 261 MB flags are paint-only and
loaded lazily in picker, then dropped when collision toggles go off. Do not
retain/eagerly decode that whole file merely to avoid a hash.

Recommend the smallest correct implementation that:
- Validates and hashes each shipped navpack at build/packaging time, binding
  revision, cache identity, pack format and nav content hash into a bundled
  release manifest. Rebuilt packs get a new identity even for the same revision.
- Normal packaged-release startup resolves its bundled asset, checks relevant
  manifest/profile/header compatibility, decodes once, and shares the immutable
  NavWorld with every bot. No repeated full-file startup SHA-256 for that asset.
  Be honest that post-packaging disk tamper detection is not supplied by merely
  reading the recorded hash; do not expand this into an adversarial security
  system or make routine users approve a trust dialog.
- External path overrides and development packs retain runtime identity
  validation, ideally checking the exact bytes they decode once and reusing
  that loaded world. Keep unsupported versions/corrupt decode failures,
  legacy274 and missing-navigation behavior explicit. No mtime-only hash cache.
- Keeps optional raw flags lazy. Decide whether release packages need that debug
  sidecar at all using the existing contributor/runtime boundary. When supplied,
  bind it to the same world; external flags must not use dimensions as sole
  identity. Preserve memory ownership and existing lazy/drop behavior.
- Keeps cache/client assets, generation safety and protocol/session ownership
  unchanged except minimal supporting seams. No new resource manager, mmap
  dependency, persistent hash database or per-bot world copies.
- Makes progress honest and understandable: Preparing world data / Loading
  navigation for bundle decode, optional Verifying custom navigation for
  external bytes, actual work counts and ACCENT/#FFB000. No identical per-file
  resets presented as a restart, and no timer-based percentage.

Name concrete implementation seams, a minimal package-manifest representation,
what can be built now without publishing a release, and focused verification.
Required proofs distinguish packaging validation from runtime loading: bad
pack/revision/cache rejected by packaging; bundled startup has zero full-file
hash passes and one decode; N=2 shares one world; external wrong hash rejected;
after-load file changes do not replace the existing in-memory world; a new
pack identity is selected explicitly for a new load. Root will run selected
native correctness checks after source review. Read/decode counts are useful;
latency/RSS claims still need controlled measurements. Historical native
changed-file failures remain evidence of the old disk-revalidation contract.
No product timeout/catalog proof changes. Commit only your report and complete
the design card. No subdelegation.
