# Cooking, furnace smelting, and flax spinning bank-cycle fixtures

This bounded extension keeps both frozen CookBot, SmelterBot and
FlaxSpinner cards and script source unchanged. Five shared scenario
names:

- `cook_bot`: CookBot Catherby Range, `fish='Raw salmon'`,
  `location='Catherby'`, `surface='Range'`. Bank (2809,3441,0), range
  stand (2817,3443,0). Raw salmon 331, Salmon 329, Cooking 80.
- `cook_bot_lobster`: same Range path, `fish='Raw lobster'`. Raw
  lobster 377, Lobster 379, Cooking 80.
- `smelter_bot`: SmelterBot default Bronze. Al-Kharid bank
  (3269,3167,0), furnace (3275,3185,0). Copper 436, tin 438, bronze
  bar 2349, Smithing 1.
- `smelter_bot_steel`: `bar='Steel'`. Iron 440, coal 453, steel bar
  2353, Smithing 30.
- `flax_spinner`: FlaxSpinner default `product='Flax'`. Bank
  (2722,3493,0), wheel (2711,3471,1). Flax 1779, bow string 1777,
  Crafting 1.

Frozen CookBot.ts is
`de121188f8d19cbe0d83f935c886444325680f9b59d252536fe1a49300cf9927`
and CookBotLogic.ts
`ef9808f30748ae0fea51362542b5311c2adf010266556763ee4caf91e8c64340`
on both `100adccc` and `8e7d965b`. Frozen SmelterBot.ts is
`328ba40736f5330e4b37beeded549f1b71c9dfaa994005646c1202bb62e2b809`
and SmelterBotLogic.ts
`f705ec6ec95ab0eda9aea477e98f7fab3a990d659a9a754bcdae64c7a5873665`.
Frozen FlaxSpinner.ts is
`9a5a82f48f990954747f5eaaa49463be3d8d076809f2aa624f3b40623c8be8e6`.
Selected 274/289 items match on both revisions. Generated game-data
JSON has no loc types; scripts query loc names Range / Furnace /
Spinning wheel / Ladder. Make-menu component IDs are snapshot-posted
buttons, not frozen constants. Cooking 80 is a fixture seed so
ordinary burn randomness does not replace the salmon/lobster product
contract; the script encodes no cook level. CookBot Fire stays behind
native fire work. Smithing main panel and LeatherCrafter are not this
hop.

Each cook/smelt/spin seed is an empty pack of input and product at the
script bank, prepared skill, and `givebank` of unnoted raw only. The
script opens that bank for the first withdraw; that trip is not the
product deposit. After Start the serial watch is station arrival, then
relevant XP, then exact unnoted product, then consumed input, then a
fresh loaded-bank deposit, pack empty of product, raw restock, closed
bank, return to the actual station, and further product. XP is never
armed after the product it must witness.

Name-only, seeded baseline product, stale or closed bank, unclosed
return, XP-only, no input consumption, noted cert ids, burnt fish 323
/ 343 / burnt lobster 381, the other cooked fish, iron/steel/bronze
cross-bars, ball of wool 1759, ground-floor flax return, missing
Cooking 80 / Smithing 30, or no further work fail.

## Adapter audit

- CookBot Range: `Bank.openBooth` / `withdrawLoad` (host
  `withdraw-load`), loc name `Range`, raw `useOn` oven, `ChatDialog.make`
  via posted `make_products` qty buttons (`ifButton`). Count-dialog is
  not on this path. Fire / `lightFire` is out of scope (`t_79175534`).
- SmelterBot: `Bank.openBooth` / `withdrawX` / `depositInventory`, loc
  name `Furnace` op `Smelt`, `ChatDialog.makeX` (`ifButton` qty=-1,
  `countDialogOpen`, `answerCountDialog`). Count-dialog reader/answer
  is mapped (`78cc99d07`). LIVE still waits Make-X `t_4b04cb5f` if
  world posting fails.
- FlaxSpinner: `Bank.withdraw` All, ladder `Climb-up`/`Climb-down`,
  wheel `Spin`, `ChatDialog.makeX`. Same Make-X LIVE gate.
- This card does not implement missing runtime operations.

Panel and host-play keep using `scenario::get` / `names()`. Resource
gnome/coal 6fc633317, Superheater Attack-30 6c7075ce, vial/potion
e4f5352ab, chicken melee 8fd0f138, bank-close generation f33703d5e,
seed-order 1ec25b521, TannerBot 3a34f879f, Falador West vial start
3368 (2f1bd9cf), RuneCrafter/Mule 6750713b8, rune observation
4cc6cc27d, and Ardy stall/pickpocket 43749c42f are unchanged.

## Verification

Isolated export and target paths are recorded in
`docs/compat/evidence/station-production-fixtures/verification.json`.
No LIVE or fixture process is launched. Root owns the catalog ×
revision cells after review. Source review does not grant live
acceptance.
