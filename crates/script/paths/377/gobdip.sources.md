# Goblin Diplomacy — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. Entity IDs are the `.jm2` values resolved through `pack/*.pack`. Coordinates that differ from 289 are taken from these 377 maps, not from `paths/289/gobdip.json`.

## Quest state

- `quests/quest_gobdip/configs/quest_gobdip.constant`: not started `0`, complete `6`. Quest points `5`.
- The native quest-list button is admitted through `quest_journal/native_377.rs`. `quests/quest_gobdip/scripts/gobdip_journal.rs2:7-56` distinguishes stages 0–6: investigate, bring orange, gave orange, gave blue, gave brown, complete. Later pages retain earlier history, so the progress rules match the latest milestone first.
- Completion: `%goblinquest = 6`, Crafting `stat_advance` 2000 tenths, one `gold_bar` (`quest_gobdip.rs2:61-65`). Scroll: `You have completed the\nGoblin Diplomacy Quest!` and `200 Crafting XP|A Gold Bar`.

## Start

- Bartender NPC 734 at `maps/m47_50.jm2:11918`, world `(3045,3257,0)`.
- `areas/area_port_sarim/scripts/bartender.rs2:9-10`: last option does not start when the barcrawl is absent. Prefer `Not very busy in here today, is it?`
- That branch sets the quest started (`bartender.rs2` goblin-diplomacy start). Every stage-changing talk rereads the native journal; NPC dialogue is not a game-chat `message` receipt.

## Armour and dyes

- Generals: wartface NPC 297 `(2957,3511,0)` and bentnoze NPC 296 `(2958,3511,0)`, `maps/m46_54.jm2:6212,6214`.
- `quest_gobdip.rs2` stage-1 last option `Do you want me to pick an armour colour for you?` sets stage 2 without taking armour. Prefer it. The offer step is not skipped just because orange armour is already held.
- Hand-ins consume orange, then blue, then brown. Stage 5 queues complete.
- The outdoor unarmed melee IDs 1769–1775 at `maps/m49_50.jm2:6702-6713` map to `goblin_unarmed_melee_1` through `_7` (`pack/npc.pack:1770-1776`). Their common drop table yields `goblin_armour` for the five roll values 34–38 (`drop_tables/scripts/goblin.rs2:6-12,73-76`). The Path uses the same bounded outdoor encounter as the live-qualified 377 Hetty coin farm, not the village's red/green goblins spread across divided buildings.
- Aggie NPC 922 `(3086,3259,0)`. `aggie.rs2:29` last option `Can you make dyes for me please?` is the dye menu. The dye submenu last option and the multi3 last option `I can do without dye` do not buy. Prefer `What do you need to make red/yellow/blue dye?` and `Okay, make me some red/yellow/blue dye please.`
- Costs: 5 coins + 3 redberries / 2 onions / 2 woad (`aggie.rs2:143-151,186-196`).
- Dye application is armour used on the dye (`goblin_mail.rs2:1-12`), message `You dye the goblin armour <colour>.` Orange is red dye used on yellow dye.
- Wyson NPC 36 `(3027,3379,0)` `maps/m47_52.jm2:12428`. First menu last `Not right now` does not sell. Prefer `I'm looking for woad leaves.` The price menu last `How about 20 coins?` gives 2 leaves (`wyson_the_gardener.rs2:1-35`).
- Coins: OBJ 995 qty 3 at `(3228,3504,0)` and qty 4 at `(3232,3500,0)`, `maps/m50_54.jm2:12022-12023`. The Path collects 35 through repeated natural pickups. The enclosed alcove is reached through the sewer ladder at `(3230,9904,0)`, `maps/m50_154.jm2:7787`, `ladders.rs2:90-97`; it is not a missing palace gate.
- Redberries OBJ 1951 at `maps/m51_52.jm2:6650,6651,6654`, anchor `(3271,3366,0)`.
- Onions: `maps/m46_50.jm2:5084-5089`, anchor `(2950,3251,0)`, `pickables.rs2:35-43`.
- Mail farm stand `(3146,3232,0)`, bounded to `(3138,3224)`–`(3152,3264)`, from `maps/m49_50.jm2:6702-6713`. The previous unbounded village encounter stalled at `(2953,3507,0)` with repeated `I can't reach that!` and only one mail. The open-melee tactic sends an NPC Attack, rather than opening intervening doors (`crates/script/src/combat/machine.rs:2321-2346,3177-3205`); an outdoor source-bounded encounter avoids selecting across that geometry without changing shared combat behavior.
- Mail looting uses the explicit inventory threshold 3. A bare loot alias means "take only while holding none" (`families/combat.rs:197-216,711-716,1775-1785`); the separate `until` predicate does not raise that threshold. The historical one-hour combat-49 witness therefore kept killing after its first mail without collecting another. Incidental goblin coins retain their one-pile policy; the separate natural palace recipe funds any remaining dye cost.
- These ingredient goals have `acquire: null` because the stage-1 authored sequence owns their acquisition and transformation order. Native preparation defers such goals to the authored steps (`quester/provision.rs`, `acquirable_without_recipe_defers_to_authored_steps_after_bank_observation`): gather the 35 palace coins before buying woad or dyes, and do not replenish consumed raw ingredients before every later step. Binding all four goals to automatic preparation instead intercepted `scan-bank`, bought woad before the coin recipe, and timed out with only 5 incidental goblin coins in the counted-mail witness.
- The stage-1 funding boundary tracks the remaining purchases: 35 before dyes, 25 with orange dye (20 for two woad + 5 for blue), 5 with orange dye and two woad, and none with both dyes. Skip and settle use the same observed inventory predicate. Requiring 35 again after the 10-coin orange recipe restarted the coin trip instead of proceeding to Wyson; the authored-order witness exposed this with orange dye and 25 coins. Insufficient remaining funds still select the natural coin recipe.

## Not copied from 289

- Journal rules are grounded in the 377 journal source above. Colour alone does not separate the armour stages.
- Dye `use_on` order is armour onto dye so `goblin_mail.rs2` matches. Orange dye is mixed in 377; it is not a shop item.
