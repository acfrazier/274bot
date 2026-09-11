# Map Input.invButton onto native component-item operations

`Input.invButton` now queues the selected bank row's exact id, slot, component and 1-based operation. Host-play `InteractReq::InvButton` re-resolves that identity on the current open/loaded bank generation and dispatches `Interactions::interact(OpTarget::Item, ActionSpec::Operation)`. It does not fall back to another same-name row and does not answer the later count dialog.

TannerBot LIVE currently fails after Start with `not impl: Input.invButton`. Both frozen sources select a `Bank.items` row by id, find the Withdraw-X slot, call `Input.invButton`, then wait for `reader.countDialogOpen` and `actions.answerCountDialog`. That caller sequence and its deadlines stay in the catalog. A queued inv-button is not a completed withdrawal.

`Bank.items()` now forwards posted `slot` and `component_id` as `slot`/`comId` so the selected identity reaches the mapping. Posted bank rows carry the live component id. Invalid operations, closed or stale banks, missing or replaced identity and forged ids send nothing.

Focused script tests run Bank.items plus Input.invButton through the isolate value-bridge with two same-name bank ids in either row order, refuse invalid/forged/wrong-slot/closed/missing cases without queueing, omit answer-count, and round-trip through FlatBuffers. Host-play tests plant two Dragonhide ids, dispatch INV_BUTTON5 on the selected id, refuse stale/closed/missing/forged/invalid op, and compose encode/decode with host dispatch. This task does not change catalogs, fixtures, client, API public types, native timeouts or LIVE evidence.

## Limits

Root owns isolated TannerBot LIVE requalification and ledger acceptance. `crates/host-play/src/lib.rs` remains a shared runtime hotspot. isolate.fbs is unchanged; inv-button reuses existing Interact slots (`bank_item_id`, `source_item_slot`, `component_id`, `stand_op`, `bank_generation`).
