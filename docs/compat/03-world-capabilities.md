# World capabilities: revision-bound source and bake qualification

Scope: selected source/cache/navigation binding plus controlled Mac region,
door and lamp Guardian qualification for revisions 274 and 289. Bank return
remains pending the first banking capability correction and review.

## Product changes

`nav::manifest` is now the single production format for cache and navigation
identity. `CacheManifest` binds an explicit revision to SHA-256 hashes of the
eight client archives. `NavManifest` binds revision, cache identity, the v8 nav
pack SHA-256, and the optional raw-flags SHA-256. `host-play` consumes this
shared format and still permits the existing unmanifested legacy 274 pack.
Revision 289 requires a matching manifest.

`nav-pack` retains its positional legacy 274 mode. Its revision-bound mode
requires all of `--revision`, `--content`, `--cache`, `--cache-manifest`, and
`--out`; optional `--flags-out` otherwise derives beside the pack. It verifies
the declared revision, every client archive, and the exact config archive before
reading world data or writing outputs. Revision 274 selects
`data/pack/config`; revision 289 selects `data/pack/client/config`.

A wrong 274 manifest supplied to a 289 bake failed with exit 1 before the
sentinel output changed. The exact command, error and equal before/after hashes
are in `evidence/world-capabilities/bake-summary.json`.

## Pinned offline inputs

The bakes used the local fixture paths recorded in `fixture-inputs.json`:

| Revision | Content commit | Content root | Cache root | Config used |
|---|---|---|---|---|
| 274 | `000c19997e07206131bcb3c884265840efce416d` | `/Users/acfrazier/experiments/Server/content` | `/Users/acfrazier/experiments/Server/engine/data/pack/client` | `/Users/acfrazier/experiments/Server/engine/data/pack/config` |
| 289 | `92649430fcbc83538d8c4367ecb96cee1a67a944` | `/Users/acfrazier/experiments/lostcity-289/content` | `/Users/acfrazier/experiments/lostcity-289/engine/data/pack/client` | `/Users/acfrazier/experiments/lostcity-289/engine/data/pack/client/config` |

The 274 content tree had one unrelated untracked test-cheat source already
recorded by `fixture-inputs.json`; the bake made no external-tree changes. The
289 content tree was clean. Generated packs remain gitignored under
`.superpowers/world-capabilities/{274,289}` and did not overwrite the user's
`~/.274bot` resources.

## Fresh bake results

| Revision | Cache identity | Mapsquares | Grid | Walkable | Transports | Teleports | Bank stands | Pack SHA-256 | Flags SHA-256 |
|---|---|---:|---:|---:|---:|---:|---:|---|---|
| 274 | `4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c` | 483 | 1792x9088 | 1,152,462 | 2,143 | 7 spell + 32 jewellery | 68 | `05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4` | `92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb` |
| 289 | `c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09` | 534 | 1792x9088 | 1,210,899 | 2,181 | 7 spell + 40 jewellery | 68 | `131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924` | `67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a` |

The exact bake commands and byte counts are machine-readable in
`evidence/world-capabilities/bake-summary.json`. The raw bake receipts are
`bake-274.log` and `bake-289.log`; the cache and nav manifests are retained
beside them.

The unchanged v8 representation decoded successfully for both outputs. The
289 differences are source-derived rather than an assumed 274 copy: 51 more
mapsquares, 58,437 more walkable tiles, 38 more transport edges and eight more
jewellery teleport edges. Both selected worlds derive 68 bank stands.

## Selected-cache source audit

`nav-input-audit` verifies the cache and nav manifests, decodes the real selected
config/interface archives, reloads the v8 world, and inventories existing Rust
food/navigation/guardian inputs. Full deterministic outputs are
`input-audit-274.json` and `input-audit-289.json`.

Bank/control findings:

- `bank_main` is root 5292 in both archives. Its inventory is component 5382.
  Capacity is 240 (8x30) in 274 and 288 (8x36) in 289. The five decoded
  inventory operations remain Withdraw 1/5/10/All/X in the same order.
- The controls root is 147 in both. Snapshot's existing indices decode to the
  same selected buttons in both archives: retaliate on/off 150/151 and run
  off/on 152/153. Revision 289 adds later emote children under the nested emote
  layer, but does not move these four early controls.
- No verified selected-cache mismatch requires a change in root-owned
  `api/snapshot.rs`.
- The selected shop root/inventory and trade side/main inventories, other-player
  label, main root and confirmation root all decode at the audited source IDs;
  their decoded component shapes are equal across the two selected archives.

Existing content findings:

- All 25 `FOOD_HEALS` names resolve to cache object IDs in both revisions, with
  the same mappings and heal values.
- Both worlds derive seven spell teleports. Item/jewellery teleport edges grow
  from 32 to 40 in 289, and are retained from the selected source rather than
  normalized away. Full endpoints, requirements, costs and option IDs are in
  each audit JSON.
- Transport endpoints are serialized from the decoded v8 pack: 2,143 edges for
  274 and 2,181 for 289. The bake logs preserve per-kind and explicit skipped-row
  counts; skipped script forms remain the existing incomplete feature rather
  than being guessed.
- The current guardian-selected interface roots and selected controls all decode
  in both archives. Existing guardian NPC name inputs resolve to the same cache
  ID sets in both revisions, as does the `whirlpool` loc input. Existing solver
  keys and unknown-event hold policy were not expanded or changed.

