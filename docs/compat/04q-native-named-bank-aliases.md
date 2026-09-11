# Publish selected-world named bank aliases before catalog evaluation

Task `t_bced5c76` implements brief 89 on `codex/rs2b0t-multirevision` after
count-dialog review. Isolated checks used exact Git regular blobs of host
`8bd34675b61c94707e6c5ed07a943102bf06cac5` plus the owned overlay. Client
gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3371 exclusive new files;
no archive extraction, overwrite or deletion. New empty target
`.superpowers/review-exports/runtime-named-banks-t_bced5c76-target`. Retry after
the disk-full crash of run 1281 reused the same named overlay (not an
independently empty-started target). Not LIVE.

## Bridge

`BANK_LOCATIONS` is import-time content from the bound `NavWorld`. Play resolves
the five shim `RUNES.bank` names against packed `bankbooth` loc tiles and the
walk surface once, then posts `{name,x,z,level}` rows on
`__rs2b0t_host.content.named_banks` before modules evaluate. The JS array maps
those rows to `{name, tile}` stands. Missing world, missing cluster booths,
wrong plane, a stand on a booth loc, or a blocked stand with no Chebyshev-1
walkable replacement omit that alias, so `BANK_LOCATIONS.find` stays undefined
and catalog `bankTile` still throws.

Preferred Falador East `(3013,3355,0)` and Varrock East `(3253,3420,0)` pass
packed 274/289 validation. Draft Edgeville `(3094,3493,0)` and Draynor
`(3093,3243,0)` are distance-2 from their packed booths; both packs publish
derived adjacent stands `(3094,3491,0)` and `(3092,3242,0)`. Al Kharid keeps
`(3269,3167,0)`. Packed booth rows stay unnamed `"Bank booth"`. `nearestBank`
still follows live `nearest_booth`. `COOK_LOCATIONS` stays the host cook-stand
set (Catherby).

Existing `spawn` / `spawn_with_game_data` constructors post empty aliases.
Production Play/slot construction passes the already-resolved facts. A new
profile or isolate does not mutate a process-global table.

## Proof (isolated empty target)

Export `.superpowers/review-exports/runtime-named-banks-t_bced5c76` with
`CARGO_TARGET_DIR=.../runtime-named-banks-t_bced5c76-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/runtime-named-banks/`.
Pinned packs (not git blobs) used only for the nav check:
274 `05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4`,
289 `131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924`.

- Isolate with named content and no snapshot: `BANK_LOCATIONS.find('Falador East').tile`
  is `(3013,3355,0)`; Mule `bankTile` shape returns that tile.
- Unknown name is undefined; `bankTile('No Such Bank')` throws
  `bankTile: unknown bank 'No Such Bank'`.
- Empty facts: `BANK_LOCATIONS` is `[]` and Falador `bankTile` throws.
- Independent isolates keep Falador-only vs Varrock-only rows; a later empty
  spawn does not inherit the earlier table.
- `nearestBank` follows a posted non-alias `nearest_booth`.
- `COOK_LOCATIONS` names remain `['Catherby']`.
- Bound world without Falador booths omits only Falador; NPC teller access is
  not a named booth alias.
- Both pinned packs publish all five aliases with equal stands.

`cargo test --locked --offline -p api named_banks -- --test-threads=1`: 6 passed.
`cargo test --locked --offline -p nav --test named_bank_aliases -- --test-threads=1`:
3 passed. `cargo test --locked --offline -p script --test named_bank_locations -- --test-threads=1`:
4 passed. Clippy `-D warnings` on `api --lib`, `nav --test named_bank_aliases`,
and `script --test named_bank_locations` passed. `rustfmt --edition 2021 --check`
on the new Rust files passed.

## Limits

Root owns isolated Mule LIVE requalification and later
withdraw/enter/craft/exit/deposit/restock/return. An exception disappearing is
not full Mule acceptance. No foreign/LIVE/fixture, ledger, timeout, ranking or
packet changes. No 18-row BankLocations table, quest gates, NPC/chest access,
or cook pairing from named banks.
