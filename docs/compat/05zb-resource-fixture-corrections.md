# Resource fixture corrections

This bounded correction changes scenario preparation and catalog qualification only. It does not modify frozen scripts, the host runtime, the client, navigation, support ledgers, or gold clocks. The implementation is commit `2d589c7bacfef32140060e663407b3c364000579` on `codex/rs2b0t-multirevision`, based on `303181613bcad13b6c3d3914921e5e26ce4620d4`.

## HerbloreSecondaries seed bank

`HerbloreSecondaries` now acknowledges its exact lobster seed at the documented Edgeville booth before teleporting to the dungeon. The fixture targets packed loc id 2213 at `(3096, 3493, 0)`; native booth op2 is `Use-quickly` in both selected packs. A repeatable interaction waits for an open, loaded bank containing Lobster 379 x28, so a missing/wrong booth or stale closed bank cannot pass the preparation arm.

The default and `herblore_secondaries_newt` still share their existing dungeon entry, exact ground-item identities, empty-outcome baseline, collection XP, bank deposit, return, and further-collection cycle. No item, route, or witness was broadened.

## Gnome Magic Chopper bounded capacity

`gnome_chop`, `gnome_fletch_short`, and `gnome_fletch_long` now seed exactly Rune axe 1359 x1 and Knife 946 x26. Knife is unstackable in the selected content and the frozen script preserves Knife as a tool; with the Rune axe, that leaves one pack slot for a log or bow. This deliberately qualifies a real chop/bank/return/further-work cycle inside the existing clock rather than claiming ordinary 28-slot throughput.

Scenario regressions require the exact Rune-axe and Knife lower/upper bounds and reject the former Steel axe fixture. The independent catalog baseline and each post-Start milestone require the Rune axe and all 26 Knives to remain held. Chop still requires Magic logs 1513 plus Woodcutting XP, the upstairs deposit, a ground-floor return, and a further log. Fletch still additionally requires exact unstrung Magic shortbow 72 or longbow 70, Fletching XP, the upstairs deposit, return, and further chop. A seeded product, wrong product id, missing tool, reduced ballast, skipped deposit, or missing return cannot qualify.

## FlaxAIO nearestFlax failure facts

The production resource predicate is unchanged. `nearestFlax` remains the frozen script's `Locs` query (`name("Flax")`, `action("Pick")`, within 12 tiles of the field) followed by the thin Reachability adapter with `adjacentOk: true`.

When `FlaxAioPick` fails, the catalog harness now emits diagnostic-only facts for every matching loc in the snapshot. Each row includes packed loc id, layer, tile, name/actions, player/field distance, and the exact native `(reachable, reachable_adj)` pair from `api::query::SceneQuery::flood_reach().at(tile)`. The failure object also records the player tile, field center and radius, whether the player is in field scope, bank open/loaded/session-generation state, reachability availability, and the nearest loc whose native adjacent result is true. The collection has no arbitrary `take` cap. These facts explain the field-to-bank trap without changing reachability, acceptance, retries, deadlines, or witness semantics.

## Frozen identity, clocks, and verification

The selected catalog revisions remain `100adccc037d9f6898080e1cad58fcfc43364775` and `8e7d965be2071d6ec65c3265e12af797082d720a`. The full isolated catalog test revalidated both checked-in frozen-source ledgers.

`SCRIPT_GOLD_DEADLINE` remains 180 seconds and `SCRIPT_GOLD_WATCH_TICKS` remains 150. No LIVE process was launched; the root task retains the catalog x client-revision LIVE matrix and final card acceptance.

The exact commit export, client gitlink, source hashes, full tests, strict Clippy, rustfmt, diff check, and limits are recorded under `docs/compat/evidence/resource-fixture-corrections/`. The isolated scenario suite passed 104 tests. The isolated catalog harness passed 41 tests with its one LIVE test ignored.
