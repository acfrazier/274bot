# Common-loot matching and bank fill capability

Use configured `sol` defaults, then same-card `reviewer` and stop. This task
depends on completed source review of first banking family t_e52e0a03; read its
actual result and report before implementation. Host branch is
codex/rs2b0t-multirevision. Read applicable instructions, fail-closed-dispatch,
plan step 6, and 04-provisioning-recovery-design.md. The Grok 4.6 design is
source guidance, not permission to adopt a false-positive predicate or an
unverified content ID. Scripts remain revision agnostic.

Implement only the related bank matching/fill additions:

- Rust owns the common-loot substring predicate and its published name list.
  Thin JS maps matchesCommonBankLoot and composes depositMatcher's existing
  script-supplied predicate with the Rust common result when includeCommon is
  true. Preserve includeCommon=false and short-circuit behavior. Verify the
  casket fact against both selected cache/source identities rather than assume
  id 405 is universal. No foreign planner/runtime/table body copied into JS.
- Bank.withdrawLoad uses the reviewed fresh bank and pending withdrawal path.
  Preserve the frozen enabled-caller contract: free slots zero may complete
  without a send; an absent/stale/empty stock row cannot count as progress;
  use actual Withdraw-All where supported, otherwise Withdraw-X for free slots.
  Settlement uses the established 4000 ms bound and observed inventory/bank
  changes from the captured pre-send baseline. A pre-existing full/empty state
  must not accidentally settle a requested transfer. No fixed sleep, no JS
  planner and no private duplicate count-dialog state machine. Reuse family 1.

Inspect only relevant callers in both full-hash catalog roots recorded in the
support ledger. Scope: api content helper and focused tests, script host
callbacks/thin shim/transport only as needed, and host-play pending bank
execution/posting plus composed tests. No client packet changes unless a
concrete missing fact is reported to root first. Do not edit nav, profiles,
frontends, external engine/content, world or root live harnesses, STATE, the
support matrix or unrelated WIP. Do not lift the 289 gate or run live clients.

Checks must traverse an actual loaded script call -> IPC/native callback ->
Rust capability -> posted/returned observation, with includeCommon on/off,
positive/negative name/ID, stale bank, real stack/fill progress, timeout and
Pause/Stop/reset. Preserve the prior banking tests and append-only schemas.
Use monotonic time; Pause retains and freezes pending work, Stop/reconnect
abort it. Preserve original command ordering and existing deadlines.

Keep small helpers together in this coherent task; no new universal job
framework or standalone subtask per mapping. Commit only scoped source/tests
and docs/compat/04-capabilities-bank-matching-fill.md with raw focused checks
under evidence/bank-matching-fill. Report exact commits and remaining live
branches. Request same-card review using reviewer, then stop. Reviewers inspect
committed source/exports without stashing or moving any WIP or index. No remote
or gitlink operations; root owns integration and live acceptance.
