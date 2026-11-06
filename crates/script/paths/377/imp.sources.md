# Imp Catcher — revision 377 source map

Canonical source is `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. This documents the selected content, not historical accuracy.

- Wizard Mizgog is NPC 706 (`pack/npc.pack:707`), placed on plane 2 at local (31,27) in `maps/m48_49.jm2:7002`, world (3103,3163,2).
- In `scripts/areas/area_wizard_tower/scripts/wizard_mizgog.rs2:1-5,39-52`, the two exact start choices are “Give me a quest!” and “Give me a quest please.”; the script sets `%imp` to started and updates the quest colour. With the four beads the started branch consumes one of each and queues completion (`:6-19`). `scripts/quests/quest_imp/scripts/quest_imp.rs2:1-6` sets the complete state and awards 875 Magic XP and the amulet; `scripts/general/configs/quest.constant:9,103` defines complete 2 and one quest point.
- Imp NPC 708 is named by `pack/npc.pack:709`; `maps/m47_51.jm2:7556-7564` places three at (3009,3307,0), (3011,3314,0), (3015,3314,0), inside the authored hunt box. Imps drop the four bead identities via `scripts/drop_tables/scripts/imp.rs2:1-19`. Loot is observed, not seeded. Inventory/bank checks are host facts, not journal text.
- Progress uses only the quest tab's three colours, no journal text. The start and hand-in are separate: unlike the old 289 path, the 377 not-started branch only starts the quest even when beads are held (`wizard_mizgog.rs2:2-19,39-52`).
