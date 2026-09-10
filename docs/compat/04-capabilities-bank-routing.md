# Bank routing and Bone Burier loop capability

Status: supported for the ordinary packed booth path used by both captured Bone Burier revisions. Unsupported bank access shapes remain explicit below.

## Supported path

- `Banking.bankNearest(...)` delegates off-scene selection to the Rust host through `walk-nearest-bank`.
- Rust selects a packed `NavWorld` booth stand, prefers the player's current plane, and arms the existing `ScriptWalkArm` / `Traveller` route to radius 1. JavaScript only waits for an adjacent observed booth and then uses the ordinary observed open path.
- An already adjacent booth opens without a route. An absent packed booth/world returns `false`; the shim does not report a synthetic success.
- Booth opening waits for a fresh bank session generation and a loaded snapshot.
- `Bank.depositAllMatching(predicate)` keeps predicate evaluation in JavaScript but bounds the transfer loop to 32 iterations. Each iteration selects one current matching row, sends one request, and waits for a Rust-observed settlement before reading fresh row identities.
- Empty open side views retain the captured 1200 ms readiness wait. Deposits have a 2000 ms settlement budget.
- Ordinary `Bank.withdraw(...)`, `withdrawById(...)`, and `withdrawAll(...)` now resolve from Rust-observed bank stock progress. Withdrawals have the caller-compatible 4000 ms settlement budget.
- Deposit and ordinary-withdraw outcomes cross the isolate FlatBuffer as a monotonic `(bank_op_result_seq, bank_op_result)` pair. Host refusal, stale bank generation, unloaded/closed bank state, disconnect, stop, and timeout complete `false` rather than silently succeeding.
- Pending bank-operation time freezes while the script is held or paused. Stop/restart and reconnect clear pending state; the script lifecycle cleanup introduced before this change remains intact.
- The Bone Burier can therefore request an off-scene packed booth route, open a fresh bank session, deposit selected inventory rows without duplicate requests, withdraw the configured exact bone item, observe inventory progress, and resume its bury loop.

## Preserved contract details

- Predicate callbacks receive a stable item name string (`''` for a nameless row) and numeric item id.
- Duplicate rows are bounded to one transfer request per observed settlement.
- Mixed keep/deposit predicates are re-evaluated against each fresh snapshot.
- Missing item names, missing operation slots, stale generations, unavailable navigation state, and unsupported access shapes fail closed.
- Legacy fixed and X withdrawal paths retain their existing behavior; `withdrawLoad` / `withdrawX` remain host-owned continuations.

## Explicit residual gaps

- Nearest-bank routing currently supports packed `BankAccess::Booth` stands only. Packed teller-NPC access, dialogue choices, deposit boxes, bank chests, and ladders are not silently mapped onto booth behavior.
- The host preference is current-plane Chebyshev proximity before Traveller route execution; it does not synchronously compare the full path cost to every bank stand.
- A route search that is accepted asynchronously but later produces no path is observed by the shim's bounded 120-second arrival wait and returns `false`; there is no invented route-success opcode.
- Live server validation is owned by the final live task. This capability record is based on deterministic isolate and host composition tests.

## Verification

Deterministic evidence is recorded in `docs/compat/evidence/bank-routing/`:

- `script-bank-loop.txt`: script isolate suite, including off-scene routing, missing-world refusal, duplicate-row bounding, fresh observed deposit settlement, and the captured Bone Burier entry path.
- `host-bank-loop.txt`: host-play suite, including Rust packed-booth selection, packet send, observed deposit and withdrawal settlement, refusal, and lifecycle behavior.
- `captured-bone-burier.txt`: both exact captured source roots request the host-owned nearest-bank route and both seeded bury paths still load and queue `Bury`.
- `quality.txt`: formatting, diff hygiene, and clippy checks.
