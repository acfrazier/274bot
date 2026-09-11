# Map the posted native inventory into ClientAdapter reader.inventory

Task `t_c0a5785e` implements brief 101 on `codex/rs2b0t-multirevision`. Isolated
checks used exact Git regular blobs of host `7c2eef6eb8db6b01933c333aa64cf5f68169d8d2`
plus the owned overlay (`client_adapter.js`, `client_adapter_inventory.rs`).
Client gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3525 exclusive new
files; no archive extraction, overwrite or deletion. New empty target
`.superpowers/review-exports/inventory-t_c0a5785e-target`. Not LIVE.

## Bridge

`reader.inventory()` maps posted `snap().inv` rows to fresh
`{slot,id,name,count,ops,comId}` objects. `comId` is the posted
`component_id`. Missing or non-integer component identity is `-1`, never a
hardcoded valid inventory component. Occupied native slots include item id 0.
Null names stay null. Counts are the posted integers. Ops are a copied array
of posted actions with no invented Drop/pad. Empty, missing, or non-array
`inv` is `[]`.

The reader does not inspect bank, equipment or bank-side rows. It does not
copy foreign `readInvComponent` / `heldOps` / tab-walk policy. Callers receive
new row objects and a sliced ops array, so mutating the result cannot corrupt
the shared posted snapshot. `inventorySize`, `countDialogOpen`, `toLocal` and
`walkTo` are unchanged.

Omitted unchanged-inv deltas keep the last posted rows. An explicit replacement
or empty post updates the next read. Two isolates keep separate inventories.

## Proof (isolated empty target)

Export `.superpowers/review-exports/inventory-t_c0a5785e` with
`CARGO_TARGET_DIR=.../inventory-t_c0a5785e-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/client-adapter-inventory/`.

- Sparse slots 0/4/8/12/27 keep duplicate Air talisman names with distinct
  ids 1438 and 2691. Item id 0 and a null name are retained. Noted Air rune
  556 keeps posted count 400.
- Posted Wear ops stay a one-entry array. comId 3214 is the posted
  component_id. Deleting that field yields comId -1.
- Empty inv with bank Cow hide 1739 and worn Bronze sword returns `[]`.
- Mutating returned id/name/count/comId/ops does not change the next read.
- Omitted inv delta keeps the talisman; replacement posts Air rune slot 5;
  explicit empty clears. Two isolates do not exchange rows.

`cargo test --locked --offline -p script --test client_adapter_inventory -- --test-threads=1`:
6 passed. `cargo clippy --locked --offline --no-deps -p script --test client_adapter_inventory -- -D warnings`
passed. `rustfmt --edition 2021 --check` on the new test file passed.

## Limits

Root owns isolated MuleCrafter LIVE requalification and remaining gates. No
API/host/client/schema/runtime dispatcher, fixtures or foreign edits. Host-play
still posts inventory `component_id: -1` today; this hop maps that sentinel
honestly and does not invent 3214. A green isolate test is not live mule or
onPaint/core-loop success. Root28 bothrev Mule missing `reader.inventory` is
retained until that reproduction.
