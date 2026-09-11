# Spellbook teleport through actual magic controls

## Candidate

Task `t_1bf9a22e` implements brief 37 on `codex/rs2b0t-multirevision`. Client gitlink is `52c37f9ce50d1f184656d5b4469c007ec8a5791a`. No live client was launched. Root owns LIVE, frontend, scenario, ledger and repository integration.

## Facts and provenance

Generator `tools/game-data/generate.ts` now extracts additive `teleports` from selected `scripts/skill_magic/configs/magic_spells.dbrow` rows that carry `tele_coord`, plus `pack/interface.pack` `magic:<dest>_teleport` component ids, into schema-3 JSON (fields append; schema version unchanged). Runtime serde is `api::game_data` with `#[serde(default)]`. Target combat/enchant/alch rows are excluded because they have no `tele_coord`. JS never copies frozen `Teleport.ts` 2004 fallbacks or AIO `CONFIG.teleports`.

| Fact | 274 and 289 packed identity |
|---|---|
| Varrock | component 1164, Magic 25, Fire 1 + Air 3 + Law 1, xp 350, coord 0_50_53_13_32 → (3213, 3424, 0) |
| Lumbridge / Falador / Camelot | 1167 / 1170 / 1174 |
| Ardougne / Watchtower / Trollheim | 1540 / 1541 / 7455 |
| count | 7 standard spellbook teleports |

Second generation was byte-identical: 274 `6bee1e16c2949cc41b873143c240eb60196b4d2467a5b322174e75836056f2bf`, 289 `1a4f8bd9a091c7a9d896bee801388889af6b082088c05984d1e5e8ba2df3dc89`.

## `Game.teleport` API

Thin shim preserves the frozen call `teleport(name)` and boolean return. Missing selected teleport facts throw `not impl` synchronously (existing isolate remap proof). Unknown names and target-spell labels (`Wind Strike`) return false without a click. Posted Magic effective below the packed requirement refuses without pressing. Otherwise Rust returns the selected-cache component; JS dispatches one `if-button` and parks. Completion is posted Magic XP increase **and** a tile change. A queued button is not arrival. Timeout after 14 isolate polls returns false. Pause/Guardian hold freeze the parked wait. Stop/session reset bumps the token so the wait returns false and does not click again. Rune planning stays brief 27; this op does not walk and does not invent nav policy. Packed landing coordinates are generated facts only.

## Callers

AIO Teleport `Game.teleport(spellName)` (then counts Law). FireGiant `Game.teleport(TELE.name)` plus its own dungeon-exit wait. GreenDragon `Game.teleport('Varrock')` plus its own distance wait. Root owns LIVE.

## Hotspot

`crates/script/src/load.rs` — composition only: `__rs2b0t_teleport` register plus snapshot/Pause/Resume/ResetSession/hold hooks. `crates/api/src/game_data.rs` — serde append. No host-play, isolate schema, client, frontend, fixture or engine edits.

## Verification

Exclusive empty Cargo target `.superpowers/review-exports/spellbook-teleport-t_1bf9a22e-target` (not the shared campaign target). Isolated receipts after commit are in `docs/compat/evidence/teleport-capabilities/`.

- `bun tools/game-data/generate.test.ts`: pass (only `tele_coord` rows; high alch omitted).
- Two full generations: hashes above, second generation byte-identical.
- `bun tools/game-data/verify.ts`: both revisions 7 teleports, Varrock 1164/25/350.
- `cargo test -p api --test game_data`: 4 passed.
- `cargo test -p script --lib teleport`: 3 passed (filter also includes host_js Game.teleport exclusion).
- `cargo test -p script --test teleport`: 7 passed (missing facts throw, unknown/target false, low level refuse, queued button then XP+tile+remaining runes, Falador both caches, timeout, Pause/hold/session abort).
- `cargo test -p script --test load_isolate isolate_remaps_api_imports_to_our_game_and_teleport_throws`: 1 passed.
- `cargo clippy -p api --lib`, `-p script --lib`, `-p script --test teleport --no-deps -- -D warnings`: pass.
- `rustfmt --edition 2021 --check` on owned Rust: pass.
- `git diff --check` on owned paths: pass.

## Limits

Root owns 274/289 live AIO Teleport / FireGiant / GreenDragon tele-escape cells, frontend integration and whole-branch review. Not LIVE. A queued spellbook click is not a successful arrival. No client, isolate schema, frontend, fixture or engine edits. Shop / Make-X / fire remain later cards.
