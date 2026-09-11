# Build-time navigation identity for bundled release assets

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: bounded architecture for brief 53. Not implementation,
LIVE, packaging, signing, or a change to t_dde220ef's frozen source.

Read once: `AGENTS.md`, `docs/execution.md`, brief 53, and the named host
files at HEAD plus client gitlink. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report. No product edits, tests on moving WIP, LIVE, STATE, subagents,
stash, reset, checkout, merge, remotes, or release actions.

Operator clarification (brief 53, 2026-09-11 01:28 UTC) controls this
design: precompute the navpack hash at build/packaging time and ship the
pack. Normal bundled startup uses that identity and decodes once, with
zero full-file SHA-256. Runtime hashing is for external/custom/development
packs. Root's earlier runtime-once reading is superseded, as is the old
task-body phrase "exact-byte validation/share" as a startup requirement
for shipped assets.

## Verdict

**Hash each shipped navpack when it is baked/packaged. Give the binary an
explicit bundled-identity table. Bundled startup checks profile/header
compatibility, reads the pack once, decodes once, and shares
`Arc<NavWorld>`. Do not SHA-256 that file at startup. External and
development packs hash the exact bytes they decode, once, then reuse the
same world. Omit `.navflags` from the ordinary user zip.**

Design approval is not source acceptance and not a measured load-time win.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD | `6d585fd92daa7a14003aa7d22871ece0790d95af` |
| Last nav/profile product | `d321908cce55ae8c7fc1a07f2e43f6e9690c5422` |
| Client gitlink | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 53 SHA-256 | `f965d745cbc9656f9616f27fe89e3824a7c44c6266c86828b60785de9a3537a8` |
| Kanban card | `t_66955ad4` |

Native 06c sizes (~73 MiB pack, ~261 MiB flags) remain the cost that
three `hash_file_with_progress` passes still pay. Historical
`validate_for_play` failures after a disk rewrite are the old
revalidation contract, not the new one.

## What source does now

`ProfileSelection::validate_nav` streams SHA-256 over the pack and, if
present, the flags file, then compares `NavManifest`.
`SharedClientTemplate::load` calls `validate_resources` (second nav
hash) and then `NavWorld::load_pack`, which `fs::read`s again; BadMagic
falls back through `load_grid` and rereads by path. `validate_for_play`
hashes a third time. Slots and the picker already share one
`Arc<NavWorld>` (`SharedClientTemplate.world`, `picker::set_pack`).
Raw flags are paint-only, decoded lazily, dropped when both collision
toggles go off, and matched only by origin/width/height.

`nav-pack` already hashes bake output into `<pack>.json`
(`revision`, `cache_id`, `nav_sha256`, `flags_sha256`).
`known-cache-identities.json` is the existing compile-time recognition
table. `panel/build.rs` bakes git identity; `--release` is an
optimization profile, not a product class (06-build-capabilities).
Default `~/.274bot/274bot.navpack` is a development path, not a bundle.

## Package boundary

Bundled treatment is selected only by a **non-empty compile-time table**
plus an **install-relative path** and **no `--nav-pack` / `NAV_PACK`
override**. Empty table (git checkouts, ordinary `cargo build
--release`) means every load is external. A user-written
`274bot.navpack.json`, a matching filename, or Cargo `--release` must
not flip the fast path.

Minimal table row, analogue of `known-cache-identities.json`, owned by
host-play:

```
revision, cache_id, format ("274V8"), nav_sha256, flags_sha256 (null on
user zip), relative_path
```

`nav_sha256` names the packaged asset. Reading it at startup is not a
fresh disk verification. Post-packaging tamper detection waits for a
signed or otherwise provenance-checked zip; do not add a trust dialog.
Rebake produces a new `nav_sha256` even for the same revision.

Until packaging is authorized, keep the table empty in git. `nav-pack`
continues to write the sidecar used by the external path.

