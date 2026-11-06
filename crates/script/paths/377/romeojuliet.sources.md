# Romeo & Juliet — revision 377 source map

Canonical source: `/Volumes/dev-scratch/fairy-ring-worktrees/canon-1105/content`. This sidecar records the exact content surfaces behind `romeojuliet.json`; map entity IDs are the values in the `.jm2` files and are resolved through the corresponding `pack/*.pack` tables. The 289 `romeojuliet.json` was a structural lead only; stage values, NPC names, tiles and journal strings below are all 377.

## Quest state machine

Varp values are `quest_romeojuliet.constant` (whole file): not started 0, spoken romeo 10, spoken juliet 20, passed message 30, spoken father 40, spoken apothecary 50, juliet crypt 60; complete is 100 (`general/configs/quest.constant:5`). The completion queue (`quests/quest_romeojuliet/scripts/quest_romeojuliet.rs2:1-4`) writes `%rjquest = 100` and sends `send_quest_complete` with five quest points (`general/configs/quest.constant:106`). Every advance in this quest is a dialogue write; there are no stat, combat or item-production gates, so all sequences are talk steps plus bank/ground recovery.

- Start (`areas/area_varrock/scripts/romeo.rs2:17-44`): “Can I help find her for you?” leads to `romeo_tell_her`, which writes `%rjquest = 10` (`:39-44`). The two refusal branches (`:46-52`) do not advance.
- Juliet's message (`areas/area_varrock/scripts/juliet.rs2:53-65`): talking to Juliet at state 10 runs `juliet_from_romeo` into `juliet_agree_message`, giving one `julietmessage` and writing `%rjquest = 20` (`:62-65`). No choices are offered on this branch.
- Message delivery (`romeo.rs2:54-63`): talking to Romeo at state 20 unconditionally deletes one message and writes `%rjquest = 30` (`:57-58`).
- Father Lawrence (`areas/area_varrock/scripts/father_lawrence.rs2:54-62`): talking at state 30 writes `%rjquest = 40` with no choices.
- Apothecary briefing (`areas/area_varrock/scripts/apothecary.rs2:97-107`): talking at state 40 writes `%rjquest = 50` with no choices.
- Potion (`apothecary.rs2:82-95`): at state 50, talking with berries held deletes them and gives one `cadava` (`:86-92`); with the potion already held the shop branch runs (`:83-85`); with neither, “Keep searching for the berries.” (`:93-95`).
- Potion delivery (`juliet.rs2:88-103`): at state 50 with the potion held, the talk deletes it and writes `%rjquest = 60` (`:95-102`). Without it (`:89-94`) there is no advance.
- Finale (`romeo.rs2:90-98`): talking to Romeo at state 60 queues completion unconditionally (`:98`).
- Post-quest lines (`romeo.rs2:12-14`, `juliet.rs2:105-109`) confirm the terminal state.

## Journal rules

All strings are from `quests/quest_romeojuliet/scripts/romeojuliet_journal.rs2`; rules run most-complete first because every state repeats the earlier paragraphs, so each rule keys on the newest paragraph.

- `romeojuliet:100` ← “I was rewarded for all of my help regardless. QUEST COMPLETE!” (`romeojuliet_journal.rs2:64-76`). Rule: `all ["i was rewarded for all of my help regardless"]`.
- `romeojuliet:60` ← “I have to find Romeo and tell him what's happened.” (`:53-63`). Rule: `all ["i have to find romeo", "tell him what's happened"]`.
- `romeojuliet:50` ← “I went to the Apothecary regarding making this cadava potion, and he told me to bring him some cadava berries.” (`:36-51`, dynamic tail for potion/berries/neither at `:45-51`). Rule: `all ["i went to the apothecary regarding making this", "potion, and he told me to bring him some"]`.
- `romeojuliet:40` ← “I need to find the Apothecary to make a cadava potion.” (`:27-34`). Rule: `all ["i need to find the apothecary"]`.
- `romeojuliet:30` ← “I should find Father Lawrence and see how we can help” (`:20-25`). Rule: `all ["i should find father lawrence"]`.
- `romeojuliet:20` ← “I should take the message from Juliet to Romeo in Varrock central square.” (`:15-19`). Rule: `all ["i should take the message", "to romeo"]`.
- `romeojuliet:10` ← “I should go and speak to Juliet. I can find her west of Varrock.” (`:11-13`). Rule: `all ["i should go and speak to juliet"]`.
- No `romeojuliet:0` rule: the not-started colour covers it, matching the 289 lead. No flags: the 377 journal prints no counts or lost-item lines; recovery is by re-talk, not by journal text.

## NPC placements

- Romeo is NPC 639 (`pack/npc.pack:640`), placed at `m50_53.jm2:12932` (`0 11 33: 639`), world `(3211,3425,0)` — Varrock Square, matching the journal (`romeojuliet_journal.rs2:9`).
- Juliet is NPC 3323 (`pack/npc.pack:3324`, `juliet_multi_visible`), placed at `m49_53.jm2:10592` (`1 22 33: 3323`), world `(3158,3425,1)` — west Varrock upper floor. The Path targets `juliet_multi_visible` because the quest script binds `[opnpc1,juliet_multi_visible]` (`juliet.rs2:1`); the bare `juliet` config (NPC 637, `pack/npc.pack:638`) is not the placed quest NPC.
- Father Lawrence is NPC 640 (`pack/npc.pack:641`), placed at `m50_54.jm2:12002` (`0 54 28: 640`), world `(3254,3484,0)` — the church north-east of Varrock.
- The Apothecary is NPC 638 (`pack/npc.pack:639`), placed at `m49_53.jm2:10591` (`0 59 12: 638`), world `(3195,3404,0)`.
- Item IDs: `julietmessage` 755 (`pack/obj.pack:756`), `cadavaberries` 753 (`pack/obj.pack:754`), `cadava` 756 (`pack/obj.pack:757`).

## Talk-only states and item recovery

- State 20 without the message: `juliet.rs2:67-75` re-gives it when neither inventory nor bank holds one (`replace-message`); Juliet's own check covers the banked case (`juliet.rs2:68`). Banked copies are withdrawn first (`scan-message-bank`, `withdraw-message`).
- State 50 without berries or potion: `take-berries` targets the ground spawn at `m51_52.jm2:6653` (OBJ 753 at `0 13 42`), world `(3277,3370,0)`; sibling spawns at `m51_52.jm2:6649` (`(3266,3361,0)`) and `:6652` (`(3273,3375,0)`) are the same bush patch. `make-potion` converts berries to potion by talk; a banked potion or berries are withdrawn first.
- States 30, 40, 60 and the finale need no items: the repeat-talk branches (`romeo.rs2:6-11`, `father_lawrence.rs2:5-22`, `juliet.rs2:77-86`, `apothecary.rs2:82-85`) restate directions without advancing, so re-running the talk step is the recovery.
- `monotonic` is true: every advance is a one-way dialogue write with no way to regress a stage.

## Candidate classification and limits

- The Apothecary's shared dialogue is partly owned by One Small Favour once `%onesmallfavour >= 9` (`apothecary.rs2:6-30`); the Path assumes a fresh account, like the other 377 quest paths.
- The cadava-berry bush growth stages (`pack/loc.pack` bush configs) are farming content and are not modelled; the Path uses the static ground spawns.
