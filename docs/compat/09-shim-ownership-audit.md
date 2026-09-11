# Thin-JavaScript shim ownership audit

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 12:46 UTC. Kind: bounded read-only architecture/fidelity
audit for brief 122. Not implementation, LIVE, fixtures, ledger, STATE,
dim, Git staging, merge, or push. Root owns repo hygiene and any follow-up.
No routine second review card.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, plan
ownership (JavaScript supplies script logic and thin API name/shape
mappings; do not reimplement foreign banking/loadout/pathfinding/recovery
planners in JS), 04u cake mapping contract, 04k nearest-bank composition,
and brief 122. Branch checked first: `codex/rs2b0t-multirevision` (not
`main`). Work was read-only except this report and
`docs/compat/evidence/shim-ownership-audit/`. Concurrent WIP after
`64d73f337` was ignored.

## Verdict

**The strict contract did not fully hold, but this range did not recreate
the foreign JavaScript runtime, routers, or CakeStall/PeriodicBank
planners.** Most of the 31 shim JS files are posted-fact projections,
identity-preserving queues, or Rust `next()` marshalling. Executable JS
sequencing exists where earlier grok46 designs already authorized one-shot
composition (`stealCakes` walk-then-interact; named `Bank.openNearest`
walk-then-OpenBooth). Those are not thin name aliases. Do not relabel
them as thin mappings merely because the packets go through Rust.

PeriodicBank and DeathRecovery are **not** violations: Rust owns due
evaluation and phase `next()`; JS runs callbacks and waits for observed
completion. The one introduced contract leak that should be corrected
before treating the banking family as clean is the **120000 ms** walk
wait on `Bank.openBooth(stand)` and `Bank.openNearestWorld`.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Audit base | `b2bd5023489ab2e0b6ba690f228217e8984ac91b` |
| Audit pin (inclusive head) | `64d73f33741cd0c49bb2bffd3cd0498674200e71` |
| Campaign HEAD at write-up (ignored WIP) | `b6013ba36ddb8bb0f5a56172be726d3b09a60622` |
| Client gitlink at pin | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Branch | `codex/rs2b0t-multirevision` |
| Shim JS vs main at base | identical (0 files) |
| Brief 122 SHA-256 | `6cc25039580efe787695635b0a27c465f7be1fa5f33eb61c016753b4a373bee7` |
| 04u SHA-256 | `42aeedddd4ba084b7a736e7a7ad5e174914265256bd8e4fb0eee8bc36eb9b5af` |
| 04k SHA-256 | `d37f043f1ed5fda1609882380011c931ef3060dc6a90abba87ac3688d99a477c` |
| Kanban card | `t_ba428941` |
| JS delta | 32 files, 1739 added / 211 deleted |

Pin SHA-256: `cake_stall.js` `d173f107…5959dd92`; `bank.js`
`475a0a55…bf39ec89`; `periodic_bank.js` `e6e73c68…8f63aecd`;
`death_recovery.js` `4e735ed2…383541c`; `autocast.js` `d834af49…3f22b4e0`.
Blobs and per-file class in
`docs/compat/evidence/shim-ownership-audit/`.

## Sorted findings

### F1. Introduced: `Bank.openBooth(stand)` / `openNearestWorld` use 120 s walk waits

- **Where:** `crates/script/src/shim/bank.js` `openBooth` 344–353
  (`delayUntil(adjacent, 120000)` after `walk-near` radius 1) and
  `openNearestWorld` 463–471 (`walk-nearest-bank`, 120000). Introduced
  `2f9099f70` (Bone Burier bank loop). Not on main.
- **Native already available:** `InteractReq::WalkNear` (60000 host
  bound), `WalkNearestBank`, `OpenBooth`. `openNearest` (398–433) and
  inherited `openNearestAccess` already wait 60000. PeriodicBank Rust
  `next()` posts `timeout_ms: WALK_BOUND_MS` (60000) and the family
  report forbids the foreign 120000 router timeout.
- **What it is:** JS composition of existing verbs, plus a foreign
  timeout. Not a 6-attempt Bank.ts clone. Named loc filter is a
  projection. `openNearest` identity-preserving walk-then-open is the
  04k-authorized mapping (`060e15a0f`).
- **Minimum correction:** Use 60000 (or timeout 0 with a Rust-posted
  outcome, as `withdrawX` already does). Do not add retry/op-fallback
  loops.
- **Regression:** BoneBurier / `Banking.bankNearest` / world-open callers
  that currently sit through 120 s on a failed approach would fail
  closed 60 s earlier. Preserve adjacent success, named loc identity,
  and `notImpl` on non-booth `openNearestAccess`.

### F2. Introduced: cake selection/restock filters duplicated in JS

- **Where:** `cake_stall.js` `matchingStall` 70–80 and
  `needsCakeRestock` 87–90 / `atGoal` 62–68. Introduced `b2848b4a7`.
  Native `crates/api/src/cake_stall.rs` already has `select_baker_stall`,
  `needs_cake_restock`, `counts_as_stall_food`, posted `baker_stall`.
  No `__rs2b0t_*` export; JS re-filters `Locs.query`.
- **What it is not:** a CakeStall.ts clone. 04u required
  `'stocked'|'combat'|'aborted'|'no-progress'`, one walk-then-interact
  per call, host-posted stand approach, `onSteal` only on food gain, and
  forbade the 90 s inner loop, chat classifier, STAND/STAND_ALT swap,
  and `classifySteal`. Pin `stealCakes` 92–163 matches that contract.
  `classifySteal` stays `notImpl`. Walk uses `Traversal.walkTo` (60000
  default), not a steal state-machine.
