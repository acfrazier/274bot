# Generated consumption and pickpocket facts

This report records the extension of the brief-44 game-data pipeline. The generator reads the pinned engine ObjType/NpcType decoders and the selected content DB table/row files. It does not start the server, connect to an account, mutate either checkout, or add Rust/runtime policy.

## Outputs

- `crates/api/data/game-data/274.json`: schema 3, 3,894 item records, 250 consumption records, 12 pickpocket groups.
- `crates/api/data/game-data/289.json`: schema 3, 4,089 item records, 250 consumption records, 12 pickpocket groups.
- `crates/api/data/game-data/manifest.json`: revisioned output sizes and SHA-256 identities.
- `docs/compat/evidence/generated-game-data/verification.json`: machine-readable verification receipt.

Existing item rows are retained. New rows preserve source order and repeated entries. Each consumption fact is one consumption action keyed by item alias and ID, with separate `stat_change`, `stat_heal`, and `heal_energy` arrays. Tuple units are the raw server units: `base` is the integer base amount and `percent` is the integer percent component; no variable effect is flattened to a maximum. Each pickpocket group retains every NPC alias/ID/name, level, raw experience, stun ticks/damage, numerator/denominator success tuple, pocket text, and every loot row with item alias/ID/name, minimum, maximum, and weight.

## Qualification

A fact is labelled `fixed_hp_heal` only when it comes from `consume_normal.dbrow`, has no stat changes or energy effect, has exactly one hitpoints heal tuple, has zero percent, and has a positive base amount. Everything else is explicitly `not_fixed_hp_heal`; this includes conditional/stat-changing potion and food effects and zero/unfinished entries. The consumer must use the qualified view rather than infer fixed healing from item names. Multi-bite foods remain one row/action per source consumable alias; the generator does not invent a whole-item total.

The selected server facts are the same in both revisions for the required examples: Lobster=12, Bread=4, and Anchovies=3 fixed HP healing. The Guard group joins `guard1`, `guard2`, and `ardougne_guard` and requires Thieving level 40. Repeated aliases and loot rows are retained rather than deduplicated; NPC and item IDs are resolved from the selected revision's decoders, so cross-revision identity is never assumed.

## Provenance and reproduction

Each payload records the pinned engine/content commit IDs, SHA-256/byte identities for packed item/NPC/config inputs, all local decoder imports (including ObjType and NpcType), the consumption/pickpocket DB files, and `consume_effects.rs2`, plus the selected cache identity. The dirty-input gate checks these exact paths in both checkouts. No absolute path or timestamp is serialized.

From this checkout, using the pinned engine dependencies:

`GAME_DATA_274_ENGINE=/path/to/engine GAME_DATA_274_CONTENT=/path/to/content GAME_DATA_289_ENGINE=/path/to/engine GAME_DATA_289_CONTENT=/path/to/content /path/to/tsx tools/game-data/generate.ts`

Verify the live source pins, dirty gates, all provenance hashes, cache identities, unique item IDs/aliases, NPC/item joins, required facts, and output hashes with:

`/path/to/tsx tools/game-data/verify.ts`

A complete generation was run twice after the schema extension. The three generated files were byte-identical on the second run. Final output SHA-256 values were:

- 274: `c83042956ea6896e9fd5de9927556399fdc8c2d10732b0e59c98cfabd3a5d6a2`
- 289: `63289f8b2b6a0d6094d11bb9ec3b651a3b78acdfe45ea258b04bbe2e00078276`
- manifest: `077dc252ec496b4160215aeb044f2a2961fb69add5f57187875c720748916697`

Verification exited 0 for both revisions and emitted the receipt at `docs/compat/evidence/generated-game-data/verification.json`.
