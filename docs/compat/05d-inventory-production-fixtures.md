# Inventory production catalog fixtures

This bounded extension keeps the frozen DartFletcher, HerbCleaner and GemCutter
cards and production script source unchanged. Six shared scenario names apply
to both frozen catalogs on revisions 274 and 289:

- `dart_fletcher`: `tier=Bronze`.
- `dart_fletcher_iron`: `tier=Iron`.
- `herb_cleaner`: default `herbs=[]`.
- `herb_cleaner_named`: `herbs=['Guam leaf']`.
- `gem_cutter`: default `gems=[]`.
- `gem_cutter_named`: `gems=['Sapphire']`.

Generated rows in both revisions: bronze_dart_tip 819 / bronze_dart 806,
iron_dart_tip 820 / iron_dart 807, feather 314; unidentified_guam 199 (display
Herb) / guam_leaf 249; unidentified_marentill 201 (display Herb) / marentill
251 (display Marrentill); uncut_sapphire 1623 / sapphire 1607; uncut_opal 1625;
chisel 1755; crushed_gemstone 1633. Unidentified herbs share the display name
Herb, so proofs use exact IDs.

Dart cases seed 100 tips and 100 feathers in pack, no product, Fletching 1 or
22, Lumbridge courtyard. There is no bank cycle. The independent witness needs
two observations: exact product id >= 10 with both input IDs down and Fletching
XP, then a later observation with more product, more input consumption and more
XP. Seeded output, one observation, XP-only, name-only, wrong-tier output, or
missing tip consumption fail.

Herb and gem cases start empty-pack at the accepted Varrock West booth
(3185,3440,0) so seed acknowledgements reuse the proven Use-quickly helper.
Remaining-production names Varrock East (3253,3420,0); the scripts use
nearestBank, so West is the same first-family booth path. Herb seeds banked
unidentified guam 199 x30 (named also 201 x4) and Herblore 3 or 5. Gem seeds
banked chisel 1755 x1 and uncut sapphire 1623 x28 (named also uncut opal 1625
x4) and Crafting 20. No outputs are seeded.

The herb/gem witness requires a full identified/cut pack, those script-created
outputs in a later loaded bank generation, restock of the exact input ID from
pre-Start bank stock in that same generation, closed bank, then another product
with further skill XP. Named cases keep the filtered stack (201 or 1625) in
bank and out of pack. Gem cases keep the chisel in pack and crushed 1633 at 0.

Panel and host-play keep using `scenario::get` / `names()`, so native
catalog_watch can select the new names.

## Verification

Implementation baseline: host `62d5b524adffb5dfe772dc574c1b2076c6e87fad`,
client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`. Harness checks ran from an
export of that committed host plus these owned files, because uncommitted
script-crate edits in the campaign worktree do not compile host-play.

- `cargo test -p scenario` — 84 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 16 passed, 1 ignored (`LIVE` cell).
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` — clean.

No LIVE or fixture process was launched. Root owns the twenty-four catalog x
revision cells after review.
