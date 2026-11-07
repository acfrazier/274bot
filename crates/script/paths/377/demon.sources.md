# Demon Slayer — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. 289 coordinates are not reused where the 377 map or loc name differs.

## Quest state

- Varp `demonstart`: 0 not started, 1 talked to Aris, 2 key hunt, 3–27 bones, 28 Traiborn's key, 29 Silverlight, 30 complete (`demon.constant:1-7`). Quest points 3.
- The admitted native button and `quests/quest_demon/scripts/demon_journal.rs2:5-56` recover 0, 1, key hunt 2, Silverlight 29 and complete 30. The journal deliberately does not expose the bone counter (`:36-45`); stages 2–28 remain the key-hunt sequence and inventory receipts distinguish the acquired keys.
- Completion scroll: `You have completed the\nDemon Slayer Quest!` / `Silverlight` (`demon_slayer.rs2:4-6`). No skill XP.

## Start

- Aris NPC 882 at `maps/m50_53.jm2:12928` `(3203,3424,0)`.
- `gypsy.rs2:79`: last option `No, I don't believe in that stuff.` does not start. Prefer `Ok, here you go.` One coin is deleted (`:151-154`). The Path naturally picks up OBJ 995 qty 4 at `(3232,3500,0)`, `maps/m50_54.jm2:12023`, through the source-derived sewer-ladder route into the enclosed alcove.
- `gypsy.rs2:200-214`: last option is the Wally joke and does not set the varp. Prefer `Okay, where is he? I'll kill him for you!` The varp is set at `:238`, before the closing menu. Prefer `Okay, thanks. I'll do my best to stop the demon.` to leave that menu.

## Keys

- Sir Prysin NPC 883 `maps/m50_54.jm2:11979` `(3204,3473,0)`. At varp 1 the last option `Gypsy Aris said I should come and talk to you.` is the quest line (`sir_prysin.rs2:248-250`). Key hunt is set at `:123` before the reminder menu. Prefer `Well I'd better go key hunting.` Holding all three keys on a later talk hands over Silverlight (`:35-36,64-76`) with no menu. Both transitions reread the journal rather than treating NPC dialogue as game-chat messages.
- Traiborn NPC 881 `maps/m48_49.jm2:7001` `(3112,3162,1)`. Tower stairs are `fai_wiztower_spiralstairs` LOC 12536 at `maps/m48_49.jm2:5972` `(3103,3159,0)`, middle `:6397`, top `:6809`. Those names are priced (`nav-bake/src/transport.rs` EXTRA_TICKS). `traiborn.rs2:34-38` offers the key request; `:130-147` offers `I'll get the bones for you.` and sets varp 3. The first step claims only a fully completed conversation, not an unobservable bone-counter value. The second talk settles only on `silverlight_key_1`. Both use the established three-tick closed-chat gap because `:206-211` closes chat, delays two ticks, adds the key, delays two ticks, and opens the next page.
- Bones: `goblin_unarmed_melee_1`–`_7` at `maps/m49_50.jm2:6702-6713`, stand `(3146,3232,0)`, box east of x=3135. `drop_tables/scripts/goblin.rs2:5-11,30-32` drops `npc_param(death_drop)`, default `bones` (`npc_combat.param:29-31`). These ids are not the 289 `goblin`/`goblin_armed` field.
- Before collecting 25 bones, `bank deposit_all` makes inventory room and keeps coins, bones, keys and Silverlight, following the established 289 recipe. The existing `pack_only` fact with that exact keep list both settles and skips the completed bank action; a top-level authored sequence cannot use the recipe's implicit cursor to avoid repeating a bank whose old skip guard stayed false. It does not seed or alter any quest state.
- Captain Rovin NPC 884 `maps/m50_54.jm2:12009` `(3204,3496,2)`. Tower stairs are `scenery_1738` at `maps/m50_54.jm2:7052` `(3202,3497,0)`, `scenery_1739` `:9774` plane 1, `scenery_1740` `:11229` plane 2. Those names are priced. `captain_rovin.rs2:57-61` adds the important option only during the key hunt. The next menu's last option is the ale joke and does not give the key. Prefer `There's a demon who wants to invade this city.` (`:28-45`).
- Drain: bucket OBJ 1925 at `maps/m50_54.jm2:12025` `(3222,3491,1)`. Palace south stairs LOC 1722 `maps/m50_54.jm2:7400` `(3212,3473,0)` climb to floor 1 (`stairs.rs2:11`). Sink LOC 874 `sink2` `:7949` `(3224,3494,0)`, category `watersource` (`water_sources.rs2:63-84`). Drain LOC 2843 `questdrain` `:8021` `(3225,3495,0)`. Pouring `bucket_water` spawns `silverlight_key_3` at `0_50_154_25_41` `(3225,9897,0)` (`demon_slayer.rs2:23-39`).
- The observed pour message is a `message_state` checkpoint for both filling and pouring, following the existing cook/cog pattern. The pour returns an empty bucket before the next selector pass; merely checking the bucket's contents would immediately refill and re-pour forever instead of walking to the sewer. Once inside the sewer, both preparation steps also skip without relying on retained chat history.
- Manhole LOC 881 `manholeclosed` at `maps/m50_54.jm2:8588` `(3237,3458,0)`, `op1=Open`. Open `loc_change`s to `manholeopen` (`manholes.rs2:1-5`). Climb is `op1=Climb down` and `p_telejump` z+6400 (`:8-9`). The paired 377 nav transport derives the closed/open operations together; the Path walks to `(3237,9858,0)` rather than duplicating those clicks.
- Sewer exit is LOC 1755 `ladder_1755` at `maps/m50_154.jm2:7911` `(3237,9858,0)`, `op1=Climb-up`, z-6400 (`ladders.rs2:90-97`). The Path walks to its surface landing `(3237,3458,0)`. 377 `loc.pack` has no `ladder_from_cellar`. Nearby aggressors cited on the walk are `maps/m50_154.jm2:8854-8873`.

## Delrith

- NPC 879 at `maps/m50_52.jm2:8272` `(3229,3370,0)`. 289's cited spawn tile is `(3229,3369)`; this Path uses the 377 row.
- Attack is refused unless Silverlight is worn (`npc_combat.rs2:172-181`). The Path equips it. At 0 hp he becomes `delrith_weakened` for 100 ticks and opens four incantation options (`delrith.rs2:9,21`). The last option is the correct phrase; it is still preferred with `strict` so a restored demon is not accepted. Options 1–3 restore him (`:32-45`).

## Not copied from 289

- Journal rules use the actual 377 journal text, without inventing a Traiborn bone counter.
- No `ladder_from_cellar` target.
- Delrith's stand is the 377 placement.
- Bones come from the 377 unarmed-melee ids, not the 289 goblin names.
