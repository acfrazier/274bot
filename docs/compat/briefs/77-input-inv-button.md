# Map selected component-item operations for TannerBot

Use grok46 defaults after selected-loc identity review releases shared request
and dispatch files. Read fail-closed-dispatch and current STATE ownership rules.
Root controls LIVE, fixtures, ledgers and repository integration.

Actual isolated2f1bd9cf/client9d TannerBot fails after Start with
`not impl: Input.invButton` on revision274. Raw receipts/logs:
evidence/catalog-harness/live/*tanner-bot*-2f1bd9cf.*. Other revision/soft/hard
cells are completing. This is a missing thin mapping onto an existing native
item operation, not permission to patch foreign TannerBot or dim it.

Both frozen TannerBot sources select a Bank.items row by exact ID, find its
Withdraw-X operation slot, then call Input.invButton(item.id,item.slot,item.comId,op).
They wait up to3000ms for reader.countDialogOpen, answer using the existing
actions.answerCountDialog and observe Inventory.used increase. See lines73-98
and input/Input.ts41-42. Preserve this caller sequence and existing deadlines.
Do not replace it with another JS bank helper or recreate foreign input code.

Existing Rust Interactions::interact(OpTarget::Item,ActionSpec::Operation)
already resolves component-item action families and validates current identity.
Implement the minimal request/name/shape bridge, preserving the script-selected
ID, slot, component and operation. Re-resolve only that exact current row at
dispatch; do not fall back to another same-name item. Bind bank requests to
fresh/open/loaded bank observations and generation where necessary. Refuse
closed/stale banks, absent or changed identity, inappropriate container/action
family, invalid operation and forged IDs. Reuse native action visibility,
component and count-dialog checks; no raw opcode table, client API, JS policy,
world copies or lifecycle changes. Unknown/unavailable operations stay honest.

Own only crates/script/src/shim/input.js, minimal InteractReq changes in
shim/mod.rs, scoped host-play dispatch in lib.rs, meaningful unique regression
tests, docs/compat/04j-input-inv-button.md and evidence/input-inv-button/.
No API/client/scenario/catalog harness/game-data/nav/frontend changes unless
an actual missing public fact first requires root coordination. Preserve the
selected-loc predecessor and all autocast/load/recovery work.

Tests should exercise the real shim/request/host composition: selected same-name
distinct bank IDs, current slot/component, current bank generation, actual
component-item command, invalid op and stale/closed/missing replacements sending
nothing. A queued operation alone is not a successful withdrawal. Preserve the
catalog's later count-dialog response and avoid a second count owner. Use an
exact Git-blob source export with explicit client bytes and a new empty target;
affected script/host-play tests plus strict affected Clippy. Commit only owned
files, request SAME-card reviewer with exact identity, then STOP. No worker LIVE.
