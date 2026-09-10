# Alcher option fixtures

This fixture extension keeps the frozen `Alcher` card and production script
source unchanged. Three stable scenario names map to that same card:

- `alcher_custom`: selects `custom`, names `rune_chainbody`, uses `alchs=1`,
  seeds two chainbodies and four Nature runes before Start.
- `alcher_ordered`: selects `rune_platebody` then `rune_chainbody`, uses
  `alchs=1`, and seeds one of each plus four Nature runes before Start. The
  witness requires the platebody stack to be exhausted while chainbody stock
  is observed, then requires chainbody consumption and post-Start magic XP.
- `alcher_large_batch`: selects `rune_chainbody`, uses `alchs=1000`, and
  seeds 1000 chainbodies and 1000 Nature runes before Start. The scenario waits
  for the 1000-note stack before its XP watch; the witness requires observed
  1000-note and 1000-rune peaks followed by consumption and XP/coin deltas.

All variants retain the existing Magic 55, Varrock West, 180-second deadline,
600ms tick and 150-tick watch settings. No preparation action is sent after
Start. The headless harness accepts each variant through `CATALOG_SCENARIO`,
uses `Alcher` for the frozen catalog card lookup, and records the variant name,
merged settings, source/catalog identity and independent observation deltas in
its existing JSON evidence path.

Focused verification:

- `cargo test -p scenario --lib` — 79 passed.
- `cargo test -p host-play --test catalog_boundary_live --features memory-profile` — 9 passed, 1 ignored (LIVE cell).
- `git diff --check` — passed.

LIVE execution for both frozen catalogs remains owned by the root task.
