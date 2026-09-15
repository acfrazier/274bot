# P4 player action slot preservation

## Change

`crates/host-play/src/lib.rs` now projects native player actions into the existing FlatBuffer string vector without removing absent entries. A missing native action is emitted as the existing empty-string sentinel, preserving the native one-based operation slot. The JavaScript shim already excludes empty and `hidden` entries from public actions and `opIndex` continues to resolve the original slot.

No synthetic Trade action is added: a missing or hidden native slot remains unavailable.

## Regression

`tests::script_snapshot_player_actions_preserve_native_slots` builds a real `Client` snapshot producer with a native player, emits the production snapshot FlatBuffer through `script_snapshot_fb`, decodes it, and verifies:

- `[null, null, Follow, Trade with, null]` remains five positional wire entries (`empty`, `empty`, `Follow`, `Trade with`, `empty`), so `Trade with` remains native op4;
- missing op4 remains an empty slot;
- hidden op4 remains `hidden` and is not synthesized or moved.

## Verification

- `cargo test -p host-play script_snapshot_player_actions_preserve_native_slots --lib` — passed (1 test).
- No LIVE run, per task scope.
- `crates/host-play/src/catalog_core.rs` and `crates/script/src/shop.rs` were pre-existing concurrent-owner modifications and were not edited or staged by this task. Full formatting is currently prevented by a pre-existing formatting difference in `catalog_core.rs`.
