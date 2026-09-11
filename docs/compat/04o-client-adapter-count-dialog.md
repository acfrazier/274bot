# Map native count-dialog reader and answer adapter

Task `t_ee0d3dbd` implements brief 87 on `codex/rs2b0t-multirevision`. Isolated
checks used exact Git regular blobs of host `4cc6cc27d7048c4c60e6b0393ae92132d9b88a1f`
plus the owned overlay (`client_adapter.js`, `client_adapter_count_dialog.rs`).
Client gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3191 exclusive new
files; no archive extraction, overwrite or deletion. New empty target
`.superpowers/review-exports/count-dialog-t_ee0d3dbd-target`. Not LIVE.

## Bridge

`reader.countDialogOpen()` is the posted native `count_dialog_open` fact.
`true` only when that field is boolean true. Posted false, a later closed
post, a missing field, or a malformed value (`1`, `'true'`) is closed state.
It does not throw `not impl`.

`actions.answerCountDialog(value)` queues the existing `answer-count`
command (`InteractReq::AnswerCount`) when `value` is a non-negative i32 and
the posted dialog is open. Invalid values and a closed dialog return false
and queue nothing. Host `Interactions::answer_count` still owns stale/closed
dialog, logout and packet validation. The shim does not invent a second
withdraw/count owner, packet, timeout or synthetic completion.

Queue admission is not a completed withdrawal. The catalog still waits on
`Inventory.used`.

## Tanner path audit

Frozen catalog TannerBot `withdrawXById` is Input.invButton (hide 1739,
slot 0, component 5382, operation 5, bank_generation 3) then
`Execution.delayUntil(reader.countDialogOpen, 3000)` then
`actions.answerCountDialog(count)` then `Inventory.used` increase. Root060
isolated logs already queued that InvButton and failed on the missing reader.

Remaining ClientAdapter members on that core hide-withdraw and tan path are
already mapped: `reader.inventorySize`, `actions.closeModal`,
`reader.modals().main`, `actions.ifButton`. `Bank.openNearest`,
`Bank.depositAllMatching`, coin `Bank.withdrawX`, `Npcs.interact('Trade')`
and `Shop.buy` are other host shims, not missing adapter members. No further
reader/actions mapping is missing on this core path.

## Proof (isolated empty target)

Export `.superpowers/review-exports/count-dialog-t_ee0d3dbd` with
`CARGO_TARGET_DIR=.../count-dialog-t_ee0d3dbd-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/client-adapter-count-dialog/`.

- Closed/open/closed snapshot posts: false, true, false. Reader queues nothing.
- Malformed `1` / `'true'` and a deleted field report closed, not a throw.
- Open dialog + `answerCountDialog(4000)` queues `InteractReq::AnswerCount { value: 4000 }`.
- `0` is a supported integer and queues. Negative, float, NaN, Inf, string,
  missing and `2147483648` queue nothing.
- Closed dialog + 4000 returns false and queues nothing.
- Tanner shape: selected InvButton on closed dialog, then posted open dialog
  and one answer-count 4000. FlatBuffer round-trip of that command succeeds.

`cargo test --locked --offline -p script --test client_adapter_count_dialog -- --test-threads=1`:
8 passed. `cargo clippy --locked --offline --no-deps -p script --test client_adapter_count_dialog -- -D warnings`
passed. `rustfmt --edition 2021 --check` on the new test file passed.

## Limits

Root owns isolated TannerBot LIVE requalification and ledger acceptance. No
API/host/client/schema/runtime dispatcher, fixtures or foreign edits. A green
isolate test is not live tanning success.