## Runtime ownership

Replace the three nav disk hashes with one origin-aware load that
returns the immutable world.

- `NavWorld::from_bytes(&[u8])`: `decode`, or `from_grid(decode_grid)`
  on BadMagic, never a second `fs::read`. `load_pack` becomes read +
  `from_bytes`.
- `NavOrigin::Bundled { identity, path }`: resolve the packaged file,
  require `identity.revision` and `identity.cache_id` to match the
  already-captured cache, let `decode` enforce 274V/v8. One read, one
  decode, zero SHA-256. Unsupported/corrupt still fail.
- `NavOrigin::External { path }`: one read, `hash_bytes` of that
  buffer, `NavManifest::verify`, `from_bytes` of the same buffer.
  Missing pack stays `NavAvailability::Unavailable`. Missing sidecar
  stays Legacy274 on 274 and a hard mismatch on 289. No mtime cache.
- `ServerProfile` stores origin + the identity string +
  `Option<Arc<NavWorld>>` after decode. `validate_resources` /
  `validate_for_play` stop rehashing navigation. Cache archive
  revalidation stays. After-load disk edits do not replace the loaded
  world; a new identity requires an explicit new bind/load.
- N bots: unchanged `Arc` share. No mmap, hash database, resource
  manager, or per-bot copies.

## Flags

Ordinary user zip: do not ship `.navflags`. Picker already falls back
to the walk word. Contributor/scenario builds may keep the sidecar.

Do not hash or decode 261 MiB at bind to avoid a later hash. When a
collision toggle turns on, hash the bytes just decoded, require
`flags_sha256` from the same origin identity, and keep the geometry
check. Dimensions alone are not identity. Drop on toggle off as today.

## Progress

Keep the 20-cell `#` bar, `theme::ACCENT` (`#FFB000`), exact counts, and
generation-scoped latest-only observer. Bundled decode publishes
`PreparingNavigation` as "Loading navigation" (steps 0/1 then 1/1, or
bytes if a later decode seam reports them). External hashing publishes
a distinct "Verifying custom navigation" once, then the same decode
stage. Do not emit `CheckingNavigationFiles` three times for one file.
No timer-based percentage. Cache stages are unchanged.

## Build now, without publishing

1. `from_bytes` + origin enum + stop nav rehash after the loaded world
   exists. Development stays external (current sidecar).
2. Empty `bundled-nav-identities.json` and the selection rule so a later
   packager can fill hashes and copy the pack next to the binary.
3. Progress copy + lazy flags hash when paint-on.
4. Update
   `navigation_and_scatter_use_the_selected_shared_world_and_validate_its_sidecar`:
   post-load disk rewrite must keep the same `Arc` world; a new bind
   selects the new identity or rejects it.

Packager work (copy pack into zip, fill the table, omit flags) waits
for packaging authorization. Signing waits longer.

## Focused verification

Native correctness after source review, not LIVE, not latency/RSS:

- Bake/package: bad revision/cache pairing rejected; rebuilt bytes get
  a new `nav_sha256`.
- Bundled fixture: zero `hash_file` calls on the pack, one decode, N=2
  `Arc::ptr_eq`.
- External wrong `nav_sha256` rejected; BadVersion / missing 289
  sidecar / missing pack unchanged.
- After-load rewrite does not replace the in-memory world; explicit new
  load does.
- Flags stay unloaded until paint-on; hash+geometry required; drop
  restores laziness.
- Progress: bundled never shows a nav byte-hash stage; external shows
  one verify pass.

Read/decode counts are the proof. Historical changed-file failures
document the contract being retired for bundled assets and for
already-loaded external worlds.

## Out of scope

Source edits, LIVE, packager zip layout, signing, catalog/timeout
proofs, cache/protocol/session ownership beyond the nav seams above,
and t_dde220ef's frozen tree. This card is design only; implementation
needs its own same-card source review and later branch review.
