# Ardougne stall and pickpocket bank-cycle fixtures

This bounded extension keeps both frozen ArdyCakes and ArdyThiever cards and
script source unchanged. Three shared scenario names:

- `ardy_cakes`: default `guardResponse='Flee'`, `solveClues=false`. Both
  catalogs. Baker's stall stand (2668,3312,0), stall (2667,3310,0), bank
  (2655,3286,0). Cake 1891, bread 2309, chocolate slice 1901. Thieving 5.
- `ardy_thiever`: `thieveTarget='Guard'`, Flee, clues off, `bankAtLootSlots=1`,
  `foodTarget=1`, `restockAtFood=0`. Both catalogs. Stand (2661,3306,0)
  leash 19. Guard coins 995, Thieving 40, 468 XP tenths, stun 8.
- `ardy_thiever_knight`: `thieveTarget='Knight of Ardougne'`, same Flee and
  loot-count banking. Same stand, leash 29. Knight coins 995, Thieving 55,
  843 XP tenths, stun 8.

Frozen ArdyCakes.ts is
`f19ce19422c16e337fdc30678b93772054ee907a35297a1cd4a8f6fb062943d7`
on both `100adccc` and `8e7d965b`. Frozen ArdyThiever.ts is
`3cee6ff99d8f24532a5564fc7215faba19ca78f0156386913497bf266fca8aa5`
on both. Shared `cakeStallData.ts`
`331570423b3c3730afa404b73909373b5150e2c85c799fe78bbf66b282516ccd`,
`CakeStall.ts`
`156cf7143f42b5fbfb1ec580723f5d07928e4a174b3f65bdda91ba4b2ac384ef`,
`targets.ts`
`bed6dfd417ec939b8b40a3bd8eb05258f50470ac6acfab3f4c2558c7dd30f078`,
and `pickpocketTargets.ts`
`13e73cb59a58c0009fa57bbb75e09b90fc5c9c58201a26fd729901f681df2d15`
match remaining-combat-world sections 9-10. Selected 274/289 items: cake
1891, bread 2309, chocolate slice 1901, chocolate cake 1897 (rejected),
noted 1892/2310/1902 (rejected), coins 995. Thieving stat id 17. This card
does not add runtime code. Fight/`isHostileAttacker` stays pending. Clue
solving stays an honest stub. PeriodicBank Off is not a bank proof;
ArdyThiever BankRun at `bankAtLootSlots=1` is.

Each core seed is an empty pack, prepared Thieving/Hitpoints, and tele to
the selected stand. ArdyCakes must steal stall food with Thieving XP, deposit
acquired stock in a fresh bank generation, empty the pack of cake 1891
(`ItemIdAtMost` at 0), close, return to STAND and steal again. Sequential
scenario arms use cake 1891 as the dated identity; the catalog witness
accepts cake/bread/chocolate slice so a bread-only steal still qualifies
the LIVE core. ArdyThiever must restock one stall food, pickpocket coins
with Thieving XP, deposit those coins, empty the pack of coins, return to
the Guard/Knight stand and pickpocket again. Stall-only XP/cakes cannot
satisfy the coin arm. Stun without XP, name-only, seeded baseline, stale
or closed bank, unclosed return, or no further work fail. Original
`SCRIPT_GOLD_DEADLINE` is unchanged.

Panel and host-play keep using `scenario::get` / `names()`. Superheater
Attack-30 correction 6c7075ce, vial/potion e4f5352ab, chicken melee
8fd0f138, bank-close generation f33703d5e, seed-order 1ec25b521, TannerBot
3a34f879f, Falador West vial start 3368 (2f1bd9cf), and RuneCrafter/Mule
6750713b8 are unchanged.

## Verification

Export: `.superpowers/review-exports/t464d9c4c-ardy` via
`docs/compat/evidence/build-isolation/export_git_files.py` (regular Git
blobs of host `bca3429ed36d` plus client `9d090ed049` plus owned overlay).
Isolated empty target:
`.superpowers/review-exports/t464d9c4c-ardy-target`
(`isolated_build=true`). Shared campaign target was not used. Catalog
source is not staged.

- `cargo test -p scenario --lib` — 92 passed.
- `ardy_thieving_cases_register_stall_guard_knight_and_bank_cycles` — passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 26 passed, 1 ignored (`LIVE` cell). Frozen-ledger identity needs
  gitignored `.superpowers/inputs` and passed from the isolated export with
  an inputs symlink.
- `ardy_cakes_requires_stall_food_xp_fresh_deposit_return_and_further_steal`
  — passed.
- `ardy_thiever_requires_coins_xp_fresh_deposit_return_and_further_pickpocket`
  — passed.
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` and catalog harness — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance.
Fight/`isHostileAttacker` stays explicit later scope.
