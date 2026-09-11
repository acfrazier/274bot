# Combat-core fixtures for four remaining fighters

This bounded extension keeps frozen ChaosDruidKiller, MossGiant,
HillGiant and AutoFighter cards and script source unchanged. Four
shared scenario names:

- `chaos_druid`: ChaosDruidKiller default Edgeville Dungeon
  (3110,9936,0) r14. Location inject Edgeville Dungeon, combat style
  index 1. Lobster 379 x12 so tripPrepared holds in the field. Own
  bank/trapdoor is not this cell.
- `moss_giant`: MossGiant default melee/strength at safespot
  (2553,3406,0) r10. buryBones=false. DeathRecovery idle. Big bones
  532 is the catalog loot witness. Banking is not this cell.
- `hill_giant`: HillGiant default melee/strength in the pit
  (3110,9832,0) r16. Target display is Giant, not an invented Hill
  giant alias. Blank weapon. buryBones=false. DeathRecovery walkBack
  idle. Loot is big bones 532 or limpwurt 225. Banking is not this
  cell.
- `auto_fighter`: AutoFighter Guard at Start position (2661,3306,0)
  r8. banking=None, solveClues=false, useSpecial=false, melee/strength,
  buryBones=false. Gem-table loot is not required. DeathRecovery idle.

Frozen ChaosDruidKiller.ts is
`51ed34065cb42a85680e122f01bd92903587ca117e3e5ed45e72e75f1a1eb2fa`,
ChaosDruidLogic.ts
`4926ffa1f26ca997f1f4ee58e5b209afdc2256ae9ce888dcdc389c5b4627416a`,
MossGiant.ts
`202ee1216b46a1c5eca0206772b4b3c78c9baf1205aad36f360e677efc0d22b5`,
HillGiant.ts
`615436b5fd4b3a727c6593d3e18a86e592b5fc160164b1ca823d9eb1d4c70c00`,
HillGiantLogic.ts
`bde08aac8e725f3775ebbf0ca3b0c8bbcedc1dad6a89af9c3cfcbfd1eb488e32`,
AutoFighter.ts
`d82972d038849d4a7f897a264ead850b4de218239d54eab16007a4e0c12266c2`,
AutoFighterData.ts
`ac0d6afe217f441b9145d5e3ef4acc6bbab5e0ea05cd9ab609f8c2e14673826e`.
Hashes match on both `100adccc` and `8e7d965b`. Selected 274/289 item
ids match on both revisions. Generated game-data JSON has pickpocket
NPCs only; scripts query display names Chaos druid / Moss giant /
Giant / Guard.

Seed is Attack/Strength/Hitpoints 40, Adamant scimitar 1331, named
food in pack, empty loot, tele the work tile, then Start. After Start
the scenario watch is Strength XP from the selected melee style.
Catalog proof is two target engagements with identity/index, live
health or in-combat evidence, a verified defeat that is not mere
despawn, that XP, and exact loot IDs where the script supports it.
Chaos herb 199-family / law 563 / nature 561 must be observed honestly
and may fail LIVE by chance. Moss/Giant guaranteed big-bone 532
qualifies actual pickup. AutoFighter does not require gem-table loot.

Name-only, seeded baseline XP/loot, readiness-only, target despawn
without combat/loot, one attack admission, no continued work, Attack
XP in place of Strength, Hill giant alias, noted cert ids, unrelated
item, or a bank roundtrip used as this core fail.

## Adapter audit

- NPC Attack: `Npc.interact('Attack')` queues mapped `op: npc` with
  posted index/name/action. Already mapped; this card does not edit it.
- Combat style: ChaosDruidKiller `Game.combatStyles()` /
  `Game.setCombatMode` by button index 1. Moss/Hill/Auto
  `setCombatStyle('strength')` by name. Already mapped. LIVE still
  waits if combat_styles never post.
- Ground Take: `GroundItem.interact` queues mapped `op: obj`. Already
  mapped.
- Eat food: inventory Eat. Already mapped.
- DeathRecovery / SolveClue / PeriodicBank / special / teleports /
  named-bank / cake / shop / Make-X / fire stay separately owned.
  Core cells leave those paths idle or injected off. Do not mark
  whole cards complete from melee.
- This card does not implement missing runtime operations.

Panel and host-play keep using `scenario::get` / `names()`. Resource
gnome/coal 6fc633317, Superheater Attack-30 6c7075ce, vial/potion
e4f5352ab, chicken melee 8fd0f138, bank-close generation f33703d5e,
seed-order 1ec25b521, TannerBot 3a34f879f, Falador West vial start
3368 (2f1bd9cf), RuneCrafter/Mule 6750713b8, rune observation
4cc6cc27d, Ardy stall/pickpocket 43749c42f, inventory reader
9cc6adb1f, station cook/smelt/spin fc5c9a49d, and FlaxAIO/secondaries
d854321b8 are unchanged.

`SCRIPT_GOLD_DEADLINE` stays 180s and `SCRIPT_GOLD_WATCH_TICKS` stays
150. Two giant kills plus loot may not fit that window on LIVE;
record the exact partial phase rather than widen timeouts. Chaos loot
may fail by chance. Root owns LIVE/native/ledger qualification.

## Verification

Isolated export and target paths are recorded in
`docs/compat/evidence/combat-core-fixtures/verification.json`.
No LIVE or fixture process is launched. Root owns the catalog ×
revision cells after review. Source review does not grant live
acceptance.
