# Hostile attacker and duel interface observations

## Candidate

Task `t_eeea20f0` implements brief 35 on `codex/rs2b0t-multirevision`. Client gitlink is `9d090ed04957e4efc254f073cda97bc5510ca72b`. No live client was launched. Root owns LIVE, frontend, scenario, ledger and repository integration.

## Facts and provenance

Generator `tools/game-data/generate.ts` extracts additive `duel` controls from selected `pack/interface.pack` into schema-3 JSON (fields append; schema version unchanged). Runtime serde is `api::game_data` with `#[serde(default)]`. Hostile names stay host-owned in `api::content` and are not a generated table.

| Fact | 274 and 289 packed identity |
|---|---|
| `select_modal` | `duel_select_type` = 6575 |
| `confirm_modal` | `duel_confirm` = 6412 |
| `win_modal` | `duel_win` = 6733 |
| `select_accept` | `duel_select_type:accept` = 6674 |
| `confirm_accept` | `duel_confirm:accept` = 6520 |
| `select_partner` | `duel_select_type:otherplayer` = 6671 |
| `select_status` | `duel_select_type:status` = 6684 |
| `confirm_status` | `duel_confirm:status` = 6571 |

Those IDs are audit inputs verified on both selected caches, not shim constants. JS reads them through `__rs2b0t_host.content.duel`. `reader.ifText` looks up currently posted selected-world widget text; an absent id is null, never a stale IfType label.

## Observations

`Game.attackedByPlayer` is the local player's `face_entity >= 32768`. Host-play posts `attacked_by_player` from `LocalPlayerView`. A later NPC or none face does not keep the previous true.

`isHostileAttacker` is the Rust predicate over posted NPC rows: exact display name in {Guard, Knight of Ardougne, Paladin, Hero}, in combat, not targeting another player, distance ≤ max, and Attack present. `HOSTILE_NAMES` stays `[]`. The thin shim calls `__rs2b0t_is_hostile_attacker` with Npc getters/methods (`inCombat`, `targetsAnotherPlayer()`, `distance()`, `actions()`). Missing distance or maxDistance is false. Disabled Fight/Flee branches that iterate `HOSTILE_NAMES` stay inert.

`reader.ifText` returns posted selected-world text for that component id. Closing the modal posts an empty widget list; previous partner/waiting labels do not remain. Challenge/Fight stay the existing host-owned player ops. A seeded modal is not a duel.

Guardian hold still posts the facts but freezes `loop()`. There is no parked wait on these observations.

## Callers

GreenDragon wilderness `Game.attackedByPlayer`. ArdyCakes / ArdyThiever `guardResponse=Fight` via `isHostileAttacker`. Duel Arena Combat Trainer partner/waiting `reader.ifText` plus posted modal IDs. Special attack and spellbook teleport remain separate.

## Hotspot

`crates/script/src/load.rs` also carries death-recovery, periodic-bank and autocast registers. This task only added `__rs2b0t_is_hostile_attacker` and snapshot apply for `attacked_by_player`/`widgets`. `crates/host-play/src/lib.rs` gained local-face posting, selected-world widget text, and one face decode test in the same runtime file as loc/inv-button/autocast arms. `crates/script/src/isolate_fb.rs` appends two Snapshot fields; sibling test `base_snapshot` literals only gained the new defaults.

## Verification

Worktree diagnostic target `target-t_eeea20f0-check` (not the shared campaign target). Isolated empty-target receipts after commit are in `docs/compat/evidence/hostile-duel/`.

- `npx tsx tools/game-data/generate.test.ts`: pass (pack parse + extracted IDs).
- Two full generations: 274 `d310498c02aff231d4bd3a45095c9bb5d89007f899ada70f7b02863410b322c1`, 289 `9e7336807bd8756d86dc52568eb009520696b1b3ba4ac7066b9c997c6ad8f380`.
- `npx tsx tools/game-data/verify.ts`: both revisions 16 spells, 14 staves, duel 6575/6412/6733/6674/6520/6671/6684/6571.
- `cargo test -p api --lib content::tests::hostile_attacker`: 1 passed.
- `cargo test -p api --test game_data`: 4 passed.
- `cargo test -p script --test hostile_duel`: 6 passed (face latch, Guard/other/Man/case/radius/action/stale, selected ifText, hold freeze, both-cache IDs, missing max/distance).
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored.
- `cargo test -p script --test host_js`: 2 passed, 1 ignored.
- `cargo test -p host-play --lib script_snapshot_posts_attacked_by_player`: 1 passed.
- `cargo test -p host-play --lib script_snapshot_fb_carries_observed_fields_only`: 1 passed.
- `cargo clippy -p api --no-deps --all-targets -- -D warnings`: pass.
- `cargo clippy -p script --no-deps --all-targets -- -D warnings`: pass.
- `cargo clippy -p host-play --no-deps -- -D warnings`: pass.

## Limits

Root owns 274/289 live GreenDragon / Ardy Fight / Duel Arena cells, frontend integration and whole-branch review. Not LIVE. A queued Challenge or a seeded modal is not a successful duel. Special and teleport stay later cards. No client, frontend, fixture or engine edits.
