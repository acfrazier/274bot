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
- Goblins 100/101 and the unarmed melee ids in `maps/m49_50.jm2` drop `goblin_armour` (`drop_tables/scripts/goblin.rs2`). The Path farms the placed ids, not a 289 morph name.
- Aggie NPC 922 `(3086,3259,0)`. `aggie.rs2:29` last option `Can you make dyes for me please?` is the dye menu. The dye submenu last option and the multi3 last option `I can do without dye` do not buy. Prefer `What do you need to make red/yellow/blue dye?` and `Okay, make me some red/yellow/blue dye please.`
- Costs: 5 coins + 3 redberries / 2 onions / 2 woad (`aggie.rs2:143-151,186-196`).
- Dye application is armour used on the dye (`goblin_mail.rs2:1-12`), message `You dye the goblin armour <colour>.` Orange is red dye used on yellow dye.
- Wyson NPC 36 `(3027,3379,0)` `maps/m47_52.jm2:12428`. First menu last `Not right now` does not sell. Prefer `I'm looking for woad leaves.` The price menu last `How about 20 coins?` gives 2 leaves (`wyson_the_gardener.rs2:1-35`).
- Coins: OBJ 995 qty 3 at `(3228,3504,0)` and qty 4 at `(3232,3500,0)`, `maps/m50_54.jm2:12022-12023`. The Path collects 35 through repeated natural pickups. The enclosed alcove is reached through the sewer ladder at `(3230,9904,0)`, `maps/m50_154.jm2:7787`, `ladders.rs2:90-97`; it is not a missing palace gate.
- Redberries OBJ 1951 at `maps/m51_52.jm2:6650,6651,6654`, anchor `(3271,3366,0)`.
- Onions: `maps/m46_50.jm2:5084-5089`, anchor `(2950,3251,0)`, `pickables.rs2:35-43`.
- Mail farm stand `(2958,3507,0)` from `maps/m46_54.jm2:6196-6218`.

## Not copied from 289

- Journal rules are grounded in the 377 journal source above. Colour alone does not separate the armour stages.
- Dye `use_on` order is armour onto dye so `goblin_mail.rs2` matches. Orange dye is mixed in 377; it is not a shop item.
