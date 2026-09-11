# Add four catalog combat-core fixtures with real target and loot witnesses

Use grok46 defaults after t_993a114f SAME-card review releases shared fixture
ownership (the actual parent card is provided by Kanban). Own scenario/lib.rs,
host-play/tests/catalog_boundary_live.rs, docs/compat/05t-combat-core-fixtures.md
and unique evidence/combat-core-fixtures/. No runtime/API/shim/client/nav/
frontend/foreign/engine/ledger/STATE edits or LIVE. Preserve prior witnesses.

Four real frozen scripts: ChaosDruidKiller default Edgeville Dungeon,
MossGiant default melee, HillGiant default melee, AutoFighter Guard at Start
position with banking=None, solveClues=false, useSpecial=false. Read
fixtures/remaining-combat-world sections2/4/8/16 then verify actual source,
NPC definitions/actions/spawns, chosen gear/skill/style/food native facts.
HillGiant targets display Giant, not an invented Hill giant alias.

Core proof is sustained script-caused combat on the selected target after
qualified scene2 Start: positive XP in actual selected style and at least two
target engagements with a verified target defeat and appropriate real loot
pickup where the script supports it. Capture target identity/index, live-health
or animation/combat evidence, exact loot IDs and ordered observations so XP
from another actor/NPC and seeded inventory cannot qualify. A disappeared NPC
alone is not defeat. Avoid relying on rare random drops: Moss/Giant guaranteed
big-bone drops532 can qualify actual pickup; Chaos druid actual supported
herb199-family/law563/nature561 must be observed honestly and may fail by chance.
No synthetic corpse/drop, awarded XP, post-Start health/tele or inventory cheats.
Prepare actual legal food, wielded gear and levels before Start, acknowledge
all mutations, and baseline after setup. Use robust ordinary gear chosen from
native facts; qualification starts after real readiness, not seeded combat.

Scope these as core combat cells with explicitly stated banking policy, not
full-bank proof. Where script can trigger deterministic ordinary bankAtLootSlots
or lootSlots setting, add a separate coherent bank variant only if the actual
roundtrip fits unchanged existing gold bounds. Never use the old audit's
suggestion to seed a listed loot after a kill; post-Start seeding is forbidden.
No invented food exhaustion or false native policy. DeathRecovery, mage/range,
special, teleports, all food/loadout/bank variants and remaining fighters retain
separate outstanding coverage; do not mark whole cards complete from melee.

Audit core runtime operations before fixtures. Named-bank/cake/special/teleport
and shop/Make-X/fire are separately owned; gate LIVE on any needed review.
No native capability edits here and no card dim based on a missing mapping.
Preserve all host/script timeouts, seed acknowledgments, XP baseline order and
fresh bank-generation semantics. FAIL +exit1 must retain the accumulated witness.

Meaningful local rejections: wrong target/index reused by a new actor, seed-only
XP/loot, readiness-only, target despawn without combat/loot, one attack admission,
no continued work, wrong style skill, unrelated item and stale bank. Use compact
facts already posted where possible; do not deep-copy the world per observation.
Exact host/client Git export+owned overlay, independent empty target, relevant
scenario/catalog tests and strict Clippy. Root handles cache cleanup and LIVE.
Commit scoped files, SAME-card reviewer with exact checked identities and
remaining limits, then STOP.
