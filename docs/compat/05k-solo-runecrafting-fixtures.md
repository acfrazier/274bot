# Solo RuneCrafter and MuleCrafter bank-cycle fixtures

This bounded extension keeps both frozen RuneCrafter and MuleCrafter cards
and script source unchanged. Three shared scenario names:

- `rune_crafter`: default `rune='Air runes'`, `mode='Solo'`. Both catalogs.
  Air talisman 1438, essence 1436, Air rune 556. Ruins (2988,3294,0),
  Falador East bank (3013,3355,0).
- `rune_crafter_earth`: `rune='Earth runes'`, `mode='Solo'`. Both catalogs.
  Earth talisman 1440, Earth rune 557. Ruins (3303,3477,0), Varrock East
  bank (3253,3420,0). Runecraft 9.
- `mule_crafter`: default `rune='Air rune'` (singular), `mode='Crafter'`,
  `partner=''`, `bankFill=true`. Both catalogs. Same Air ids as
  `rune_crafter`, but this card's ruins are (2983,3288,0). Blank partner is
  this card's actual solo behavior; do not treat the two cards as aliases.

Frozen RuneCrafter.ts hashes match remaining-production section 2:
`a944049b42ee5adf882aaf119fb7b7f4604b63e2d07ff34e2db6dbf9e3954843`
on both `100adccc` and `8e7d965b`. Frozen MuleCrafter.ts hashes match
section 17:
`bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9`
on both catalogs. `runeCraftLocations.ts` is
`40db500c2c081a0978738796fcce62965bd39ac70d0995345e15443d4042f221`
on both. Selected 274/289 items: `blankrune` 1436 (unnoted Rune essence),
`cert_blankrune` 1437 (noted, rejected), `air_talisman` 1438,
`earth_talisman` 1440, `airrune` 556, `earthrune` 557. Runecraft stat id
20. Temple interiors sit at world z > 4000; the overworld is z ≤ 4000.
This card does not add runtime code. RuneCrafter Runner/Mule Recipient and
MuleCrafter `mode='Mule'` / named-partner trade remain pending.

Each core seed is an empty pack, 200 banked unnoted essence and one
selected talisman, no crafted runes, no noted essence, and tele to the
selected bank. The script must withdraw unnoted essence, use the talisman
on the selected Mysterious ruins, convert essence to the selected rune
with Runecraft XP inside the altar (z > 4000), take the portal back to
those ruins, deposit those script-created runes into a fresh bank
generation, empty the pack of runes (`ItemIdAtMost` product at 0), restock
essence, close (IF_CLOSE advances generation 1→2), return to the ruins and
convert again. The pack-empty watch sits between bank runes and further
runes so the first craft cannot satisfy the terminal arm. Queued
Craft-rune, temple readiness without conversion, XP-only, name-only,
seeded runes, noted 1437, wrong rune, stale/closed bank, absent portal, or
no further craft fail. Original `SCRIPT_GOLD_DEADLINE` is unchanged.

Panel and host-play keep using `scenario::get` / `names()`. Superheater
Attack-30 correction 6c7075ce, vial/potion e4f5352ab, chicken melee
8fd0f138, bank-close generation f33703d5e, seed-order 1ec25b521, TannerBot
3a34f879f, and Falador West vial start 3368 (2f1bd9cf) are unchanged.

## Verification

Export: `.superpowers/review-exports/t942179cc-runecraft` via
`docs/compat/evidence/build-isolation/export_git_files.py` (regular Git
blobs of host `f79a4aa2cdd8` plus client `9d090ed049` plus owned overlay).
Isolated empty target:
`.superpowers/review-exports/t942179cc-runecraft-target`
(`isolated_build=true`). Shared campaign target was not used. Catalog
source is not staged.

- `cargo test -p scenario` — 91 passed.
- `rune_crafter_cases_register_altar_conversion_and_bank_cycles` — passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 24 passed, 1 ignored (`LIVE` cell). Frozen-ledger identity needs
  gitignored `.superpowers/inputs` and passed from the campaign checkout.
- `rune_crafter_requires_temple_conversion_portal_deposit_restock_and_further_craft`
  — passed.
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` and catalog harness — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance.
Trade/paired modes stay explicit later scope.
