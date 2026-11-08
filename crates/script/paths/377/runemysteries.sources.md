# Rune Mysteries — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. This sidecar records the exact content surfaces behind `runemysteries.json`; map entity IDs are the values in the `.jm2` files and are resolved through the corresponding `pack/*.pack` tables. The 289 `runemysteries.json` was a structural lead only; stage keys follow the 377 varp values (complete 6), not the 289 colour-only stages.

## Quest state machine

Varp values are `quest_runemysteries.constant` (whole file): not started 0, started 1, given talisman 2, received package 3, given package 4, received notes 5; complete is 6 (`general/configs/quest.constant:1`). The completion queue (`quests/quest_runemysteries/scripts/quest_runemysteries.rs2:1-4`) writes `%runemysteries = 6`, grants Runecrafting access and the air talisman, and sends `send_quest_complete` with one quest point (`general/configs/quest.constant:107`).

- Duke dispatch (`areas/area_lumbridge/scripts/duke_horacio.rs2:12-15`) routes not-started to `duke_rune_mysteries_start`, complete to `duke_rune_mysteries_complete`, and anything else to `duke_rune_mysteries_in_progress`.
- Start (`duke_horacio.rs2:21-38`): “Have you any quests for me?” then “Sure, no problem.” writes `%runemysteries = 1`, sends progress, and gives one `air_talisman` (`:27-34`). Declining (`:35-38`) leaves state untouched.
- Talisman delivery (`areas/area_wizard_tower/scripts/sedridor.rs2:52-75,98-109`): “I'm looking for the head wizard.” then “Ok, here you are.” deletes the talisman, writes `%runemysteries = 2` and shows the package lecture (`head_wizard_incredible`, `:111-134`). Refusing the errand (`:131-133`) stays at 2 and is re-talkable.
- Package accept (`sedridor.rs2:120-130`): “Yes, certainly.” gives one `research_package` and writes `%runemysteries = 3`.
- Package delivery (`areas/area_varrock/scripts/aubury.rs2:21-29,35-37,49-61`): “I have been sent here with a package for you.” with the package held deletes it and writes `%runemysteries = 4` (`:58-61`). Talking without the package (`:52-57`) does not advance.
- Notes collection (`aubury.rs2:2-8`): talking at state 4 automatically writes `%runemysteries = 5` and gives one `research_notes` (`:6-8`).
- Notes delivery (`sedridor.rs2:140-180`): talking at state 5 with notes held returns the air talisman, deletes the notes and queues completion (`:177-180`). Talking banked-only (`:149-152`) or empty-handed (`:144-148`) does not advance.

## Journal rules

All strings are from `quests/quest_runemysteries/scripts/runemysteries_journal.rs2`; rules run most-complete first because every state repeats the earlier paragraphs.

- `runemysteries:6` ← “I delivered the notes to Sedridor … QUEST COMPLETE!” (`runemysteries_journal.rs2:44-52`). Rule: `all ["i delivered the notes to sedridor"]`.
- `runemysteries:5` ← “Aubury was interested in the research package and gave me his own research notes … I should take the notes to Sedridor …” (`:36-43`). Rule: `all ["gave me his own research notes", "i should take the notes to sedridor"]`.
- `runemysteries:4` ← “I took the research package to Varrock and delivered it. … I should speak to Aubury again …” (`:29-35`). Rule: `all ["i took the research package to varrock and delivered it", "i should speak to aubury again"]`.
- `runemysteries:3` ← “I should take this Research Package to Aubury in Varrock” (`:23-28`). Rule: `all ["i should take this research package to aubury in varrock"]`. The stage-4/5 texts repeat the delivered-package paragraph but never this line, so order still disambiguates.
- `runemysteries:2` ← “I gave the Talisman to the Head Wizard … I should talk to the Head Wizard again” (`:17-22`). Rule: `all ["i gave the talisman to the head wizard", "i should talk to the head wizard again"]`. The line is the author's best-guess state (`:21` comment) and is still the only text distinguishing state 2 from state 1.
- `runemysteries:1` ← “I spoke to Duke Horacio … I agreed to take it to the Wizards' Tower … I need to find the Head Wizard and give him the Talisman” (`:12-16`). Rule: `all ["i spoke to duke horacio", "i need to find the head wizard and give him the talisman"]`.
- `runemysteries:0` ← “I can start this quest by speaking to Duke Horacio of Lumbridge …” (`:9-11`). Rule: `all ["i can start this quest by speaking to duke horacio"]`.
- No flags: the 377 journal never prints a lost-item or count line, so there is nothing for a flag to capture. No-item states are carried by the step predicates, not the journal.

