# Remaining fighter core fixtures

This bounded extension keeps frozen RockCrab, GreenDragon, FireGiant and
ArdyFighter cards and script source unchanged. Four shared scenario names:

- `rock_crab`: RockCrab default melee/strength at spot 1 (2704,3726,0)
  r4. bankStrategy=Off, solveClues=false. Lobster 379 x8. Catalog
  requires a native dormant Rocks activation into Rock Crab before
  combat progress. Banking is not this cell.
- `green_dragon`: GreenDragon default melee/strength at the wilderness
  field (3096,3814,0) r22, z>=3520. Rune scimitar 1333, worn Dragonfire
  shield 1540, lobster 379 x12. useSpecial=false, usePotions=false,
  escape=Flee to bank, solveClues=false, buryBones=false. Loot is dragon
  bones 536 or green hide 1753 only. Black/red/blue hide 1747/1749/1751
  fail. Escape/bank is not this cell.
- `fire_giant`: FireGiant default melee already inside the east dungeon
  room (2575,9893,0) r10, z>=9000. Waterfall Quest via existing
  `~completequests` + DrainDialogs. Glarial's amulet 295 and rope 954
  prepared. escapeTele=Barrel (free). buryBones=false. Approach, barrel
  escape and bank stay unqualified. Loot is big bones 532.
- `ardy_fighter`: ArdyFighter default Guard/strength at (2661,3306,0)
  r12. bankStrategy=Off, solveClues=false. Thieving 5. No seeded
  cake/bread/chocolate slice. Catalog requires a post-Start stall steal
  of 1891/2309/1901 then Guard combat. Chocolate cake 1897 fails.
  Loadout food is ignored by the script.

Frozen RockCrab.ts is
`7e12b775e0133353f5f45ab8bdca48c62eabe75adec7d09ade997f4f7c62a5ff`,
RockCrabSpots.ts
`a9442a9213e8457c53269072bce9de21ed2625eff719fef4cba8816155201494`,
AmmoLogic.ts
`d6b7166c89c0c7040d27d50272fab08ec3c6df0b8d0516e2c50a6cdbb9ad352b`,
GreenDragon.ts
`46b667c46e82ca8f4d4c7461b93f6be8af05b021bd18126e82784d4264bf1fe9`,
GreenDragonLogic.ts
`d31023052e0bc86b98441657ffb28fd3eb9090abbaefdab00acf00f3687930fd`,
FireGiant.ts
`b7463f7f4f81088270fbdf382aeb6da9eee58aa9ce64678382cd02b4f04e3f21`,
FireGiantLogic.ts
`0640d29191a7ea0faaa9dd44884e968def4f262d68fab38599c699b0f7c2e43a`,
ArdyFighter.ts
`54d57cd5d161c0179cfd9c2d780ddfd4108ad1c857f85f631c6c02bd450e5a6d`.
Hashes match on both `100adccc` and `8e7d965b`. Selected 274/289 item
ids match on both revisions. Generated game-data JSON has pickpocket
NPCs only; scripts query display names Rocks / Rock Crab / Green dragon
/ Fire giant / Guard.

Seed is Attack/Strength/Hitpoints 40, the named weapon, legal food or
empty stall food, extra amulet/rope/shield where required, tele the
work tile, then Start. After Start the scenario watch is Strength XP
from the selected melee style. Catalog proof reuses combat99 two
engagements, identity/index, live health or in-combat evidence, a
verified defeat that is not mere despawn, that XP, plus the extra
gates above.

Name-only, seeded baseline XP/loot/food, readiness-only, target
despawn without combat/loot, one attack admission, no continued work,
Attack XP in place of Strength, Rock crab / Green Dragon / Fire Giant
aliases, black/red/blue hide, chocolate cake 1897, pack-only shield,
missing Rocks activation, missing stolen food, unrelated item, or a
bank roundtrip used as this core fail.

## Adapter audit

- NPC Attack: `Npc.interact('Attack')` queues mapped `op: npc` with
  posted index/name/action. Already mapped; this card does not edit it.
- RockCrab wake: script Attacks dormant `Rocks` and waits for `Rock
  Crab`. Native rename/spawn; no synthetic activation here.
- Combat style: RockCrab/GreenDragon/FireGiant `setCombatStyle`
  melee/strength by name. ArdyFighter `combatStyle=strength`. Already
  mapped. LIVE still waits if combat_styles never post.
- Ground Take: `GroundItem.interact` queues mapped `op: obj`. Already
  mapped.
- Eat food / stall steal: inventory Eat and Baker's stall Steal from.
  Already mapped for Ardy cakes; ArdyFighter reuses that steal path.
- GreenDragon shield: script `Equipment.equip` after Start if the
  shield is in pack. Catalog baseline requires worn 1540.
- FireGiant quest: `Quests.status('Waterfall Quest') === 'notStarted'`
  parks. Seed uses existing `~completequests`. Approach rope/raft is
  not this cell.
- DeathRecovery / SolveClue / PeriodicBank / special / potions /
  teleports / named-bank stay separately owned. Core cells leave those
  paths idle or injected off. Do not mark whole cards complete from
  melee.
- This card does not implement missing runtime operations.

Panel and host-play keep using `scenario::get` / `names()`. Combat-core
chaos/moss/hill/auto 04d7b1f48 and earlier fixtures are unchanged.

`SCRIPT_GOLD_DEADLINE` stays 180s and `SCRIPT_GOLD_WATCH_TICKS` stays
150. Two dragon/giant kills plus loot may not fit that window on LIVE;
record the exact partial phase rather than widen timeouts. Root owns
LIVE/native/ledger qualification.

## Verification

Isolated export and target paths are recorded in
`docs/compat/evidence/remaining-fighter-core-fixtures/verification.json`.
No LIVE or fixture process is launched. Root owns the catalog ×
revision cells after review. Source review does not grant live
acceptance.
