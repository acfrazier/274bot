# TannerBot conversion and bank-cycle fixtures

This bounded extension keeps both frozen TannerBot cards and script source
unchanged. Two shared scenario names:

- `tanner_bot`: default `hideType='Soft leather'`, `buyThread=false`. Both
  catalogs. Hide 1739, product 1741, tan-all widget 8686.
- `tanner_bot_hard`: `hideType='Hard leather'`, `buyThread=false`. Both
  catalogs. Same hide 1739, product 1743, distinct widget 8690.

Frozen TannerBot.ts hashes match remaining-production section 20:
`b8038b17ac72eec3cf7b2e84f8323e4cd898f1975de7d210cfba333e552b8d7f`
on both `100adccc` and `8e7d965b`. Selected 274/289 items: cowhide alias
`cow_hide` id **1739** (274 display `Cow hide`, 289 `Cowhide`), soft
`leather` 1741, hard `hard_leather` 1743, coins 995. Tanning awards no XP.
A Tanner conversation is not a shop (`SHOPMAIN` 3824 / Dommik). Host
`actions.ifButton` already queues `if-button`; this card does not add
runtime code. `buyThread=true` / Dommik `Shop.buy('Thread')` remains
pending on the already queued shop capability.

TannerBot core seeds an empty pack, 28 banked hides and 5000 coins, no
leather, and tele to Al-Kharid bank (3269,3167,0). The script must withdraw
hides+coins, convert at the Tanner stand (3277,3191,0) after the selected
tan-all widget on interface 679, spend coins, deposit those script-created
leather items into a fresh bank generation, empty the pack of leather
(`ItemIdAtMost` product at 0), restock hides, close (IF_CLOSE advances
generation 1→2), return to the Tanner and convert again. The pack-empty
watch sits between bank leather and further leather so the first tan cannot
satisfy the terminal arm. Queued if-button, coins-only, name-only, seeded
leather, wrong product, shop conversion, stale/closed bank, or no further
tan fail. Original `SCRIPT_GOLD_DEADLINE` is unchanged.

Panel and host-play keep using `scenario::get` / `names()`. Superheater
Attack-30 correction 6c7075ce, vial/potion e4f5352ab, chicken melee
8fd0f138, bank-close generation f33703d5e, and seed-order 1ec25b521 are
unchanged.

## Verification

Export: `.superpowers/review-exports/t7dd2b216-tanner` via
`docs/compat/evidence/build-isolation/export_git_files.py` (regular Git
blobs of host `4b69c0e965` plus client `9d090ed049` plus owned overlay).
Isolated empty target:
`.superpowers/review-exports/t7dd2b216-tanner-target`
(`isolated_build=true`). Shared campaign target was not used. Catalog
source is not staged.

- `cargo test -p scenario` — 90 passed.
- `tanner_bot_cases_register_conversion_and_bank_cycles` — passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 24 passed, 1 ignored (`LIVE` cell).
- `tanner_bot_requires_widget_conversion_deposit_restock_and_further_tan`
  — passed.
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` and catalog harness — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance.
