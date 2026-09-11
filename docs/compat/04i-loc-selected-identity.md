# Preserve selected loc identity through compatibility dispatch

`Loc.interact` now queues the selected loc type `id` with the existing tile and action. Host-play `InteractReq::Loc` dispatch matches that id on the current snapshot row at `(x,z,level)` and refuses when the identity is missing, replaced, forged, or the requested action is absent. It does not fall back to another loc that merely shares the tile. When `id` is omitted, first-row coordinate matching is unchanged.

The confirmed FlaxPicker 274 stall is Wall 980 then Ground Flax 2646 on the same tile. JS already selected Flax/Pick; the host previously bound the first coordinate row and `ActionSpec::Label("Pick")` refused on the wall. Selected 2646 now dispatches OP_LOC2 on the flax typecode in either snapshot row order. A raw forged id cannot bypass current-snapshot or action checks. Loc id travels on the existing FlatBuffer `Interact.index` slot (same as `open-booth`); old buffers without index decode as legacy.

Focused script tests run Loc.interact through the isolate value-bridge with wall-first and flax-first posted rows, refuse an absent action without queueing, and round-trip selected and legacy requests through FlatBuffers. Host-play tests plant wall 980 and flax 2646 on one tile, dispatch the selected id, refuse stale/missing/forged/invalid action, keep legacy first-row behavior, and compose encode/decode with host dispatch. This task does not change catalogs, fixtures, client, API public types, native timeouts, or LIVE evidence.

## Limits

Root owns isolated 274/100adccc FlaxPicker LIVE requalification and ledger acceptance. Layer is not added to the posted SceneEntity; loc type id plus tile distinguishes the confirmed co-located wall/flax pair. Same-type co-located layers are not covered. `crates/host-play/src/lib.rs` remains a shared runtime hotspot.
