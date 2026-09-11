# Complete named nearest-bank approach with existing host verbs

Named `Bank.openNearest(name, op)` now selects that scene loc once, walks it with existing `WalkNear` radius 1 when Chebyshev > 1 (60_000 ms family bound, no teleports), then posts the same `id/name/action/x/z/level` as named `open-booth`. Adjacent calls stay click-only. Completion is `waitSnapshotAfter` 5000 ms on a fresh loaded generation, not a queued click. Missing or replaced identity after approach returns false and sends no click. Already-open keeps `waitReady(5000)`.

Unnamed `Banking.open()` / `openBooth()` without a stand remain click-only. `openBooth(stand, …)` still walks the supplied stand. Low-level `open_named_booth_at` still geometric-walks when Chebyshev > 1. Non-default `openNearestAccess` still throws `not impl`. This hop does not add opcode/API/client methods, `bankStand`, retry/dialog fallback, a bank session, hold token, or Pause clock.

Focused script tests run the real isolate bridge for distant WalkNear-then-same-identity, adjacent click-only, missing/replaced target, unnamed click-only, fresh-generation completion, and the unsupported access throw. Existing low-level geometric booth tests remain the OpenBooth control. Root owns East/West vial LIVE after review and an isolated build.

## Limits

No worker LIVE. `crates/script/src/shim/bank.js` is a concurrent hotspot with Input.invButton `slot`/`comId`; this hop only adds the named WalkNear composition.
