# Runecrafting observation order and catalog failure witness

This bounded correction keeps frozen RuneCrafter/MuleCrafter source, gold
timeouts, and runner `StatXpGain` baseline semantics unchanged. The Rune
scenario armed the crafted-item watch before Runecraft XP. The runner
captures an XP baseline only when a `StatXpGain` watch begins, so a
same-snapshot craft made that baseline late. Native 289/newer Earth on
frozen 6750713b failed `stat_xp_gain(20)>=1` at step 20 after a first 27
Earth craft, deposit/restock, and second altar entry with 27 essence;
that receipt stays immutable. The headless Air pack-empty miss is the
separate 05o uncertainty; this card does not treat it as the same cause
or repair bank/route production behavior.

Shared `rune_craft_variant` now arms XP after ruins arrival and before
observing the selected rune. Essence withdrawal, rune IDs, conversion,
portal exit, fresh deposit, pack-empty, restock, closed bank, return and
further-craft arms are otherwise unchanged. `SCRIPT_GOLD_DEADLINE` and
`SCRIPT_GOLD_WATCH_TICKS` are unchanged. Air 556, Earth 557, essence 1436,
noted 1437, talismans 1438/1440, Runecraft stat 20.

A real `ScenarioRunner` regression feeds a reduced ruins → product/XP
watch sequence with controlled observations where rune output and XP
change together. The original item-then-XP order captures the already
applied craft XP and misses `stat_xp_gain(20)>=1`. The corrected
XP-then-product order keeps the pre-craft baseline and passes. Seed-only
essence at the ruins still cannot satisfy XP. Catalog `CoreWitness` still
requires ordered bank return and further XP; seed-only and incomplete
cycles still fail.

Catalog `RunnerStatus::Failed` now emits the same accumulated
`CoreWitness` JSON the outer-timeout arm already emitted (`error` +
`witness`, including `rune_crafter_cycle` and `latest`). Success,
deadline and lifecycle policy are unchanged. No new tracing system.

Panel and host-play keep using `scenario::get` / `names()`. Superheater
Attack-30 correction 6c7075ce, vial/potion e4f5352ab, chicken melee
8fd0f138, bank-close generation f33703d5e, seed-order 1ec25b521, TannerBot
3a34f879f, Falador West vial start 3368 (2f1bd9cf), RuneCrafter/Mule
6750713b8, and Ardy stall/pickpocket 43749c42f are unchanged.

## Verification

Export: `.superpowers/review-exports/t67dc0fab-rune-obs` via
`docs/compat/evidence/build-isolation/export_git_files.py` (regular Git
blobs of host `d49dccbdb533` plus client `9d090ed049` plus owned overlay).
Isolated empty target:
`.superpowers/review-exports/t67dc0fab-rune-obs-target`
(`isolated_build=true`). Shared campaign target was not used for the
isolated run. Catalog source is not staged. Disk-full required
`cargo clean` of completed caches `t942179cc-runecraft-target` and
`t464d9c4c-ardy-target` before the empty-target compile.

- `cargo test -p scenario --lib` — 93 passed.
- `rune_crafter_cases_register_altar_conversion_and_bank_cycles` — passed.
- `runecraft_xp_watch_before_product_catches_simultaneous_craft` — passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 28 passed, 1 ignored (`LIVE` cell). Frozen-ledger identity needs
  gitignored `.superpowers/inputs` and passed from the isolated export with
  an inputs symlink.
- `rune_crafter_requires_temple_conversion_portal_deposit_restock_and_further_craft`
  — passed.
- `catalog_failure_emits_accumulated_core_witness_and_latest_observation`
  — passed.
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check` on scenario and the catalog harness — clean.

No LIVE or fixture process was launched. Root owns repeating failed Earth
cells on a fresh reviewed isolated binary. Source review does not grant
live acceptance. No PASS claim here.
