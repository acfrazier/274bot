# Witch's Potion — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. Map entity IDs are the values in the `.jm2` files, resolved through `pack/*.pack`. Nothing here is taken from a 289 Path where the 377 text, id, or coordinate differs.

## Quest state

- Quest vars: `quests/quest_hetty/configs/quest_hetty.constant:1-3` (`0` not started, `1` started, `2` objects given). Complete is `3` (`scripts/general/configs/quest.constant:6`). Quest points are `1` (`quest.constant:100`).
- The quest-list colour only distinguishes not-started, in-progress, and complete (`scripts/general/scripts/quests.rs2:300`), so stages `1` and `2` need the 377 journal. Its `questlist:hetty` button is ID 7348 (`pack/interface.pack:7234`). `journal:hetty` rules map the started, objects-given, and completed journal text to `hetty:1`, `hetty:2`, and `hetty:3`.
- Completion writes `%hetty = 3`, advances Magic by 3,250 tenths, and sends the scroll (`quest_hetty.rs2:9-13`). The scroll text is `You have completed the\nWitches Potion Quest!` and `325 Magic XP`.

## Start dialogue

- Hetty is NPC 307, placed at `maps/m46_50.jm2:8521` (`0 24 6: 307`), world `(2968,3206,0)`. Handler: `areas/area_rimmington/scripts/hetty.rs2:1-12`.
- First menu (`hetty.rs2:5`) is `I am in search of a quest.` then `I've heard that you are a witch.` The last option does not start the quest (`:8-11`). The Path prefers the first line.
- The quest menu (`hetty.rs2:43`) is `Yes help me become one with my darker side.`, `No I have my principles and honour.`, `What, you mean improve my magic?` The last option does not set the varp. The Path prefers the first line. That branch writes `%hetty = 1` at `:80` and names the four ingredients at `:82`.

## Ingredients

- **Eye of newt:** Betty is NPC 583 at `maps/m47_50.jm2:11894` (`0 4 59: 583`), world `(3012,3259,0)`. `scripts/_unpack/377/all.npc:15356` is `op3=Trade`; `:15360` is `owned_shop,magicshop`; `:15361` sell multiplier 1000. `pack/inv.pack:26` names that shop. `pack/obj.pack:222` maps 221 to `eye_of_newt`; `all.obj:2689` cost is 3, so one eye is 3 coins at that multiplier. There is no 377 ground spawn of `eye_of_newt`. Palace coin spawns at `maps/m50_54.jm2:12022-12023` have no route on `274bot-377-v17` from Lumbridge `(3220,3220,0)` or Draynor bank `(3092,3243,0)`. The Path kills `goblin_unarmed_melee_*` at `maps/m49_50.jm2:6702-6713` until 3 coins (`drop_tables/scripts/goblin.rs2` coin bands).
- **Onion:** LOC 3366 at `maps/m46_50.jm2:5087` (`0 6 51: 3366`), world `(2950,3251,0)`. `all.loc:24808` is `op2=Pick`. `general_use/scripts/pickables.rs2:35-43` produces `onion` and `You pick an onion.`
- **Rat's tail:** NPC 2682 `rat_indoors` at `maps/m46_50.jm2:8516-8520`, stand `(2956,3203,0)`. `drop_tables/scripts/rat.rs2:1-5,15-17` drops `rats_tail` only while `%hetty` is 1 or 2 and none is held or banked. The Path kills only after the start talk.
- **Burnt meat:** cow NPC 81 at `maps/m50_51.jm2:7967`, world `(3252,3288,0)`. `drop_tables/scripts/cow.rs2:11` always drops `raw_beef`. The placed range is LOC 9682 `rimmington_poor_range` at `maps/m46_50.jm2:6174` (`0 26 9: 9682`), world `(2970,3209,0)`, not the generic `range` loc. `all.loc:63141-63147` names it Range, `forceapproach=east`, `category=cooking_oven`. `skill_cooking/scripts/cooking.rs2:1` handles `oplocu,_cooking_oven`. The Path stands east at `(2972,3209,0)`. `cooking_generic.dbrow:41-50` maps raw beef to cooked or burnt meat; the burn line is `You accidentally burn the meat.` `:52-59` burns cooked meat with `successchance,1,1` and `You deliberately burn the nicely cooked meat.`

## Hand-in and drink

- With all four items, Hetty deletes them and sets `%hetty = ^hetty_objects_given` (`areas/area_rimmington/scripts/hetty.rs2:15-24`), then emits `Ok, now drink from the cauldron.` through `~chatnpc` (`:25`). These dialogue lines are not the type-0 chat lines read by Path `message` facts; the supplied traces show the prompt only in dialogue. The durable stage-2 journal text is `I should drink from the cauldron and improve my magic!` (`quests/quest_hetty/scripts/hetty_journal.rs2:40-42`). The hand-in step now settles on `stage_in hetty:2`, which selects a separate cauldron sequence without re-acquiring the consumed ingredients.
- The cauldron is LOC 2024 `hettycauldron` at `maps/m46_50.jm2:6025` (`0 23 5: 2024 10 2`), world `(2967,3205,0)`, shape 10. `all.loc:13953` is `op1=Drink From`. `quest_hetty.rs2:1-4` completes only when `%hetty = 2`. The live account could stand on `(2967,3206,0)` after the hand-in; the Path anchors the drink there.

## Not copied from 289

- 289 targets loc `range` and cites a 225 unpack. 377 places `rimmington_poor_range` (9682) on that tile.
- 289 journal rules quote different not-started wording (`speaking to hetty`). 377 reads `questlist:hetty`: the journal text distinguishes its started and objects-given stages, which quest-tab colour cannot.
- 289 buys raw beef from Wydin. 377 cows drop raw beef (`cow.rs2:11`); no shop stock file for Wydin is in this content tree.
