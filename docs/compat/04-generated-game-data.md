# Generated game-data assets

This report records the bounded extraction owned by brief 44. The generator reads the pinned engine decoders and packed inputs; it does not start a world, connect to an account, or mutate either server checkout. Runtime Rust is intentionally not changed in this task.

## Outputs

- `crates/api/data/game-data/274.json`: 3,894 ObjType records, 1,222,127 bytes.
- `crates/api/data/game-data/289.json`: 4,089 ObjType records, 1,284,622 bytes.
- `crates/api/data/game-data/manifest.json`: schema and output hashes.

Each asset has schema version 1, the selected revision, engine/content commit IDs, and SHA-256/byte identity for `data/pack/server/obj.dat` and `data/pack/client/config`. The item rows retain `alias`, `id`, display `name`, `cost`, `stackable`, `members`, certificate links/templates, and all three server wear-position fields. Model, sprite, and render data are excluded. Certificate normalization is performed by the server's `ObjType.parse`/`toCertificate` decoder before serialization.

The assets preserve every decoded ID and debug alias. In the selected inputs every decoded record has a unique alias; same display names remain separate rows. Unknown content is not synthesized.

## Provenance

274:

- engine commit `4c95f87efe00b068cadbd229d94736626907bd1a`
- content commit `000c19997e07206131bcb3c884265840efce416d`
- decoder: `src/cache/config/ObjType.ts`
- packed inputs: `data/pack/server/obj.dat`, `data/pack/client/config`

289:

- engine commit `cc359656b4acd216ca452495874b6beba9a0ac75`
- content commit `92649430fcbc83538d8c4367ecb96cee1a67a944`
- decoder: `src/cache/config/ObjType.ts`
- packed inputs: `data/pack/server/obj.dat`, `data/pack/client/config`

The generated JSON records the input hashes; no machine-absolute path or wall-clock timestamp is serialized.

## Reproduce

From the 274bot checkout, with the pinned server engine's installed dependencies:

`/Users/acfrazier/experiments/Server/engine/node_modules/.bin/tsx tools/game-data/generate.ts`

The generator has the two approved checkout roots as explicit read-only inputs. It writes only the three asset files listed above. For schema/fact verification:

`/Users/acfrazier/experiments/Server/engine/node_modules/.bin/tsx tools/game-data/verify.ts`

## Verification evidence

A second complete generation was run and both output hashes were byte-for-byte unchanged. The verifier checks schema/revision, provenance and input hashes, unique IDs/aliases, both Rune platebody and Rune chainbody facts, and distinct same-name `Dragonhide` rows. It observed 3,894/4,089 records and 25 dragonhide-or-platebody custom rows for 274/289 respectively. The detailed machine-readable result is in `evidence/generated-game-data/verification.json`.

Representative source/cache facts include Rune platebody (1127), Rune chainbody (1113), and the distinct `dragonhide_black`/`red`/`blue`/`green` IDs (1747/1749/1751/1753), plus their certificate IDs. These are observations from the selected packs, not hand-maintained constants.

## Subsequent fact-table inventory

These families remain separate extraction work; this generator does not implement them:

- Consumption/healing: `content/scripts/player/configs/consumption/consume.dbtable`, `consume_normal.dbrow`, `consume_effects.dbrow`, `consume_messages.dbrow`, and effect scripts under `content/scripts/player/scripts/consumption/effects/scripts/` (including `kebab.rs2`, `strange_fruit.rs2`, and `consume_effects.rs2`).
- Pickpocket requirements, XP, stun damage/ticks, and pocket outputs: `content/scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable`, `pickpocket.dbrow`, and `content/scripts/skill_thieving/scripts/thieving.rs2`.
- Spell/control families: `content/scripts/skill_magic/configs/magic.dbtable`, `magic_spells.dbrow`, `magic_staff.dbrow`, plus combat spell data at `content/scripts/skill_combat/configs/combat/magic/magic_combat_spells.dbrow`.
- Production families: cooking (`skill_cooking/configs/cooking_source/cooking_generic.dbtable/.dbrow`), smithing (`skill_smithing/configs/smithing/smithing.dbtable/.dbrow`), fletching (`skill_fletching/configs/fletching.dbtable` and arrow/dart/bolt/cut-log/stringing tables), crafting (`skill_crafting/configs/gem/gem.dbtable/.dbrow`, `leather/leather.dbtable/.dbrow`), runecraft (`skill_runecraft/configs/runecraft.dbtable/.dbrow`), mining (`skill_mining/configs/mine.dbtable/.dbrow`, `gem_rock_table.dbrow`), and woodcutting (`skill_woodcutting/configs/trees.dbtable/.dbrow`).

The following are host selection policies, not server fact tables: curated standing spots and fire/cooking/cow locations in `crates/api/src/content.rs`, and common-loot matching in the same file. They must not be presented as generated server facts.