These are source/cache shape checks, not proof that server packets present the
same live modal/NPC state or that a solver completes a random event.

## Verification

Passed on the campaign worktree:

- `cargo test -p nav`: 264 tests total across lib/bin/integration targets, no
  failures (254 nav library, 7 nav-pack, 3 resource-manifest).
- `cargo test -p host-play --test session_profile`: 10 passed, including cache/nav
  mismatch rejection before shared resource construction and both-revision
  profile construction.
- `cargo fmt --all -- --check`.
- `cargo clippy -p nav --all-targets --no-deps -- -D warnings` and
  `cargo clippy -p host-play --all-targets --features memory-profile --no-deps
  -- -D warnings`: changed targets passed.
- `git diff --check`.

## Evidence inventory

- `evidence/world-capabilities/bake-summary.json`: commands, identities, hashes,
  counts and wrong-revision negative.
- `evidence/world-capabilities/cache-manifest-{274,289}.json`: selected cache
  archive identities.
- `evidence/world-capabilities/nav-manifest-{274,289}.json`: pack/cache/revision
  bindings.
- `evidence/world-capabilities/bake-{274,289}.log`: fresh bake receipts and
  source-derived counts.
- `evidence/world-capabilities/input-audit-{274,289}.json`: selected cache,
  interface, food, guardian, transport and teleport inventories.

## Still pending outside this offline task

Root still owns controlled live qualification against its frozen sources:
actual door passage, region movement, bank return including the 289 capacity,
and guardian hold/resume/reset observations. Ordinary 289 bot operation remains
gated. This report does not remove that gate, claim live acceptance, change
script suitability policy, or qualify a public release.

## Controlled world and Guardian qualification

Six scoped Mac cells passed with the selected fresh nav pack and local engine.
Every account completed actual mainland seed and observed logout/relogin before
its behavior baseline. Both nav_full cells cleared the scenario's engine-speed
override; no shared engine timing was changed. These are functional results,
not performance measurements.

| Revision | Case | Frozen host / client | Required observed result | Exit |
|---|---|---|---|---:|
| 274 | nav_full | 367af78f / 6cb5a0b1 | Route from (3220,3212) to (3220,3264), scene 2, crossed mapsquare | 0 |
| 289 | nav_full | 367af78f / 6cb5a0b1 | Same selected-world route and arrival predicate | 0 |
| 274 | nav_door | 43a57c36 / 6cb5a0b1 | Fresh scene closed 1530 at (2816,3438), open 1531 at (2816,3439), inside (2817,3443) | 0 |
| 289 | nav_door | 43a57c36 / 6cb5a0b1 | Same closed/open/inside observations through selected-world Traveller | 0 |
| 274 | guardian_lamp | 43a57c36 / 6cb5a0b1 | Lamp 2528, host hold/claim, real skill interface, consumption, Strength XP, hold release and walk to (3221,3212) | 0 |
| 289 | guardian_lamp | 43a57c36 / 6cb5a0b1 | Same complete Guardian predicates and resumed host action | 0 |

Raw logs and receipts: `evidence/world-capabilities/live/`. The 367af78f binary
SHA is `03c964a1f0fb014238b0fe419f2c92119359e505b6c96a452fee562fb6bb57b1`;
43a57c36 SHA is `1d3433903046c6a0c79252090c1e4b3b63a4b2bcdb5390256c72dfe517690155`.
Root reverified all 486 exported source hashes after each build. These candidates
exclude concurrent banking/client changes. Exact source/build/binary receipts
are under `evidence/world-capabilities/harness/`.

### Retained failed and limited cells

- 289 nav_door at 367af78f failed, exit 1, after its 180 s runner bound with
  zero route ticks. The harness had already proved mainland preparation and
  moved to Catherby, then applied the old x >= 3000 mainland seed heuristic.
  Catherby x=2813 could never release it. Root f52565f0 bypasses only that
  redundant geographic heuristic after the exact mainland seed/relogin proof.
- 289 guardian_lamp at 367af78f failed, exit 1, in the 30 s item-preparation
  wait. `give lamp` did not resolve: both content obj packs map ID 2528 to the
  internal name `macro_genilamp`, and ObjType.getId uses that internal map.
  Root 6b925583 corrects only the fixture command.
- 289 nav_door at f52565f0 exited 0 with door opening and arrival, but its
  pre-route baseline was read during scene loading (a stale shifted tile
  2818,3438). This cell is retained as limited evidence, not the accepted
  closed-before-open proof. Root 43a57c36 waits for actual scene 2 at the
  outside stand; both final cells observe the correct closed/open leaf tiles.
  `catherby-packed-edges.json` confirms both selected packs contain the same
  (2816,3438) door transport; the discrepancy was premature observation.

Same-card Grok 4.5 run 1128 approved the initial corrected harness and the
panel/TUI selected-world ScenarioRunner binding. The subsequent root changes
above are small fixture corrections, checked by focused formatting/diff/build
and the actual failed-then-passing cells; final campaign review covers their
combined source/evidence. The existing 43 Guardian unit tests cover claim,
hold/resume and reset policy. This lamp cell does not claim live qualification
of every supported solver, every frontend, banking or the catalog ledger.
