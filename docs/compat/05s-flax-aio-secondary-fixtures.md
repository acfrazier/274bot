# FlaxAIO and secondary-collection cycle fixtures

This bounded extension keeps both frozen FlaxAIO and
HerbloreSecondaries cards and script source unchanged. Five shared
scenario names:

- `flax_aio`: FlaxAIO own script, `picking=true`, `spinning=true`.
  Field (2741,3444,0), FlaxAIO bank stand (2725,3493,0), wheel
  (2711,3471,1). Pick flax 1779, wheel makeX to bow string 1777 with
  Crafting XP, deposit strings, closed return to the field, further
  Pick. Not concatenated FlaxPicker/FlaxSpinner.
- `flax_aio_pick`: `picking=true`, `spinning=false`. Full flax pack,
  deposit 1779, closed return, further Pick. Bow string 1777 must not
  qualify.
- `flax_aio_spin`: `picking=false`, `spinning=true`. Banked flax at
  FlaxAIO's own booth stand, wheel conversion+XP, string deposit,
  restock, closed upstairs return, further spin.
- `herblore_secondaries`: default `secondary="Red spiders' eggs"`.
  Egg field (3120,9952,0), Edgeville bank (3094,3493,0). Ground Take
  223, deposit, closed return, further Take. Lobster 379 x50 is the
  permitted food seed; eggs are not given.
- `herblore_secondaries_newt`: `secondary="Eye of newt"`. Betty
  (3012,3259,0), Draynor bank (3093,3243,0). Coins 995 x5000, purchase
  221, deposit, return, further purchase. Distinct from ground eggs.

Frozen flaxaio.ts is
`6fd81a87dc88822298f5f5557e3f6163a5dadf46480e7f686fc4892001b608a1`,
picking.ts
`c0955fac57da78d375857e4d5e1629ad61b933f3fabbdce9edafa9af4b1124b5`,
spinning.ts
`590278465105338defb4e27289b936a36f6770773cc5732d3ed4caa7a52aaab0`,
banking.ts
`f7b2b65a78a5c57b7abb6a2be9006efd7d2391883829fc7354559e4cf4cd5f64`,
walking.ts
`824e3b76044c95d71daf7064ce05eedd179a77e0c475858cb2fae5b9a311157e`.
Frozen HerbloreSecondaries.ts is
`daa91562ff5993a6b560a7471c62a5c6c69a1f63b5c823c7a2aa3965fec28523`
and HerbloreSecondariesLogic.ts
`716b0502695ce938bcfaca5451feec15111f349e8be9c6e70a6972864e629716`.
Hashes match on both `100adccc` and `8e7d965b`. Selected 274/289 item
ids match on both revisions. Generated game-data JSON has no loc
types; scripts query loc names Flax / Spinning wheel / Ladder / Bank
booth and NPC Betty. Make-menu component IDs are snapshot-posted
buttons, not frozen constants.

FlaxAIO default seed is an empty pack at the field with Crafting 1
and no product. Pick-only is the same without a crafting gate.
Spin-only banks flax 1779 x56 at stand (2725,3493,0), never strings.
Eggs seed lobster food at Edgeville, then tele the dungeon field with
an empty pack. Newt seeds coins at Draynor, then tele Betty with an
empty pack. After Start the serial watch is the named collection or
conversion, then a fresh loaded-bank deposit, closed return to the
actual work tile, and further work. XP is never armed after the
product it must witness.

Name-only, seeded baseline product, stale or closed bank, unclosed
return, XP-only, no flax consumption on spin paths, noted cert ids,
ball of wool 1759, ground-floor flax return on spin-only, shop+ground
mode mix (223 vs 221), missing coin spend on the newt cell, or no
further work fail.

## Adapter audit

- FlaxAIO pick: loc name `Flax` op `Pick`, `Traversal.walkResilient`
  / local walk, `Bank.openBooth` Use-quickly, `Bank.depositInventory`.
- FlaxAIO spin: ladder `Climb-up`/`Climb-down`, wheel `Spin`,
  `ChatDialog.makeX('Flax', fibreCount)`. Count-dialog reader/answer
  is mapped (`78cc99d07`). LIVE still waits Make-X `t_4b04cb5f` if
  world posting fails.
- HerbloreSecondaries eggs: `GroundItems` Take 223, `Bank.openNearest`
  booth, `Bank.depositAllMatching` (keep lobster food). Ground spawns
  cannot be fabricated; missing eggs on this world is a fixture miss.
- HerbloreSecondaries newt: `Shop.open('Betty')`, `Shop.buy` name
  Eye of newt qty min(free,50). `Shop.buy` queues `if-button` only
  when the stock row publishes `component_id`; otherwise it throws
  `notImpl('Shop.buy')`. LIVE waits shop `t_1591d140` if required.
  Do not pretend existing Shop.buy succeeds.
- This card does not implement missing runtime operations.

Panel and host-play keep using `scenario::get` / `names()`. Resource
gnome/coal 6fc633317, Superheater Attack-30 6c7075ce, vial/potion
e4f5352ab, chicken melee 8fd0f138, bank-close generation f33703d5e,
seed-order 1ec25b521, TannerBot 3a34f879f, Falador West vial start
3368 (2f1bd9cf), RuneCrafter/Mule 6750713b8, rune observation
4cc6cc27d, Ardy stall/pickpocket 43749c42f, inventory reader
9cc6adb1f, and station cook/smelt/spin fc5c9a49d are unchanged.

`SCRIPT_GOLD_DEADLINE` stays 180s and `SCRIPT_GOLD_WATCH_TICKS` stays
150. A pick+spin full pack plus wheel plus bank plus return may not
fit that window on LIVE; record the exact partial phase rather than
widen timeouts. Root owns LIVE/native/ledger qualification.

## Verification

Isolated export and target paths are recorded in
`docs/compat/evidence/flax-aio-secondary-fixtures/verification.json`.
No LIVE or fixture process is launched. Root owns the catalog ×
revision cells after review. Source review does not grant live
acceptance.
