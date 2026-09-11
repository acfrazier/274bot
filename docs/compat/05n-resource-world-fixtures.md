# Magic-tree and coal-truck resource-cycle fixtures

This bounded extension keeps both frozen GnomeMagicChopper and CoalTrucks
cards and script source unchanged. Four shared scenario names:

- `gnome_chop`: `fletchLogs=false`. Both catalogs. West magics
  (2372,3425,0), upstairs gnome booth (2445,3425,1), south bank stairs
  (2444,3416,0) radius 30 covering the north pin (2445,3443,0). Magic
  logs 1513, Woodcutting 75, steel axe 1353.
- `gnome_fletch_short`: `fletchLogs=true`, Fletching 80 and at most 84.
  Knife 946. Unstrung magic shortbow 72. Strung 861 is rejected.
- `gnome_fletch_long`: `fletchLogs=true`, Fletching 85. Unstrung magic
  longbow 70. Strung 859 is rejected.
- `coal_trucks`: no SETTINGS. Mine (2582,3481,0), mine truck stand
  (2575,3486,0). Coal 453, Mining 30, steel pickaxe 1269. Partial
  mine/truck/further cycle. Seers haul/bank/return is not claimed.

Frozen GnomeMagicChopper.ts is
`3b0d034c761f9b98361561627817a6d806d359f659000cbccdf22e25772471e1`
on both `100adccc` and `8e7d965b`. Frozen CoalTrucks.ts is
`d20112f0bfb44612ae53e0c56009c6b3e50808375c2a54dc43ed2160e55fcf36`
and CoalTrucksLogic.ts
`d6ed36c14e276cc9f85ffdc59fe910a1ec00eb63c162cd19c7cd27a021bc97b5`
on both. Selected 274/289 items: magic logs 1513, noted 1514, unstrung
short 72 / long 70, noted 73/71, strung 861/859, knife 946, steel axe
1353, coal 453, noted 454, steel pickaxe 1269. Woodcutting stat 8,
Fletching 9, Mining 14. Selected loc shots on both revisions: Magic
tree 1306 at south-bank/east pins with Chop down; gnome bank staircase
1742 Climb-up at (2444,3414,0) beside the south pin. Coal rock loc ids
2096/2097 are the frozen `miningRocks.ts` / CoalTrucksLogic content
(item 2096 is Odd cocktail in a different namespace). This card does
not add runtime code. Script-owned death/boat/shop recovery and the
missing-knife stop stay out of ordinary core claims.

Each gnome seed is an empty pack of logs/bows, prepared Woodcutting 75,
steel axe held, and tele to west magics. Fletch variants also seed
Fletching 80/85 and knife 946. GnomeMagicChopper always opens the
upstairs bank for gear prep; that trip is not the log/bow deposit.
`gnome_chop` must chop 1513 with Woodcutting XP, deposit those logs in a
fresh upstairs bank generation, empty the pack of 1513, close, return
to ground stairs and chop again. Fletch variants must then consume
those logs into exact unstrung 72 or 70 with Fletching XP before the
same upstairs deposit/return/further chop. Name-only, seeded baseline,
stale or closed bank, unclosed return, XP-only, strung 861/859, the
other unstrung id, noted 1514/73/71, missing knife, or no further work
fail.

Coal seed is an empty pack of 453, Mining 30, steel pickaxe, tele mine.
The script only hauls to Seers after truck 120 is full. Filling 120
coal at the script's measured ~2s/coal plus five deposit trips and the
Seers walk exceeds `SCRIPT_GOLD_DEADLINE` 180s. No truck-content seed
primitive exists (`give` / `givebank` only). Root coordination is
requested for any missing truck seed. The fixture therefore observes
real mining XP and coal 453, a mine-truck deposit (pack empty of coal
at the truck stand with the bank closed and bank coal unchanged), then
further mining because the truck is not yet full. A Seers booth deposit
cannot satisfy the truck arm. Original `SCRIPT_GOLD_DEADLINE` and
`SCRIPT_GOLD_WATCH_TICKS` are unchanged.

Panel and host-play keep using `scenario::get` / `names()`. Superheater
Attack-30 correction 6c7075ce, vial/potion e4f5352ab, chicken melee
8fd0f138, bank-close generation f33703d5e, seed-order 1ec25b521,
TannerBot 3a34f879f, Falador West vial start 3368 (2f1bd9cf),
RuneCrafter/Mule 6750713b8, rune observation 4cc6cc27d, and Ardy
stall/pickpocket 43749c42f are unchanged.

## Verification

Export: `.superpowers/review-exports/t_aa061a28-resource` via
`docs/compat/evidence/build-isolation/export_git_files.py` (regular Git
blobs of host `8bd34675b61c` plus client `9d090ed049` plus owned overlay).
Isolated target: `.superpowers/review-exports/t_aa061a28-resource-target`
(`isolated_build=true`). Shared campaign target was not used. Catalog
source is not staged. Run 1278 crashed disk-full during catalog/clippy;
retry-r1286 reused that exclusively owned target after overlay sha256
matched and did not independently empty-start it.

- `cargo test -p scenario --lib` — 94 passed.
- `resource_world_cases_register_gnome_log_bank_fletch_and_coal_truck` — passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 31 passed, 1 ignored (`LIVE` cell). Frozen-ledger identity needs
  gitignored `.superpowers/inputs` and passed from the isolated export with
  an inputs symlink.
- `gnome_chop_requires_log_xp_upstairs_deposit_ground_return_and_further_chop`
  — passed.
- `gnome_fletch_requires_unstrung_product_xp_deposit_return_and_further_chop`
  — passed.
- `coal_trucks_requires_mining_xp_truck_deposit_not_bank_and_further_mine`
  — passed.
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check` on scenario and the catalog harness — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance. Seers
haul/bank/return stays an explicit later cell pending a truck-content
seed primitive.
