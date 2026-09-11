# Vial filling and potion bank-cycle fixtures

This bounded extension keeps both frozen VialFiller and PotionMaker cards and
script source unchanged. Four shared scenario names:

- `vial_filler`: `buyVials=false`, Falador West (2946,3369,0) to the west
  fountain (2949,3381,0). Both catalogs.
- `vial_filler_east`: same fill, `bank=Falador East` (3013,3355,0). Both
  catalogs. The frozen source lists that stand; shop-buy stays pending.
- `potion_maker`: default Custom Guam leaf + Eye of newt. Both catalogs.
- `potion_maker_named`: named selector `herb=Ranarr weed` /
  `secondary=Snape grass`. Both catalogs. Distinct herb/unf/secondary/finished
  ids, not an equivalent Guam constant.

Frozen VialFiller.ts/Logic hashes match remaining-production section 21:
`58f95dca5a65bfc6048c1da6a3eda8a8153b90979c0a30c237e76ed437c8bd4c` /
`e704f90e6e6b96a44a408edad53fb45c92f64b682505b0b2637627e90767bfeb`.
Frozen PotionMaker.ts/Logic:
`36704aa9951b18bcb27c16d5740577937757847209daa504b0df5ecd9e758beb` /
`0acc3221465d8322ecaf2fb2a14a253b81c0b3adecb19a44675c6832aa18636e`.
Selected 274/289 items: empty vial `vial_empty` 229, water `vial_water` 227,
Guam leaf 249, Eye of newt 221, unfinished `guamvial` 91, Attack potion(3)
`3dose1attack` 121, Ranarr weed 257, unfinished `ranarrvial` 99, Snape grass
231, Prayer potion(3) `3doseprayerrestore` 139. Filling grants no XP.

VialFiller core seeds an empty pack, 56 banked empty vials, no water vials,
and tele to the selected Falador stand. The script must withdraw empties,
fill at the fountain (exact 229→227 while standing there), deposit those
script-created water vials into a fresh bank generation, restock empties,
close, return to the fountain and fill again. Queued useOn, name-only vials,
seeded water, or a fill away from the fountain fail. `buyVials=true` / Jatix
`Shop.buy` remains pending on the already queued shop capability.

PotionMaker uses `nearestBank`, so these cells reuse the accepted Varrock West
booth (3185,3440,0). Seed before Start: empty pack, Herblore 3 (default) or 38
(named; source has no level field, so this is fixture-sufficient rather than a
generated-data requirement), `setvar druidquest 4` plus relog so the journal
row `Druidic Ritual` is green, banked herb/water/secondary ×42, no unfinished
or finished potions. Named also banks leftover Guam 249 ×14 that must stay.
The frozen script queues batched `useOn` calls; this fixture does not rewrite
that policy or change timeouts. Independent catalog witness needs unfinished
id after herb+water, then that unfinished falling into the finished id with
Herblore XP, deposit of script-created finished potions, same-generation
herb/water restock, closed bank, then further unfinished from the restock.
Unfinished-only, seeded product, XP-only, name-only, wrong recipe ids, or
named Guam withdrawal fail.

Panel and host-play keep using `scenario::get` / `names()`. Superheater
Attack-30 correction 6c7075ce and existing `SCRIPT_GOLD_DEADLINE` are
unchanged.

## Verification

Implementation baseline: host `141f9ccece0d4944d035f53805873c714b91cdc2`,
client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`, plus these owned files.
Frozen export: `/Users/acfrazier/experiments/274bot/.worktrees/t_534895e5-src-24432659`
(git archive of `244326594` + client archive + owned overlay; frozen catalog
inputs linked read-only). Isolated empty target:
`/Users/acfrazier/experiments/274bot/.worktrees/t_534895e5-target-24432659`
(`isolated_build=true`). Shared campaign target was not used.

- `cargo test -p scenario` — 89 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 23 passed, 1 ignored (`LIVE` cell).
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance.
