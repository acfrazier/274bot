# P4 HerbCleaner eventual empty-bank Stop native fixture

Native option row for frozen `herbcleaner-empty-bank-live`. No LIVE, app build, frozen JavaScript/catalog edit, or panel-log consumption was performed.

## Source pins

- Host base: `d27a4b67dc6ed3329e393507307f77894819a4d4` on `codex/cache-abi-e2e-release`
- Client: `8227e8e60d28642b2eb09a5aa73bda58f6bf20ba`
- Authorized catalog/reference: `96410ec5c779f3d8fe537268cae1a21c0174d16c`
- Refused catalogs for this row: `100adccc037d9f6898080e1cad58fcfc43364775` and `8e7d965be2071d6ec65c3265e12af797082d720a` (they do not carry the frozen eventual empty-bank Stop behavior)
- Frozen harness: `.superpowers/inputs/rs2b0t-96410ec5c779f3d8fe537268cae1a21c0174d16c/e2e/herbcleaner-empty-bank-live.ts`
- Frozen implementation: `.superpowers/inputs/rs2b0t-96410ec5c779f3d8fe537268cae1a21c0174d16c/src/bot/scripts/HerbCleaner/HerbCleaner.ts`

## Native row

| Native id | Seed | Settings | Outer window |
| --- | --- | --- | --- |
| `herb_cleaner_empty_bank` | Herblore 20, Varrock West, empty pack, bank exactly 20 unidentified guam (199) and zero unidentified Marrentill (201) | `herbs=Guam leaf,Marrentill` | 420s (`budget_min` 7) |

The scenario starts only after the loaded seed bank proves guam 20 and Marrentill 0. It then watches post-Start clean guam plus Herblore XP. It deliberately does not require a full pack or a final deposit: the frozen 20-guam case may Stop with clean herbs still held.

Existing `herb_cleaner` and `herb_cleaner_named` production/deposit/refill witnesses remain on `HerbCleanerCycle`; the terminal fixture has its own row and witness.

## Changed seams

- `crates/script/src/load.rs` copies `ScriptRunner.stop` tick and reason from the isolate into one bounded `ScriptStopReceipt`. The reason is UTF-8-safe truncated to 256 bytes.
- `crates/script/src/slot.rs` publishes one `ScriptLifecycleReceipt` for the current runtime generation after script-requested Stop. Fresh Start and operator Stop clear it. Reading it is non-consuming and does not drain the panel's pending-log queue.
- `crates/host-play/src/lib.rs` reads the receipt at the existing snapshot-publication boundary before status publication. `script_observe_with_npc_boxes` may reap the isolate later in the same frame, so a newly published terminal receipt is attached to the next frame's observation.
- `crates/host-play/src/catalog_core.rs` adds the dedicated baseline and ordered witness: post-Start clean guam plus Herblore XP, then a fresh loaded bank generation with both selected unidentified stocks absent, then native `Stopped` with exact reason `every selected herb is empty in the bank`.
- `crates/scenario/src/lib.rs` adds the exact frozen seed/settings/deadline scenario.
- `crates/e2e/fixtures/native-suite/derive.py` and `suite-manifest.json` move the frozen reference association to the distinct native row.

No extra JS↔Rust wire, world copy, supervisor, policy layer, reference-script change, or unbounded log/proof buffer was added.

## Checks

Final focused checks passed:

- `cargo test -p script slot::tests::script_requested_stop_cleans_slot_work_and_allows_fresh_restart -- --exact`
- `cargo test -p script slot::tests::script_stop_receipt_bounds_utf8_reason -- --exact`
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live tests::herb_cleaner_empty_bank_requires_cleaning_fresh_exhaustion_and_script_stop -- --exact`
- `cargo test -p scenario tests::inventory_production_cases_register_exact_ids_and_bank_cycles -- --exact`
- `cargo test -p scenario tests::nav_full_is_a_mainland_follow_to_a_cross_square_destination -- --exact`
- `cargo test -p e2e --lib` — 73 passed
- `cargo test -p e2e --test suite_offline` — 29 passed
- `cargo run -p e2e --bin e2e-suite -- list --only herb_cleaner_empty_bank --json` — exactly one selectable native case
- Regenerated the manifest to a temporary file from the pinned reference and compared it byte-for-byte with `suite-manifest.json`
- `git diff --check`

The witness regression also proves these negative paths do not qualify: empty seeded bank, no post-Start cleaning, stale bank generation, guam remaining, Marrentill remaining, wrong Stop reason, and missing lifecycle receipt. Slot regressions prove operator Stop does not synthesize a script receipt, fresh Start clears an old receipt, pending panel logs remain available, and an overlong multibyte reason stays bounded.

Two broader branch checks were attempted but are not evidence for this scoped row: full `cargo test -p script` reached the branch's `host_js_dts_is_fresh` generated-ABI mismatch, and full `catalog_boundary_live` reached the unrelated Firemaker assertion in `noncombat_core_cells_require_source_cycles_and_refuse_seed_only_paths`. The focused affected checks above pass.

## Proof limits

This is source/offline qualification only. No headed LIVE run was authorized, so the implementation does not claim observed gameplay completion or a human-read screenshot. Root still owns the original headed case after source review. The receipt proves only the latest script-requested terminal Stop for the current generation; it is intentionally a single bounded value, not history. The exact reason is required, and crashes, errors, missing scripts, manual/operator Stop, stale or empty baselines, and unrelated Stop reasons cannot satisfy the witness.
