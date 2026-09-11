# Bundled navigation loading (build-time identity)

Task: `t_6533221b`
Brief: `docs/compat/briefs/55-bundled-navigation-loading.md`
Design: `docs/compat/06h-navigation-validation-reuse-design.md` (`09f26d0a`)
Branch: `codex/rs2b0t-multirevision`
Client gitlink inspected: `56d80272bcbda3eb1e22db096c1c5e21d3497de4`
Kind: authorized source groundwork. Not packaging, signing, LIVE, or a measured load-time win.

## Result

Shipped navpacks keep a compile-time identity. Bundled startup reads and
decodes the pack once and does not SHA-256 it. External/custom packs hash
the exact bytes they decode, once. `ServerProfile` holds the resulting
`Arc<NavWorld>`; template load, pre-Play validation and slots reuse that
world. Flags stay optional, lazy and paint-only.

The checked-in table `crates/host-play/src/bundled-nav-identities.json` is
empty. `cargo build --release`, a user sidecar, or a matching filename
cannot select bundle trust.

## Ownership

- `NavWorld::from_bytes` decodes the provided buffer, including the 274N
  grid fallback, without a second `fs::read`. `load_pack` is one read plus
  `from_bytes`.
- Origin selection (`crates/host-play/src/nav_identity.rs`) uses the
  compiled table, an install-relative path, and no `--nav-pack`/`NAV_PACK`
  override. macOS `Contents/MacOS` resolves to `Contents/Resources`;
  ordinary binaries use the executable directory. Row shape, containment
  and `274V8` are validated. There is no user trust UI.
- Bind captures cache, then loads navigation. Bundled: one read, one
  decode, zero pack hashes. External: hash the same buffer, check
  revision/cache/`nav_sha256`, decode. Missing pack stays Unavailable.
  Missing 289 sidecar stays a hard mismatch. Missing 274 sidecar stays
  Legacy274 when the bytes decode. Corrupt/BadVersion still fail.
- `validate_resources` / `validate_for_play` recheck cache archives only.
  They do not rehash navigation or flags.
- Flags: omitted from ordinary startup. Paint-on hashes the sidecar
  against the selected identity's `flags_sha256`, then geometry. Unknown
  or wrong identity cannot apply foreign flags; toggles-off drops the
  decode. Walk-word paint remains the fallback.
- Progress: bundled decode is `Loading navigation` (steps 0/1, 1/1).
  External hashing is one `Verifying custom navigation` pass, then the
  same decode stage. The repeated per-file nav bar is gone.

## Retired post-load-disk contract

Historical `validate_for_play` failures after rewriting the pack or
flags file on disk were the old revalidation contract. They must not be
reimposed.

After a successful bind/load, a disk rewrite does not replace the
in-memory `Arc`. A new explicit bind hashes/selects a new identity or
rejects it. Flags changes are paint-on only.

## Verification

Peer WIP on `script`/`host-play` interact and loadout files was not
restored or stashed. Checks ran on an export of committed HEAD plus this
card's owned files:

`.superpowers/task-exports/t_6533221b-navload-1789091527`

Target dir: `.superpowers/task-exports/target-t6533221b`

- `cargo test -p nav --lib -- manifest::tests world::tests::from_bytes world::tests::load_pack_keeps_stale_v5 world::tests::load_pack_path_round_trips_a_fixture_grid`: 6 passed
- `cargo test -p host-play --lib -- nav_identity::tests`: 6 passed
- `cargo test -p host-play --test session_profile`: 17 passed
- `cargo test -p panel --lib -- picker::tests::flags_sidecar picker::tests::sidecar_file picker::tests::sidecar_flags picker::tests::picker_pack app::tests::loading_text`: 6 passed
- `cargo clippy -p nav --lib --tests -- -D warnings`: passed
- `cargo clippy -p host-play --lib -- -D warnings`: passed
- selected-file `rustfmt --check`: passed

No LIVE run. No latency/RSS claim. Native external + synthetic bundled
fixture observations remain root-owned after review.

## Out of scope

Packaging the zip, filling the identity table with release hashes,
signing, catalog/LIVE proofs, and concurrent interact/loadout WIP.
