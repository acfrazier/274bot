# Paired NatureCrafter Air and Duel Arena fixtures

`crates/host-play/tests/paired_catalog_live.rs` and
`crates/host-play/tests/support/paired_catalog.rs` are a standalone ignored
LIVE pair harness through production `Play`. Two minted actors share one
loopback `local-274` / `local-289` template. Frozen catalog cards load
through the real registry and controller. Script core loops are not
replaced with harness interactions.

No shared scenario, `catalog_boundary_live.rs`,
`support/two_slot_isolation.rs`, API, runtime, shim, client, nav,
frontend, or engine files were edited. Named-bank `t_bced5c76`, special
`t_68de6f48`, teleport `t_1bf9a22e`, and shop `t_1591d140` stay foreign.

## Cases

Both revisions and both frozen catalogs (`100adccc`, `8e7d965b`) are
selected by env. NatureCrafter.ts
`025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a` and
DuelArena.ts
`5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090` are
identical on both catalogs.

1. NatureCrafter Air. `rune=Air runes`, Master + Runner, exact partner
   screen names. Air ruins `(2983,3288,0)`, Falador East
   `(3013,3355,0)`, unnoted essence 1436, Air 556, Air talisman 1438,
   RC 1. `unnote=null`; Nature island / Jiminua / boat are not accepted.
   Pre-Start seed may place the pair at the ruins and give the runner a
   first unnoted load plus `givebank blankrune 200`. That first pack is
   seed. A later Falador East open/loaded withdraw and return is the
   script restock. Witness: both offer and confirm phases with the
   minted counterpart, 1436 leaving the runner, master positive
   Runecraft XP and Air 556, then restock/return/second transfer/craft
   when they fit the unchanged 180s gold window.

2. Duel Arena Combat Trainer. Two actual scripts. Challenge/Fight
   through posted player ops 1/2. Generated duel controls on both
   caches are 6575 / 6412 / 6733 / 6674 / 6520 / 6671 / 6684 / 6571.
   The frozen catalog still hardcodes those IDs; they match. Seed a
   right-hand `bronze_scimitar` and tele `(3368,3274,0)`. A seeded
   modal or queued Challenge is not a duel. Witness: matching peer,
   both select and confirm modals, fight-pen occupancy, in-combat, and
   Attack/Strength/Defence XP from hits. End/reset and a further
   challenge/combat are attempted inside the same 180s window and are
   not faked.

`SCRIPT_GOLD_DEADLINE` stays 180s and `SCRIPT_GOLD_WATCH_TICKS` stays
150. Preparation is 180s. One shared observation window after both
actors are `ingame && scene_state==2`. If bank-return or a second duel
does not finish inside that bound, the cell may PASS an explicit
partial claim (`first-transfer-craft` / `first-combat`) and must not
print a full-cycle claim.

## Operation gates

Air `Trade.request` maps to `{ op: 'player', name, action: 'Trade' }`.
`Trade.accept` / `decline` use posted `trade_accept_id` /
`trade_decline_id` and throw when id `< 0`. Catalog
`Trade.offerAll(name, filter)` / `Trade.offer(name, n, filter)` extra
arity is ignored by the shim; Air `shortRouteWithdraw` caps at 25 so
the name-only `offerAll` path is the one used. `Bank.openBooth` walks
the hardcoded Falador East tile then opens exact `Bank booth` /
`Use-quickly`; it does not consult `BANK_LOCATIONS`. Shop, named-bank
aliases, special, and `Game.teleport` are unused by these cases.

Duel `Input.interactPlayer(index, 1|2)` reads posted player ops.
`reader.ifText` / `reader.modals().main` / `actions.ifButton` use the
generated control ids. Combat XP must come from hits inside a pen.

## Environment

`LIVE=1` is required for the ignored cells. Missing `LIVE` skips.
Failure prints `FAIL:` and exits 1.

- `PAIRED_CATALOG_REVISION`: `274` or `289`
- `PAIRED_CATALOG_NAV_PACK`: revision-compatible nav pack
- `PAIRED_CATALOG_ROOT`: frozen catalog root
- `PAIRED_CATALOG_COMMIT`: `100adccc037d9f6898080e1cad58fcfc43364775` or
  `8e7d965be2071d6ec65c3265e12af797082d720a`

Optional: `PAIRED_CATALOG_ENGINE_DIR`, `PAIRED_CATALOG_NAV_FLAGS`.
Vault, JS home and settings use private temporary storage. Operator
`~/.274bot` is not read or written.

## Verification

Retry `r1288` after crashed run 1282 (disk-full; no owned sources were
left). Export of committed host `1d4fb9142caf99dcf7d1c02da119710c35ba4e12`
plus client `9d090ed04957e4efc254f073cda97bc5510ca72b` plus owned overlay:
`.superpowers/review-exports/paired-catalog-t_435afcb0`. Isolated empty
target `.superpowers/review-exports/paired-catalog-t_435afcb0-target`
(`isolated_build=true`). Shared campaign target was not used.

- `rustfmt --check -- crates/host-play/tests/paired_catalog_live.rs crates/host-play/tests/support/paired_catalog.rs` — pass.
- `cargo test -p host-play --test paired_catalog_live` — 19 passed, 2 ignored
  (wrong partner, one-sided confirmation, seed-only inventory/XP, stale
  trade, missing conservation, no bank restock, no second transfer, seeded
  modal, queued Challenge without combat, no further full-cycle work,
  frozen hashes, generated duel controls on both caches, operation gates).
- `cargo clippy -p host-play --test paired_catalog_live --no-deps -- -D warnings` — pass.
- `cargo test -p host-play --test paired_catalog_live paired_catalog_air_live -- --exact` — 0 passed, 1 ignored.

No LIVE client or engine was launched. These checks are fixture/build
evidence, not gameplay acceptance. Root owns headless/native cells and
ledger updates.
