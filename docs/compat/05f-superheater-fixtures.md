# Superheater production and staff-branch fixtures

This bounded extension keeps both frozen Superheater cards and script source
unchanged. Three shared scenario names:

- `superheater`: default `bar=Bronze`, Staff of fire. Both catalogs.
- `superheater_steel`: `bar=Steel` (1 iron + 2 coal). Both catalogs.
- `superheater_fire_battlestaff`: Bronze with Fire battlestaff 1393 and no
  Staff of fire 1387. Newer catalog `8e7d965b` only. Old catalog `100adccc`
  still requires Staff of fire; that cell is refused, not weakened.

Frozen Superheater.ts/Logic hashes match remaining-production section 11:
`100adccc` `9188208c0351` / `41b9bf85fc0b`; `8e7d965b` `4c4ea711e2f7` /
`74df1dfcc8bc`. Selected 274/289 items: copper 436, tin 438, iron 440, coal
453, nature 561, staff of fire 1387, fire battlestaff 1393, bronze bar 2349,
iron bar 2351, steel bar 2353. Fire battlestaff is a posted fire provider in
both staff tables. Superheat Item is not a SPELL_DB row; source
`MAGIC_REQUIRED` is 43. Exact Superheat XP is not in generated spell facts,
so witnesses require Magic and Smithing XP after Start rather than an
invented XP constant.

Remaining-production named Varrock East. Superheater uses `nearestBank`, so
these cells reuse the accepted Varrock West booth (3185,3440,0) and
Use-quickly helper. Seed before Start: empty pack, Magic 43, Smithing 1 or
30, banked staff/natures/ores, no bars, staff not worn. Script must withdraw
and equip. Independent catalog witness needs exact bar id, primary and
secondary ore consumption, nature consumption, both XP, the equipped staff
id, deposit of script-created bars while keeping natures, same-generation
ore restock (steel coal == 2 × iron), closed bank, then further bars and XP.
Queued cast, XP-only, name-only, seeded bars, partial iron bars, or the
wrong staff fail. Diagnostics: missing Start baseline is fixture
preflight; post-Start qualify failure is product/capability.

Panel and host-play keep using `scenario::get` / `names()`.

## Verification

Implementation baseline: host `6401d3a4a0fd983d90691a472c23000845bdea4b`,
client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`, plus these owned files.
Frozen export: `/Users/acfrazier/experiments/274bot/.worktrees/t_10f975b7-src-6401d3a4`
(git archive of that host + client archive + owned overlay; frozen catalog
inputs linked read-only). Isolated empty target:
`/Users/acfrazier/experiments/274bot/.worktrees/t_10f975b7-target-6401d3a4`
(`isolated_build=true`). Shared campaign target was not used.

- `cargo test -p scenario` — 86 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 20 passed, 1 ignored (`LIVE` cell).
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance.

## Alternative staff fixture correction

Root correction 6c7075ce seeds Attack 30 only for Fire battlestaff, adds an
acknowledged native stat proof before Start, and refuses an under-level
Start baseline. The existing bank-cycle witness test now rejects the missing
Attack requirement, then accepts a properly seeded baseline; its exact
isolated binary passes that focused test. No gameplay policy or foreign
script changed. Both earlier wield failures remain under the 6d750e65 run.

Root built native/headless 6c7075ce in a new empty target, verifying all 2,240
source files and binary hashes. This composed candidate also includes canStep
5612b265 and DirectNavigator 5ce85959. DirectNavigator review 1221 passed;
canStep review remains the gate before new LIVE launches on this candidate.
