# Special attack through selected-world weapon controls

## Candidate

Task `t_68de6f48` implements brief 36 on `codex/rs2b0t-multirevision`. Client gitlink is `9d090ed04957e4efc254f073cda97bc5510ca72b`. No live client was launched. Root owns LIVE, frontend, scenario, ledger and repository integration.

## Facts and provenance

Generator `tools/game-data/generate.ts` now extracts additive `special` controls from selected `pack/interface.pack`, `pack/varp.pack`, `pack/param.pack`, packed obj params, and `scripts/skill_combat/configs/combat.constant` into schema-3 JSON (fields append; schema version unchanged). Runtime serde is `api::game_data` with `#[serde(default)]`.

| Fact | 274 and 289 packed identity |
|---|---|
| `energy_varp` / `armed_varp` | `sa_energy` = 300 / `sa_attack` = 301 |
| `armed_value` / `max_energy` / `arm_confirm_ticks` | 1 / 1000 / 2 |
| bars | 14 combat `*:specbar` roots, including blunt 425→7462, stabsword 2276→7562, hacksword 2423→7587, polearm 8460→8481 |
| weapons | 12 `specwep`+`sa_energy` identities: dragon dagger 1215/1231 cost 250, magic short/longbow 350, rune thrownaxe 100, dragon halberd 300, … |

Staff layout 328 has no spec bar. Dragon battleaxe and Excalibur keep `specwep` without `sa_energy` and are omitted from cost. 289 param ids differ (`specwep` 139 vs 136); costs still come from each revision's packed params. Second generation was byte-identical. JS never copies `SPECBAR_BY_ROOT` or `SPEC_COST`.

## `Special` API

Thin shim preserves the frozen ABI. `energy`/`armed` still throw when varps 300/301 are unposted (no invented 0). `cost`/`ready`/`barComponent`/`arm` throw `not impl` when generated special facts are unavailable. Known weapons without a special return `null` cost and are not ready. `barComponent` is -1 when the combat tab is unreadable or has no specbar. `arm` is idempotent when already armed (`sa_attack==1`); otherwise one `if-button` on the posted bar and `Execution.delayUntilTicks` (2 ticks). A queued click is not completion.

Pause skips isolate ticks. Guardian hold skips the Execution pump. Stop/session reset bumps the special token so a parked wait returns false and does not click again. Melee/range `Game.combatStyleResolution` and Autocast are unchanged. Spell teleport stays a later card.

Host-play always posts varps 300 and 301, including 0, so empty energy / unarmed observation is not dropped by the nonzero/take-32 cap (108 remains reserved; remaining nonzero rows `take(29)`).

## Callers

GreenDragon / AutoFighter `useSpecial` `Special.ready` / `Special.arm`. Spellbook teleport remains separate.

## Hotspot

`crates/host-play/src/lib.rs` — always-post 300/301 beside existing varp 108. `crates/script/src/load.rs` — composition only: `__rs2b0t_special` register plus Pause/Resume/ResetSession/hold hooks. `crates/script/src/shim/mod.rs` — `content_json` special key after duel. `crates/api/src/game_data.rs` — serde append.

## Verification

Worktree diagnostic target `.superpowers/check-t_68de6f48` (not the shared campaign target). Isolated empty-target receipts after commit are in `docs/compat/evidence/special-capabilities/`.

- `npx tsx tools/game-data/generate.test.ts`: pass (fixture bars/costs omit battleaxe and staff).
- Two full generations: 274 `6ca4c04acd7f12f635b99c044009d24d0af09d1d23afb709e8f0e6ca5a634232`, 289 `f7c78bf2ac32b29c618cc5690c555495abd29f101d1e1eba7e382dc4b8f0422b` (second generation byte-identical).
- `npx tsx tools/game-data/verify.ts`: both revisions 14 bars, 12 weapons, 300/301/1000.
- `cargo test -p api --test game_data`: 4 passed.
- `cargo test -p script --lib special`: 2 passed.
- `cargo test -p script --test special`: 9 passed (missing facts throw, costs/ready/wielded/bar both caches, already armed, no staff bar, one bar click then posted armed, 2-tick timeout, weapon change, Pause/hold/session abort, unposted energy).
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored.
- `cargo test -p script --test load_isolate isolate_special_energy`: 2 passed.
- `cargo clippy -p api --lib`, `-p script --lib`, `-p script --test special`, `-p host-play --lib --no-deps -- -D warnings`: pass.
- `rustfmt --edition 2021 --check` on owned Rust: pass.

## Limits

Root owns 274/289 live GreenDragon special cells, frontend integration and whole-branch review. Not LIVE. A queued spec-bar click is not a successful spend. Spell teleport stays a later card. No client, isolate schema, frontend, fixture or engine edits.
