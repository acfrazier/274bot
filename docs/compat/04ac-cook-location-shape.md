# Map native cooking stands into the consumed location shape

Task `t_16aac9ce` implements brief 107 on `codex/rs2b0t-multirevision`. Isolated
checks used exact Git regular blobs of host
`04d7b1f48180ab9b93bbd6fec45a655df4385233` plus the owned overlay. Client
gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3903 exclusive new files;
no archive extraction, overwrite or deletion. One exclusive target
`.superpowers/review-exports/cook-location-shape-t_16aac9ce-target`.
Not LIVE.

## Bridge

Posted `host().content.cook_stands` stays the Catherby row from
`api::content::COOK_STANDS`: bank `(2809,3441,0)`, range stand `(2817,3443,0)`.
`cook_locations_data.js` maps that row into the shape frozen CookBot actually
reads: `where.bank.tile` using the same `{name, tile}` bank object as named
banks, and `where.surface.stand` as the host range target.

Unsupported facts stay explicit null/false. There is no native approach, loc
tile, or loc name on COOK_STANDS, so those fields are null. `verified` is
false. CookBot then uses `plan?.locName ?? settings.rangeName` (`Range`) and
`plan?.arriveRadius ?? 0`. Missing `bank.access` / `npcAccess` keeps the
existing `openBooth` path. Auto has no nearest-bank pairing or `bankUnlocked`
policy, so `resolveCookLocation('Auto')` is null and CookBot stops. Unknown
names do not invent a nearest surface. `buildCookLocations` does not consume
`BANK_LOCATIONS`. Catherby is not added to named banks.

Root 912 old-catalog cook_bot / cook_bot_lobster stopped during onStart
(ticks 45/39). The stop reason was inferred from this shape mismatch, not
logged.

## Proof (isolated empty target)

Export `.superpowers/review-exports/cook-location-shape-t_16aac9ce` with
`CARGO_TARGET_DIR=.../cook-location-shape-t_16aac9ce-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/cook-location-shape/`.

- CookBot-shaped Catherby traversal reads bank `(2809,3441,0)` and stand
  `(2817,3443,0)`. Bank is not a Tile. Old `range` field is absent.
- Posted surface locName/loc are null; stand is not reused as loc; verified is
  false. Consumer locName fallback is `Range`, arriveRadius 0, open path booth.
- Auto, Seers, and Falador East stop with `no bank called '…'`; no nearest
  fallback. Custom does not stop. Options stay Auto/Catherby/Custom.
- Named-bank aliases remain Falador East…; Catherby is not among them.

`cargo test --locked --offline -p script --test cook_location_shape -- --test-threads=1`:
3 passed. `cargo test --locked --offline -p script --test named_bank_locations -- --test-threads=1`:
4 passed. Clippy `-D warnings` on `script --lib` and
`script --test cook_location_shape` passed.
`rustfmt --edition 2021 --check` on `cook_location_shape.rs` passed.

## Limits

Root owns fresh catalog LIVE for cook_bot / cook_bot_lobster and any later
missing operation after startup. Auto nearest-bank pairing, `bankUnlocked`,
foreign surface database, approach/loc tiles, and new navigation/timeouts are
out of scope. Exclusive target cache 3.6G; root owns cleanup after review.
