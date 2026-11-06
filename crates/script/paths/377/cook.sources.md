# Cook's Assistant — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. This sidecar records the exact content surfaces behind `cook.json`; map entity IDs are the values in the `.jm2` files and are resolved through the corresponding `pack/*.pack` tables.

## Quest state and hand-in

- Cook's NPC is `cook`, placed at map `m50_50.jm2:10644` (`0 9 15: 278`), resolved by `pack/npc.pack:279` (`278=cook`). The Lumbridge handler dispatches the start, in-progress, and completed quest branches at `areas/area_lumbridge/scripts/cook.rs2:1-13`.
- The start dialogue first offers “What's wrong?” then asks “Yes, I'll help you.” / “No, I don't feel like it. Maybe later.” (`quests/quest_cook/scripts/quest_cook.rs2:1-39`). The affirmative branch writes `%cookquest = 1` and sends progress at `:31-36`.
- In-progress Cook dialogue checks for `pot_flour`, `egg`, and `bucket_milk` together (`quest_cook.rs2:42-47`). The completion branch deletes one of each, then queues completion (`:77-85`). The queue writes `^cook_complete`, advances cooking by 3,000 tenths, logs completion, and sends `send_quest_complete` with 300 Cooking XP (`:87-91`). The complete value is 2 (`general/configs/quest.constant:4`). The Path therefore uses only the colour bindings `cook:0`, `cook:1`, `cook:2`; its progress rules/flags remain empty. It adds no journal reader or dialogue-rule surface.
- `cook_journal.rs2:4-29` independently confirms the journal's not-started, in-progress ingredient, and completed states. The selected 377 dialogue UI is not supplied by this Path; the terminal talk step is settled by the quest colour.

## Ingredients and world targets

- **Egg:** `maps/m50_51.jm2:7983-7985` has OBJ 1944 at plane 0, local `(26,37)`; `pack/obj.pack:1945` maps 1944 to `egg`. Chunk `(50,51)` resolves this to `(3226,3301,0)`.
- **Empty bucket:** the same OBJ section has ID 1925 at plane 0, local `(25,30)` (`m50_51.jm2:7984`); `pack/obj.pack:1926` maps it to `bucket_empty`, world tile `(3225,3294,0)`.
- **Empty pot:** `maps/m50_50.jm2:10731` places OBJ 1931 at plane 0, local `(9,14)`; `pack/obj.pack:1932` maps it to `pot_empty`, world tile `(3209,3214,0)`. The Path takes this ground spawn; it does not assume a purchase.
- **Milk:** the milkable target is the `fat_cow` location, not a generic cow NPC. `maps/m50_51.jm2:7329` places LOC 8689 at plane 0, local `(52,11)`; `pack/loc.pack:8690` maps 8689 to `fat_cow`; `all.loc:53381-53394` names it “Dairy Cow” and gives it `op1=Milk`. Chunk `(50,51)` resolves to `(3252,3275,0)`. `general_use/cow_milking.rs2:19-50` implements the location and its use-on handler: an empty bucket is consumed and `bucket_milk` is produced.
- **Wheat:** `maps/m49_51.jm2:5323` places LOC 5583 at plane 0, local `(22,36)`; `pack/loc.pack:5584` maps it to `wheat_tall`. `all.loc:39416-39427` labels it Wheat and defines Pick; `general_use/scripts/pickables.rs2:13-29` produces `grain`. The world tile is `(3158,3300,0)`.

## Windmill flour sequence

- The Lumbridge mill base is LOC 1781 at `maps/m49_51.jm2:5645` (plane 0, local `(30,42)`), mapped by `pack/loc.pack:1782` to `millbase`. `all.loc:12407-12450` maps `millbase` through `millcheck_multi`: state 0 is `millbase_empty`, positive flour states are `millbase_flour`, whose operation is Empty. `windmills.rs2:21-69` checks that the player has `pot_empty`, consumes it, and produces `pot_flour` when flour is available.
- The ladder occupies the same local `(28,43)` tile on all three floors: `maps/m49_51.jm2:5584` (LOC 12964, floor 0), `:7067` (12965, floor 1), and `:7305` (12966, floor 2). `pack/loc.pack:12965-12967` maps these to `qip_cook_ladder`, `qip_cook_ladder_middle`, and `qip_cook_ladder_top`; `all.loc:90004-90040` defines their exact climb operators. `ladders+stairs/scripts/ladders.rs2:159-176` moves one floor per operator. These map to world tile `(3164,3307)`.
- The Path uses four explicit ladder interactions: ground `Climb-up` to floor 1, middle `Climb-up` to floor 2, top `Climb-down` to floor 1, then middle `Climb-down` to floor 0. Each interaction settles against a `near` fact on the destination level; no unavailable transport or guessed ladder is used.
- The upper-floor hopper is LOC 2714 at `maps/m49_51.jm2:7318` (plane 2, local `(30,43)`), mapped by `pack/loc.pack:2715` to `hopper_lumbridge`; `all.loc:19305-19312` identifies the hopper. `windmills.rs2:85-97` consumes `grain` on the non-full hopper and records the hopper state.
- The upper-floor controls are LOC 2718 at `maps/m49_51.jm2:7316` (plane 2, local `(30,41)`), mapped by `pack/loc.pack:2719` to `hopperlevers1`; `all.loc:19341-19350` gives the Operate action and mill coordinates. `windmills.rs2:99-123` handles the lever, moves the flour count into the mill-base state, and emits the cited grain-slides / full-bin messages used by the Path's settle predicates.

## Candidate classification and limits

The full direct-call inventory and explicit partial-surface notes are in `/Volumes/dev-scratch/fairy-ring-evidence/QUESTER-377-1106/quest-candidates.md`. A `send_quest_complete` occurrence is only a static terminal marker, not proof of a full path or runtime capability.

Cook's Assistant completion is source-backed end to end through the three items, windmill, positive dialogue branch, and quest-colour transition. The separate Lumbridge range handler is still explicitly TODO (`quest_cook.rs2:93-101`); range cooking is not part of this Path and is not claimed implemented. No cargo, test, lint, formatting, smoke, or live check was run here; the parent owns validation and the live witness.
