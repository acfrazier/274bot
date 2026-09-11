# Autocast through selected-world combat controls

## Candidate

Task `t_79dfc132` implements brief 34 on `codex/rs2b0t-multirevision`. Client gitlink is `9d090ed04957e4efc254f073cda97bc5510ca72b`. No live client was launched. Root owns LIVE, frontend, scenario, ledger and repository integration.

## Facts and provenance

Generator `tools/game-data/generate.ts` now extracts additive `autocast` controls from selected `pack/interface.pack` and `pack/varp.pack` into schema-3 JSON (fields append; schema version unchanged). Runtime serde is `api::game_data` with `#[serde(default)]`.

| Fact | 274 and 289 packed identity |
|---|---|
| `staff_tab_root` | `combat_staff_2` = 328 |
| `choose_com` | `combat_staff_2:auto_choose` = 353 |
| `spell_panel_root` | `staff_spells` = 1829 |
| `spell_grid_base` | `staff_spells:ssb0` = 1830 |
| `toggle_com` | `combat_staff_2:auto_toggle` = 349 |
| `magic_varp` | `attackstyle_magic` = 108 |
| `selected_value` / `armed_value` | 2 / 3 from `auto_cast.rs2` / `player_attackstyles.rs2` |

328/353/1829/349/1830/varp 108 are audit inputs verified on both selected caches, not shim constants. JS reads them through `__rs2b0t_autocast`. `spellButtonCom` uses posted `spell_grid_base + ssb`. Inventory-target casts still use magic-tab `spell_buttons`.

## `Autocast.arm`

Thin shim preserves the frozen three-press sequence and 3000 ms waits: open combat tab, press choose, wait staff tab root == spell panel, press `1830+ssb`, wait magic varp == 2, press toggle, wait armed (varp == 3). Missing packed choose/toggle/grid throws `not impl` and sends nothing. Unknown SPELL_DB name or a combat tab that is not the staff layout returns false. A queued if-button is not completion; isolate proof waits for the posted panel root and varp 3.

Pause skips isolate ticks. Guardian hold skips the Execution pump. Stop/session reset bumps the autocast token so a parked wait returns false and does not press the next control. Melee/range `Game.combatStyleResolution` is unchanged.

Host-play always posts varp 108, including 0, so armed/selected observation is not dropped by the nonzero/take-32 varp cap. Live staff vs chooser identity stays `side_tab_ifaces[0]`.

## Callers

ChickenKiller / RockCrab / MossGiant / FireGiant / AutoFighter / GreenDragon mage `ArmAutocast`. Special attack and spellbook teleport remain separate.

## Hotspot

`crates/script/src/load.rs` also carries death-recovery and periodic-bank registers. This task only added `__rs2b0t_autocast` and Pause/Resume/ResetSession/hold hooks. `crates/host-play/src/lib.rs` only gained the always-posted attackstyle_magic varp. Concurrent static-loc snapshot hunks were not edited.

## Verification

Worktree diagnostic target `target-t_79dfc132-check` (not the shared campaign target). Isolated empty-target receipts after commit are in `docs/compat/evidence/autocast-capabilities/`.

- `npx tsx tools/game-data/generate.test.ts`: pass (pack parse + extracted IDs).
- Two full generations: 274 `e08bcf585d3f310381e2fc12bc1a99d3345b3799e1e2807b9ab01d711e4bc87d`, 289 `2404040764d0282b0458b6ee28b6871b48f0b90ed1e334e0b085770a498eaf34`.
- `npx tsx tools/game-data/verify.ts`: both revisions 16 spells, 14 staves, autocast 328/353/1829/1830/349/108/2/3.
- `cargo test -p api --test game_data`: 4 passed.
- `cargo test -p script --lib autocast`: 2 passed.
- `cargo test -p script --test autocast`: 8 passed (missing control throw, unknown spell, staff not attached, initially armed on both caches, three presses then posted varp 3, stale chooser does not press grid, Pause/hold/session abort, melee resolution).
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored.
- `cargo test -p script --test combat_style_contract`: 1 passed.
- `cargo test -p script --test spell_facts`: 6 passed.
- `cargo clippy -p api -p script -p host-play --no-deps -- -D warnings`: pass.

## Limits

Root owns 274/289 live mage Autocast.arm cells, frontend integration and whole-branch review. Not LIVE. Queued choose/spell/toggle is not a successful arm. Special and teleport stay later cards. No client, isolate schema, snapshot.rs, frontend, fixture or engine edits.
