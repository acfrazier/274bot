# Selected-world spell facts and targeted inventory casting

## Candidate

Task `t_63138b8b` implements brief 27 on `codex/rs2b0t-multirevision`. Client remains `56d80272bcbda3eb1e22db096c1c5e21d3497de4`. No live client was launched. Acceptance checks used a new empty Cargo target `target-t_63138b8b` (not the shared campaign target). Frozen composition includes the already-landed use-on identity commit `a4157243` and later loadout-editor `50f2be8a`; this family did not need extra use-on source to compile.

## Facts and provenance

Generator `tools/game-data/generate.ts` now extracts additive `spells` and `staves` into schema-3 JSON (fields append; schema version unchanged). Runtime serde load is still `api::game_data` with `#[serde(default)]`.

| Table | Source | Shape |
|---|---|---|
| `spells` | `scripts/skill_combat/configs/magic/magic_combat_spells.dbrow` named rows with `continue_by_autocast=true` | 16 combat spells, `ssb` 0..15 in file order, level, rune display names/counts |
| `staves` | `scripts/skill_magic/configs/magic_staff.dbrow` aggregated by staff alias | 14 staves; lava provides Earth+Fire |

274/289 both yield Wind Strike..Fire Wave with identical ssb/level/rune names. 289 extra unnamed autocast rows are not SPELL_DB. Staff of fire / Fire battlestaff / Lava battlestaff / Mystic fire staff / Mystic lava staff all provide Fire rune. Staff of air does not.

Pinned inputs: 274 engine `4c95f87efe00b068cadbd229d94736626907bd1a` / content `000c19997e07206131bcb3c884265840efce416d`; 289 engine `cc359656b4acd216ca452495874b6beba9a0ac75` / content `92649430fcbc83538d8c4367ecb96cee1a67a944`. Second generation was byte-identical (`274.json` `a03e1a47…`, `289.json` `693b1d01…`).

Foreign `gen-spelldb.ts` is not runtime authority. SPELL_DB SETTINGS keys remain the static 16 names so origin parse can inline `Object.keys(SPELL_DB)`. Posted selected-revision rows fill `{ssb,level,runes}`. STAFF_RUNES is posted or `{}`.

`spellButtonCom` is `1830 + ssb` or `-1`. 1830 is the frozen staff_spells grid base used by mage SETTINGS/autocast checks; inventory-target casts use posted magic-tab `spell_buttons` (`button_type==2` + `target_base`). Posted buttons win for `Game.castOnItem`. No silent 274 ID fallback.

High alchemy / Superheat Item are not SPELL_DB rows. Callers open side tab 6, then `castOnItem` matches the posted target button. Rune costs for those spells stay in `magic_spells.dbrow` for a later family if needed.

## Rust helpers

Thin JS maps to `__rs2b0t_runes_per_cast` / `__rs2b0t_spell_button_com`. Unknown spell → `null` costs / `castsAvailable` 0 / com `-1`. Staff substitution drops provided runes. Empty remaining cost is `[]` (JS `+Infinity`); the 16 combat spells always keep a catalytic rune, so enabled callers never hit that branch on selected data.

## `Game.castOnItem`

Missing posted spell control throws `not impl`. Null/stale inventory returns false and does not queue. Matching posted button queues `use-widget-on` and returns true (dispatch accepted). Callers own the XP/item wait (Alcher `notesHeld`, Superheater 3 ticks). No second wait inside the op. Spellbook opening stays `Game.openSideTab(6)`. Pause/Guardian still freeze parked caller waits; Stop/session abort keep existing isolate abort. `castOnLoc` / `castOnNpc` unchanged.

## Callers

Alcher and Superheater inventory-target casts; mage `castsAvailable` / `runeWithdrawList` / `spellButtonCom`; later catalog Superheater `STAFF_RUNES` → `FIRE_STAVES` (Staff of fire first, then every fire provider). Autocast, teleport, hostile facts, shop, make menus and fire remain subsequent.

## Hotspot

`crates/script/src/load.rs` also carries loadout accessor registers and a concurrent inventory-row slot publish. This task only added the two spell helper registers.

## Verification

Isolated target: `target-t_63138b8b`. Raw logs: `docs/compat/evidence/spell-capabilities/`.

- `tsx tools/game-data/generate.test.ts`: pass (unnamed autocast rows excluded; lava aggregates Earth+Fire).
- Two full generations: byte-identical.
- `tsx tools/game-data/verify.ts`: 16 spells, 14 staves, five fire providers, Wind Strike / Fire Wave runes.
- `cargo test -p api --test game_data`: 4 passed.
- `cargo test -p script --test spell_facts`: 6 passed (STAFF_RUNES FIRE_STAVES, staff substitution, missing control, stale inv, Superheat queue + observed Magic XP/item).
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored.
- `cargo test -p script --test rs2b0t_registry parse_settings_inlines_shim_food_banking_and_spell_keys`: pass.
- `cargo test -p script --test load_isolate kernel_facade_game_cast_on_item_queues_use_widget_on`: pass.
- `cargo clippy -p api -p script --no-deps -- -D warnings`: pass.

Banking regressions retained via gold_stubs withdraw-X / already-adjacent bank access.

## Limits

Root owns 274/289 live Alcher/Superheater/mage acceptance, frontend integration and whole-branch review. Autocast arm, spellbook teleport, and high-alch/superheat rune tables in SPELL_DB are out of family. Queued `use-widget-on` is not a successful cast; isolate proof observes posted Magic XP and ore count after dispatch.