- **Minimum correction:** rustyscript wrappers for `select_baker_stall`
  / `needs_cake_restock`; shim keeps one-shot walk+interact+ABI returns.
  Optional: steal-resolve wait should not be a Pause-advancing 2400 ms
  isolate clock (`RESOLVE_MS`); prefer observed inv change with host
  freeze, same as withdraw-x timeout 0.
- **Regression:** isolate tests that assert JS loc filter / return codes
  must keep the 04u ABI. Do not restore unfiltered `Locs.query().nearest()`.

### F3. Inherited, expanded: `Autocast.arm` click order stays in JS

- **Where:** `autocast.js` 50–104. Base already opened the combat tab and
  fired choose/spell/toggle, then waited once for armed. `90d5fb039`
  added Rust `controls`/`observe`/`begin` token and **stepwise** waits
  (`panel_open`, `selected`, `armed`, 3000 ms each).
- **Native:** `__rs2b0t_autocast` has no `next()`. `actions.ifButton` and
  `Game.openSideTab` exist. Unlike PeriodicBank, JS owns the phase order.
- **Minimum correction:** only if tightening this family — Rust `next()`
  emitting choose/spell/toggle, JS dispatching ifButton. Not a foreign
  autocast-planner port. Do not block stage1 or banking on this.
- **Regression:** keep `notImpl` when posted coms are missing; keep
  false (not throw) when staff tab is absent or a step does not take.

### F4. Inherited residuals (do not treat as this range's new policy)

Not introduced here; still not thin mappings if a later card touches them:

- `Banking.bankNearest`: open + optional deposit + `delayTicks(1)` on
  main; this range retargeted open to `openNearestWorld` (F1).
- `food.shouldEatToUseFood` / `shouldEatFood`: eat decision helper on
  main. This range only mapped `foodForms` from posted items.
- `cakeStallData.shouldReset` / `LOCKOUT_TICKS=10` /
  `RESET_AFTER_REFUSALS=3` on main; `classifySteal` still blocked.
- `thieving_targets.spotRow` Guard/`spots[0]` fallback (fail-open if the
  named target is missing). `isHostileAttacker` in this range is a
  correct Rust predicate marshal.
- `Bank.depositAllMatching` 32-iteration JS loop on main; PeriodicBank
  correctly treats deposit as a script-predicate step after Rust `next()`.
- `Game` combat-label matching (`includes`) on main; this range only
  widened `combatStyleResolution` shape.

An ancillary stub remains a stub: `Traversal.remaining`,
`Tools.toolRestockPlan` / `bankHasBetterGatherTool`, `Game.castOnNpc`,
`DeathRecovery.needs` / AcquireTask, `cakeStallData.classifySteal`.

## Spot-check: PeriodicBank / DeathRecovery

These are the intended ownership split, not automatic loop violations.

Rust `periodic_bank.rs` `next()` chooses Access/Open/WaitReady/Deposit/
AfterDeposit/Close/Return from observation (combat/due/backoff live in
`validate`/`begin`). JS `periodic_bank.js` 98–236 switches on `kind`,
queues `walk-near` / `walk-nearest-bank` / `open-booth`, waits with
Rust `timeout_ms` (fallback 60000/4000), runs `opts.deposit` /
`afterDeposit`, acks before the service continues. Off still
`validate()===false` and sends nothing.

Rust `death_recovery.rs` `next()` chooses wait-respawn / wait-ticks /
walk-near / walk_back. JS 74–138 dispatches; `walkBack` is the
script-owned WildyAgility callback; `needs` still throws
`notImpl('DeathRecovery.needs')`.

JS `fail()` `delayTicks(3)` after PeriodicBank missing access is
acknowledgement delay, not due-evaluation. Do not “fix” these by moving
the for-loop into a cloned TS controller, and do not count a queued walk
as recovery/bank completion (already the Rust tests’ rule).

## Safe boundaries for incremental merge

Root already split stage1 (client/profile/protocol/world). This audit
does not choose staging mechanics.

1. **Merge-safe as ownership:** posted projections (reachability, cook/
   bank locations, spelldb fill, tools bestFrom, ranged dart alias,
   loadout rustyscript, combat runes, tile distance, loc id, inventory
   use-on identity, input inv-button, paint buttons, BotHost tick
   listeners, SettingsStore.displayString, hostile-attacker predicate,
   special cost/bar, DirectNavigator scene walk, Traversal.preload
   no-op). PeriodicBank / DeathRecovery marshal loops. 04k `openNearest`
   60 s walk-then-open.
2. **Merge with F1 noted / corrected:** `openBooth(stand)` and
   `openNearestWorld` 120 s waits. Do not expand them into a JS bank
   router while correcting the bound.
3. **Merge as 04u composition, not as a steal controller:** `stealCakes`.
   Wire Rust select/needs when convenient (F2). Keep `classifySteal`
   blocked. Catalog TaskBot remains the loop owner.
4. **Do not hold** stage1, world, or rust-next families for Autocast
   click-order (F3) or inherited eat/reset helpers (F4).
5. **Do not merge** a later JS inner-loop, chat classifier, stand-swap,
   AcquireTask, or Banking.bankNearest foreign router as “thin mapping”.

No product/source edits, build, LIVE, remotes, merge, or push in this
card.
