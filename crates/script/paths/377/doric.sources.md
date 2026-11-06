# Doric's Quest — revision 377 source map

Canonical source is `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. The Path is consistent with these scripts; it is not independent historical proof.

- Doric is NPC 284 (`pack/npc.pack:285`), at local (8,59), plane 0 in `maps/m46_53.jm2:8071`, world (2952,3451,0). His start choices “I wanted to use your anvils.” and “Yes, I will get you materials.” and the `%doricquest = 10` write are `scripts/quests/quest_doric/scripts/quest_doric.rs2:50-77`. The same script `:84-96` requires and removes six clay, four copper ore and two iron ore; `:112-117` awards 1,300 Mining XP and completes the quest. `scripts/general/configs/quest.constant:3,91` sets complete 100 and one QP.
- Imp NPC 708 (`pack/npc.pack:709`) appears at (3009,3307), (3011,3314), (3015,3314), plane 0, `maps/m47_51.jm2:7556-7564`. `scripts/drop_tables/scripts/imp.rs2:1-3,10-11,64-65` yields clay. The search box surrounds these placed NPCs; the stand tile is the second placement.
- Dwarf NPC 118 (`pack/npc.pack:119`) appears at (2998,3445) and (2999,3453), plane 0, `maps/m46_53.jm2:8088-8089`. The search box surrounds these placements and the stand tile is the first. `scripts/drop_tables/scripts/dwarf.rs2:1-6,13,41-46` yields iron ore and copper ore. Counts are held inventory targets; no ores or quest rewards are seeded. This route uses content drops rather than requiring level 15 mining.
- Progress uses only the quest tab's not-started, in-progress and complete colours; no journal is requested. Item counts are direct inventory facts.