## NPC placements

- Duke of Lumbridge is NPC 741 (`pack/npc.pack:742`), placed at `m50_50.jm2:10724` (`1 12 20: 741`), world `(3212,3220,1)` — upstairs in Lumbridge Castle, matching the journal (`runemysteries_journal.rs2:10`).
- The head wizard (Sedridor) is NPC 300 (`pack/npc.pack:301`), placed at `m48_149.jm2:6535` (`0 31 35: 300`), world `(3103,9571,0)` — the Wizards' Tower cellar.
- Aubury is NPC 553 (`pack/npc.pack:554`), placed at `m50_53.jm2:12955` (`0 53 10: 553`), world `(3253,3402,0)` — the Varrock rune shop.
- Item IDs: `air_talisman` 1438 (`pack/obj.pack:1439`), `research_package` 290 (`pack/obj.pack:291`), `research_notes` 291 (`pack/obj.pack:292`).

## No-item states and lost-item recovery

Every hand-out in this quest is re-issuable in the source, and each sequence models the no-item state explicitly:

- State 1 without the talisman: `recover-duke` selects “Have you any quests for me?” from the Duke menu (`duke_horacio.rs2:5,7`). That choice dispatches through `duke_handle_rune_mysteries` (`:12-15`) to `duke_rune_mysteries_in_progress` (`:40-47`), which detects `obj_gettotal(air_talisman) = 0` and re-gives it. The money option does not reach this branch. A banked talisman is withdrawn first (`withdraw-talisman`).
- State 1 with the talisman but refusing the package, and state 2 generally: `accept-package` replays `head_wizard_incredible` until the package is held.
- State 3 without the package: `sedridor.rs2:11-17` detects the loss and teleports the package back (`recover-package`). A banked package is withdrawn first (`withdraw-package`).
- State 5 without notes in inventory: `sedridor.rs2:144-148` refuses progress and points back at Aubury; `aubury.rs2:9-20` re-gives duplicates when `obj_gettotal(research_notes) = 0` (`recover-notes`). A banked copy is withdrawn first (`withdraw-notes`). The banked-only branch (`sedridor.rs2:149-152`) is why withdraw precedes the delivery talk.
- State 4 needs no recovery step: the notes are granted by the talk itself, and `collect-notes` skips once they are held.

## Candidate classification and limits

- The state-2 journal line is flagged best-guess in the source (`runemysteries_journal.rs2:21`); the rule for `runemysteries:2` inherits that uncertainty but the stage itself is source-certain (`sedridor.rs2:98-109`).
- Sedridor's lost-package teleport visual is unimplemented upstream (`sedridor.rs2:15` todo); the Path relies only on the re-given item fact.
- Post-quest teleports (`sedridor.rs2:32-42`, `aubury.rs2:22-24,32-33,45-47`) are out of scope and unmodelled.

## Live fixture and unexpected combat

The Varrock approach is not safe for an unprepared level-3 account. Mugger
175 is placed just south of the shop at `(3249,3391,0)` in `m50_52.jm2`
(NPC section; `0 49 63: 175`), within its wander range 12 and hunt range 5
of the shop approach. Its level is 6 and its hunt mode is `cowardly`
(`scripts/_unpack/377/all.npc:5012-5046`). The same map places level-7/20
dark wizards by the southern Varrock entrance.

`all.hunt:37-40` selects the outside-Wilderness not-too-strong filter;
engine `Npc.ts:1017-1019` excludes a player above twice the NPC level.
The live witness therefore labels and observes a combat-only level-40
Attack/Strength/Defence/Hitpoints fixture plus five trout before Start.
This leaves Runecrafting, quest variables, quest items and quest points
unseeded. The harness runbook's area-sized-fixture rule owns this setup;
it is not a quest requirement.

The 289 Rune Mysteries Path has no combat step or automatic fight/flee/resume
rule. Shared TalkRun only hands an interrupt to an explicitly authored
`expect_combat` target. Other combat interrupts park Quester without advancing
the journal or cursor; the runner's combat-interrupted-talk regression also
rejects Resume/SessionReady as an implicit restart. The explicit combat family
has its own encounter and abort/escape rules, but is not a background interrupt
handler. No such policy is added to this non-combat quest.
