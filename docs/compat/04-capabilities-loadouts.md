# Loadout slots, quantities and required provisioning

## Candidate

Task `t_3737503d` implements brief 26 on `codex/rs2b0t-multirevision`. The client remains at `56d80272bcbda3eb1e22db096c1c5e21d3497de4`. No live client was launched.

## Store and migration

`script::Loadout` now persists `{ name, worn: {slot: name}, carry: [{item, qty}], unassigned? }`. Legacy `{ worn: [name], carry: [name] }` files are read without inventing slots from array order: unnamed gear stays in `unassigned`, string carry becomes `{item, qty: 1}`. Rename/save keeps those items. Temporary test files only; operator `~/.274bot/loadouts.json` was not read or rewritten.

## Rust-owned accessors

Thin JS mappings call host functions. `selectedLoadout` still matches case-insensitively, else first, else null. `weaponOf` reads `worn.righthand` (not first worn entry). `suppliesOf` copies carry including positive quantities. `gearOf` lists named slots then unassigned legacy gear so old items cannot vanish. `foodOf` / `scriptFood` keep the generated `FOOD_HEALS` path.

## Wearpos facts

Slot search uses generated `wear_position` from the selected ObjType decoder (`getWearPosId`), not display names:

| Slot | wearpos | Witness |
|---|---|---|
| hat | 0 | `rune_full_helm` (appearance 8/11 stay unmapped) |
| back | 1 | `cape_of_legends` |
| front | 2 | `amulet_of_power` |
| righthand | 3 | `rune_scimitar`; `iron_2h_sword` also has wearpos2=5 |
| lefthand | 5 | `rune_kiteshield` |
| hands | 9 | `leather_gloves` |
| feet | 10 | `leather_boots` |
| ring | 12 | `gold_ring` |
| quiver | 13 | `bronze_arrow` |

Certificates (`certificate_template != -1`) are excluded from pickers. Search is bounded and query-gated for supplies so the editor does not serialize the full table per frame.

## Editors

Native panel: labeled 3-column equipment layout, slot search with alias/id identity, supply item/qty rows, Copy current equipment (disabled without a focused ingame character, preserves supplies), New/Duplicate/Delete/Save with real file-write feedback. Invalid empty names and failed writes restore the previous store image and do not report Saved. TUI edits `slot=item` and `item:qty` text with the same preservation; it does not copy a graphical layout.

## Provisioning

No foreign loadout planner, AcquireTask, or LoadoutPanel clone. Enabled callers still decide when to bank. After a fresh `Bank.ready()` snapshot, scripts withdraw configured positive quantities and wear `weaponOf`/gear through existing host withdraw-X and Wear ops. A stale/closed bank does not queue those ops. Queued wear/withdraw is not treated as a successful provision; isolate composition asserts the queued requests, not a completed result. Pause/Stop/reconnect keep the existing withdraw/wear pending paths. Quest `gearOf`/`weaponOf` callers stay out of family.

`crates/script/src/load.rs` also contains a concurrent 4-line inventory-row `slot` publish from another card. That file is a hotspot; this task only added the three accessor registers.

## Verification

Raw outputs: `docs/compat/evidence/loadout-capabilities/`.

- `cargo test -p api generated_wearpos`: pass.
- `cargo test -p script --lib loadout`: 10 passed.
- `cargo test -p script --test loadouts_bag`: 4 passed.
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored.
- `cargo test -p tui loadout`: 6 passed (this run, before concurrent scenario WIP).
- `cargo test -p panel loadout`: 5 passed (this run, before concurrent scenario WIP).
- `cargo clippy -p api -p script --no-deps -- -D warnings`: pass.

A later tui/panel rerun could not compile because shared `crates/scenario` WIP references missing `door_opener_scenario` / `gnome_course_scenario` / `flax_picker_scenario`. That crate is outside this task. No live run.

## Limits

Root owns 274/289 live acceptance, native UI screenshot proof, frontend integration and final whole-branch review. Bank-availability previews, acquisition automation, preset marketplaces and further visual redesign remain deferred. Dim/quest-only helpers were not expanded.
