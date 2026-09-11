# Unique excerpts (t_b3223dcc)

Line numbers are from the objects hashed in `hashes.json`. Not product
patches. Do not clone the foreign bodies into the shim.

## Foreign Bank.openNearest (Bank.ts 470–510, 531–548)

openNearest first interacts, then if distance > 1 walks a reachable
cardinal counter (`Reachability.canReach`) radius 1 / 15000 ms, else
walks the booth tile radius 1, then retries an adjacent named loc.

bankStand neighbours are booth±1 cardinal only. Diagonals are not
considered. Geometric signum-adjacent used by host OpenBooth is a
different tile rule.

## Shim Bank.openNearest (bank.js 273–349)

openNearest → openBooth(undefined, name, op). No stand walk. Named loc
filter + distance sort → queue open-booth with identity →
waitSnapshotAfter 5000.

openBooth(stand, …) already queues walk-near stand radius 1 (wait
120000) before that click. openNearestAccess queues walk-near to
nearest_booth radius 1 (wait 60000) then unnamed openBooth.

## Host open_named_booth_at (interact.rs 924–1002)

Chebyshev > 1 → Walk to
`(loc.x + signum(px-loc.x), loc.z + signum(pz-loc.z))`.
Chebyshev ≤ 1 → loc op. StaleTarget on id/name/action miss. No packed
nav. No door leg.

## Host dispatch (host-play HEAD)

OpenBooth named → open_named_booth_at. WalkNearestBank → nearest packed
Booth stand loc tile + ScriptWalkArm.route_with_radius radius 1.
Pause does not drain isolate interacts. Hold drops dispatch.

## PeriodicBank (periodic_bank.rs)

ACCESS_RADIUS = 1, WALK_BOUND_MS = 60_000, BANK_WAIT_MS = 4_000.
Access phase: walk-near dest radius 1, else walk-nearest-bank.
Open phase: open-booth with id and optional name/action, then wait
generation.

## VialFiller bankLeg

walkTo(bankStand) radius 3 / 60_000 then
Bank.openNearest('Bank booth', 'Use-quickly'). East stand (3013,3355,0).
Stand is not passed into openNearest.

## 05l East sequence (retained, not recopied)

Return WalkNear (3013,3355) radius 3 arrived (3010,3352). Eleven
OpenBooth (3011,3354) id 2213 Use-quickly. No Close. Fail
fresh_bank_item_id(227)>=1 at (3010,3352). West 2f return from
(2943,3372) opens (2945,3367). Same mapping, different booth geometry.
