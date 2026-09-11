# Bounded native Baker stall mapping (selected loc 2561)

Task `t_24333a16` implements brief 97 on `codex/rs2b0t-multirevision` after
named-bank review. Isolated checks used exact Git regular blobs of host
`7c2eef6eb8db6b01933c333aa64cf5f68169d8d2` plus the owned overlay. Client
gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3526 exclusive new files;
no archive extraction, overwrite or deletion. New empty target
`.superpowers/review-exports/cake-stall-native-t_24333a16-target`. Not LIVE.

## Bridge

Host-owned Baker stall facts live in `api::cake_stall` and are posted on
`__rs2b0t_host.content.baker_stall` through the existing `content_json` seam,
same shape as pickpocket spots. Selected 274/289 headed loc type **2561** is
name `Baker's stall`, op2 `Steal from`, 2×2 `block_walk`, at `(2667,3310,0)`.
A second same-id stall at `(2655,3311,0)` is rejected by the posted stall
neighborhood (Chebyshev ≤ 3). LIVE occupancy at `(2668,3312,0)` is the walk
stand. Alternate `(2669,3310,0)` and flee `(2655,3298,0)` are the remaining
selected-cluster tiles; they were not independently pack-walked in this
slice. Unknown profiles do not invent extra stalls. Missing or malformed
posted facts fail closed (`no-progress`, no loc command).

`stealCakes(opts)` is one walk-then-interact onto existing `Traversal.walkTo`
and `Loc.interact`. Traversal is registered before CakeStall so that import
resolves. Selection filters name, steal op, loc id 2561, and the posted stall
tile before nearest. The loc command keeps selected id/tile/op. Caller
abort/shouldEat/fillTo/tick lockout and full/stocked gates skip dispatch.
After an awaited walk the mapping re-reads facts and the loc. Observation wait
is 2400 ms (existing resolve bound). `onSteal` fires once on a stall-food
delta (Cake/Bread/Chocolate slice); admission or chocolate cake 1897 does not.
`onReset` is never called. `classifySteal` stays `not impl`.

Return contract: `'stocked' | 'combat' | 'aborted' | 'no-progress'`. One food
below `fillTo` is honest `no-progress`, not stocked. Boolean true/false is not
a result. This is not 90s CakeStall controller equivalence.

## Proof (isolated empty target)

Export `.superpowers/review-exports/cake-stall-native-t_24333a16` with
`CARGO_TARGET_DIR=.../cake-stall-native-t_24333a16-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/cake-stall-native-mapping/`.

- Posted pins: stall `(2667,3310,0)`, stand `(2668,3312,0)`, alt `(2669,3310,0)`,
  flee `(2655,3298,0)`, name/op, cake items.
- Closer door plus farther qualifying stall queues loc 2561 `Steal from`.
- Other same-id stall is not the interact target; depleted/wrong-op/absent
  facts queue nothing.
- Abort/eat/combat/lockout/full/fillTo skip dispatch; walk then deleted facts
  revalidates to `no-progress`.
- One cake below fillTo 28 calls `onSteal` once and returns `no-progress`;
  fillTo 1 returns `stocked`. Chocolate cake does not count. No invented reset.

`cargo test --locked --offline -p api cake_stall -- --test-threads=1`: 5 passed.
`cargo test --locked --offline -p script --test cake_stall_native_mapping -- --test-threads=1`:
6 passed. Clippy `-D warnings` on `api --lib` and
`script --test cake_stall_native_mapping` passed.
`rustfmt --edition 2021 --check` on the new Rust files passed.

## Limits

Root owns repeating the six failed Ardy families plus newer counterparts and a
native run under unchanged gold bounds (actual steal/XP, deposit, closed
return, further work). One-attempt mapping is not full controller acceptance.
No foreign/LIVE/fixture, load.rs/slot.rs/host-play/nav/client, timeout, dim,
or packet changes. `classifySteal` remains unsupported.
