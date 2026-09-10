# Host boundary qualification

Current outcome: **pending**. Frozen host `fede3c0d` / client `6cb5a0b1`
has completed source review, but the first controlled 289 live cell failed.
Production 289 slots remain gated. No campaign acceptance or performance claim.

Source and offline evidence: `02a-host-outbound.md`,
`02b-host-snapshot-reset.md`, and
`reviews/host-boundary-integration-grok46.{md,json}`. Actual Grok 4.6 run 1112
approved the integrated source on 2026-09-10; final whole-campaign review remains.

## First controlled cells

Both ran one revision per process against the recorded loopback fixtures,
with the same immutable binary SHA
`3ce3ec1d1570e7cab2f0999490630457ea6a8c16760abaaa54fc2136cb8b6444`.
The source export excludes concurrent world/banking implementation.

| Revision | Result | Observed outcome |
|---|---|---|
| 274 | PASS, exit 0, 36.290 s | After mainland preparation/relog, walked 3220,3212 to 3221,3212; Hans dialogue 4882 with new text; Door 1530 replaced by Door 1531 offering Close; actual IF logout emptied actors/inventory/bank. |
| 289 | FAIL, exit 1, 6.269 s | Login/scene2 and mainland relog succeeded; post-baseline walk reached 3221,3212. The visible snapshot had 19 NPCs but no Hans with Talk-to, so NPC/loc/logout acceptance did not run. |

Raw logs/receipts: `evidence/host-boundary/live/r{274,289}-fede3c0d.{log,json}`.
Failure cleanup is not logout proof. Readiness probes and seed actions are not
action acceptance. Neither failure logs nor predicates have been discarded.

## Fixture correction under review

The pinned 289 Hans source declares Talk-to and a patrol spanning
x=3202..3221 and z=3205..3233, so the chosen static start does not guarantee
visibility of that naturally roaming actor. The first log records only NPC
count, so it cannot establish his exact position or conclusively attribute
the absence to patrol visibility rather than publication.

Root is making the fixture deterministic using the existing local `npcadd hans`
command: observe a new nearby Talk-to identity before the action baseline, then
require actual dialogue from that selected identity afterward. The handler
creates a temporary NPC with a 500-cycle lifetime. No external server patch,
new host capability or deadline extension is involved. The revised harness
also records the pre-seed NPC observations. All action/logout predicates remain.
The correction requires focused source review and a justified new controlled
cell before 289 acceptance; an incidental rerun of the old fixture is not a fix.

Primary fixture references: lostcity-289 content commit `92649430`,
`scripts/areas/area_lumbridge/configs/lumbridge.npc`, and engine `a275ea81`,
`src/network/game/client/handler/ClientCheatHandler.ts` (existing npcadd branch).
