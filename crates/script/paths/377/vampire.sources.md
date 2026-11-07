# Vampire Slayer — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`.

## Quest state

- Varp `vampire`: 0 not started, 1 started, 2 spoke to Harlow, 3 complete. Quest points 3.
- The admitted native button and `quests/quest_vampire/scripts/vampire_journal.rs2:7-55` recover stages 0, 1, 2 and 3. Harlow's first talk settles on the durable stage-2 line `I have spoken to Dr Harlow`, not his transient dialogue.
- Completion: `stat_advance(attack, 48250)` and the scroll `You have completed the\nVampire Slayer Quest!` / `4,825 Attack XP` (`quest_vampire.rs2:41-45`).

## Start and stake

- Morgan NPC 755 at `maps/m48_51.jm2:7323`, world `(3098,3268,0)`.
- `areas/area_draynor/scripts/morgan.rs2:16-18`: last option `Have you got any tips` does not start. Prefer `Ok, I'm up for an adventure.` The player echo adds an o (`Okay`); the menu text does not.
- Harlow NPC 756 at `maps/m50_53.jm2:12938`, world `(3222,3397,0)`.
- First talk (`harlow.rs2:11-22`) prefer `Morgan needs your help` (last is also that line). It sets varp 2 and does not take a beer or give the stake.
- Second talk (`harlow.rs2:25-50`) consumes one beer and adds `stake` unless one is already held. Prefer `Ok mate.` so the stake-held menu does not pick `No, you've had enough.`
- Beer: bartender NPC 734 `(3045,3257,0)` `maps/m47_50.jm2:11918`. `bartender.rs2:39-40` sells one beer for 2 coins. The first option is `Could I buy a beer please?`; the last option does not buy. Coins: OBJ 995 qty 4 at `maps/m50_54.jm2:12023` `(3232,3500,0)`.
- The repeated ground pickup targets that natural coin spawn; the nav route uses the source-derived `ladder_1755` return into the palace alcove. No goblin-coin workaround or item seed bypasses that route.

## Tools and the crypt

- Stairs LOC 1722 at `maps/m48_51.jm2:5291` `(3099,3266,0)`, `op1=Climb-up`, `forceapproach=south` (`stairs.loc:131-138`). `ladders+stairs/scripts/stairs.rs2:7` lands at `(3102,3266,1)`. As in the existing 289 Path, a walk targets that landing so navigation owns the transport and arrival. The authored sequence's skip guard includes the same landing: the old cupboard-centred radius 4 excluded it (distance 6), so the successfully completed climb was selected again and tried to return downstairs.
- Garlic cupboard LOC 2612 `garliccupboardshut` at `maps/m48_51.jm2:7074` `(3096,3269,1)`, `op1=Open`, `forceapproach=east`. Open is a `loc_change` to `garliccupboardopen` (`cupboards.rs2:34`), not a varp morph. That observed open loc both settles and skips the open step, matching the existing 289 recipe. Search the open id (`cupboards.rs2:18,24,71-76`) for `garlic`.
- Hammer OBJ 2347 at `maps/m45_51.jm2:7018` `(2934,3286,0)`.
- Crypt stairs LOC 2616 `cryptstairsdown` at `maps/m48_52.jm2:8258` `(3115,3357,0)`. `stairs.loc:415-420` offers `Walk-Down` (not Climb-down). `stairs.rs2:366-370` teleports to `0_48_152_5_43` `(3077,9771,0)`. The Path walks to that landing through the priced transport rather than owning a second explicit click.
- Coffin LOC 2614 at `maps/m48_152.jm2:6754` `(3077,9775,0)`, `op1=Open`. `quest_vampire.rs2:1,9-27` spawns `count_draynor` at `0_48_152_6_46` `(3078,9774,0)`. There is no placed count; the open step settles on `npc_present`.
- Kill requires `stake` and `hammer` held (`count_draynor.rs2:57-73`). The stake is consumed. Garlic is optional damage reduction, still collected.

## Not copied from 289

- Crypt op is `Walk-Down`, not a 289 Climb-down name.
- The count is a spawn, not a placed morph id.
