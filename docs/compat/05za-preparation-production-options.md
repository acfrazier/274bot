# Preparation and remaining production option fixtures

This bounded fixture correction keeps the frozen catalog scripts and host runtime
unchanged. It moves hostile-field preparation acknowledgements ahead of the
field teleport, establishes the documented CoalTrucks combat prerequisite, and
adds the three distinct production scenarios selected by the 06z audit. The
implementation is commit `0c4f180276ca3eb2721bfb6d19e70f252afc14af` on
`codex/rs2b0t-multirevision`, based on
`c6d45f42a983c1b4209e29642b00c9803a1d7aaf`.

## Safe preparation before hostile fields

`combat_core_scenario` now seeds and acknowledges Attack 40, Strength 40,
Hitpoints 40, the selected weapon, food, extra equipment, Thieving when needed,
and empty loot outcomes on the safe initial tile. Only after every applicable
precondition passes does it teleport into the hostile field and then Start the
catalog card. This applies to `chaos_druid`, `moss_giant`, `hill_giant`,
`auto_fighter`, `rock_crab`, `green_dragon`, `fire_giant`, and `ardy_fighter`.
For ChaosDruid this means a post-tele Weaken cannot invalidate the Attack 40
precondition before Start.

The scenario regression locates the first field-arrival arm, requires the
Attack/Strength/Hitpoints acknowledgements to precede it for every shared
fighter, and rejects any stat/item precondition arm left between arrival and
Start. Existing fields, levels, equipment, loot witnesses, and
`Proof::Stat` effective-level semantics are unchanged.

## CoalTrucks combat prerequisite

The old fixture entered the mine at combat level 1 and both observed revisions
died to the level-27 giant bats. CoalTrucks itself documents that those bats are
aggressive below combat level 55. The corrected scenario therefore prepares and
acknowledges ordinary Attack, Strength, Defence, and Hitpoints level 48, Mining
30, steel pickaxe 1269, and empty coal outcomes before teleporting into the bat
mine. With ordinary Prayer/Ranged/Magic defaults, those four melee/HP levels
produce native combat level 55 without Attack 99, invulnerability, or a
post-Start cheat.

The independent catalog observation reads `local_player.player.combat_level`.
Its CoalTrucks Start baseline requires Mining 30, steel pickaxe 1269, no raw or
noted coal, the mine tile, and native combat level at least 55. The regression
rejects 54 and accepts 55.

The existing bounded core still witnesses real Mining XP, Coal 453, arrival at
the mine truck, an empty pack after truck deposit, and further Coal 453. Filling
the truck to 120, hauling to Seers, banking, and returning cannot fit the
unchanged gold clock from an empty truck and has no deterministic truck-content
seed. This fixture does not claim that the partial mine-truck cycle is full-haul
card acceptance.

## Three distinct production scenarios

### `alcher_defaults`

The scenario injects `items=[]` and `alchs=1`, exercising the frozen catalog's
`DEFAULT_ALCH_ITEMS` fallback rather than a custom or named chip. Before Start it
sets Magic 55 and seeds only unnoted Yew longbow 855 x1, Nature rune 561 x1, and
Staff of fire 1387 x1 in the Varrock West bank. It acknowledges an open, loaded
bank, no noted Yew longbow 856, and no Rune chainbody, then closes the bank.

After Start the ordered witness requires noted Yew longbow 856 x1 withdrawal,
65 Magic XP, consumption of the noted target and Nature rune, exactly 768 Coins
995, and no Rune chainbody at any point. The catalog-side `AlcherDefaults`
witness independently requires a new bank generation for the noted withdrawal
and the same exact consumption, XP, and coin outcome.

### `bank_fletcher_shafts`

The scenario injects `material=Logs` and `product=Arrow shafts` with no explicit
mode. It starts with Knife 946, Logs 1511 x27 in the pack, Logs x54 in bank,
Fletching 1, and no Arrow shafts 52. The first production arm requires exact
Arrow shafts 52 x405, Logs consumption, and Fletching XP. It then requires a
fresh open/loaded-bank deposit, exact Logs x27 restock with the corresponding
bank decrease, a closed bank, fresh post-restock XP, and further shafts.

### `bank_fletcher_headless`

The scenario injects `material=Logs` and `product=Headless arrows`; the selected
product chooses the catalog's attach path, where material/knife are ignored. It
starts with Feathers 314 x30 and Arrow shafts 52 x30 in the pack, x60 of each in
bank, Fletching 1, and no Headless arrows 53. The first production arm requires
Headless arrows 53 x30, both inputs consumed, and Fletching XP. It then requires
the same exact deposit, same-generation restock, bank-close, fresh XP, and
further-product sequence using both inputs.

## Independent witnesses and named false positives

The catalog harness has separate `AlcherDefaults` and
`BankFletcherOptionCycle` states; it does not infer success from scenario step
completion. Its exact-ID and ordering checks reject:

- an Alcher seed/baseline alone or a noted withdrawal without the cast outcome;
- raw Yew longbow 855 substituted for noted id 856;
- a Rune chainbody entering the empty-items fallback path;
- a pre-seeded shaft/headless product at Start;
- a BankFletcher baseline or first product batch without deposit/restock/further
  production;
- deposit and restock observations that are not open, loaded, fresh, and in the
  required bank generation;
- further product without bank closure, both applicable inputs decreasing, and
  fresh Fletching XP;
- CoalTrucks native combat level 54; level 55 is the minimum accepted baseline;
- hostile-field preparation acknowledged only after arrival.

## Frozen source identities

The two frozen catalogs are
`100adccc037d9f6898080e1cad58fcfc43364775` and
`8e7d965be2071d6ec65c3265e12af797082d720a`.

- `Alcher.ts` is byte-identical: `1a09baa1117544a2bb80da145829e69c9ced029ad5af533616afa8e7a36f2032`.
- `CoalTrucks.ts` is byte-identical: `d20112f0bfb44612ae53e0c56009c6b3e50808375c2a54dc43ed2160e55fcf36`.
- Old-catalog `BankFletcher.ts`: `194eb82dd3613cf2fea77a5cb04fba1dadbacad78efa6e0eee16c1f4a1c3f3ee`.
- Newer-catalog `BankFletcher.ts`: `48a9f13b774931cd2ea5298cf625c480f071713d459ade293c1caccfb383aa2d`.

The selected 274/289 item IDs used by these fixtures are exact native-content
rows. Frozen catalog source was not edited.

## Gold clock, verification, and limits

`SCRIPT_GOLD_DEADLINE` remains 180 seconds and
`SCRIPT_GOLD_WATCH_TICKS` remains 150. No LIVE process was launched. No product,
client, engine, navigation, runtime, ledger, support-matrix, or STATE file was
changed by this task.

The exact export, source hashes, client gitlink, focused and full test results,
strict Clippy result, formatting check, and false-positive inventory are under
`docs/compat/evidence/preparation-production-options/`. The isolated scenario
suite passed 104 tests. The isolated catalog harness passed 40 tests with its
one LIVE test ignored. Root retains the catalog x client-revision LIVE matrix
and final card acceptance.
