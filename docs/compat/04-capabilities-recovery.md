# Observed death recovery and script callbacks

## Candidate

Task `t_0c5ce97f` implements brief 29 on `codex/rs2b0t-multirevision`. Acceptance used the root-provided exact export `.superpowers/review-exports/death-recovery-root-8fd0f138` (host `8fd0f13870dbe7d207c1604726937ca36963d5aa`, client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`) plus the five owned overlay files. Overlay sha256 matched the sibling manifest. Checks used a new empty sibling target `death-recovery-root-8fd0f138-target`. No live client was launched.

Earlier `target-t_0c5ce97f` logs were shared working-source diagnostics. The `/tmp/t_0c5ce97f-src-r1233` checkout-index freeze against later HEAD `4d7e87b4` is also diagnostic, not the acceptance tree.

## Service

DeathRecovery is a small Rust-owned service (`crates/script/src/death_recovery.rs`). The shim keeps the frozen `new DeathRecovery(bot, opts)` ABI and marshals settings plus script callbacks. `validate()` stays false until a *new* posted game-chat death line; idle construction still sends nothing.

Rust latches `/oh dear.*you are dead/i` from posted ordered `chat_lines` sequences. The first observe baselines the current ring without latching, so seeded flags, duplicate snapshots and old chat cannot re-trigger. A later higher sequence matching the death line latches once. `needs` / `AcquireTask` throw `not impl` at construction; no enabled caller passes them.

Sequence, using existing host walk (60_000 ms) and Execution waits (Pause/Guardian skip the pump):

1. Wait up to 20_000 ms for `ingame && tile` (timeout still advances, matching the frozen execute).
2. Wait three actual game ticks.
3. If `walkBack` is supplied (WildyAgility `recoverAndReturn`, also HillGiant `travelToPit`), invoke that script callback; callback completion is recovery.
4. Else queue existing `walk-near` to the configured anchor/radius and recover only when the posted tile is actually within radius.

A queued walk or a mocked initial anchor is not completion. `onDeath` / `onRecovered` remain JS and fire once per observed event.

Pause/Guardian hold freeze clocks and do not send. Stop, isolate drop and session replacement bump the run token and drop the latch. Logout/reconnect clears the client chat ring (`chat_text` emptied on cold login) and `reset_session` discards in-flight recovery so leftover chat cannot resume old work. Ordinary death keeps `ingame` false only for the respawn wait; the latch is not aborted.

## Callers

Enabled frozen constructors: ChickenKiller r3, RockCrab r4, Ardy/AutoFighter/MossGiant/GreenDragon/ArdyCakes r6, WildyAgility `walkBack` to ridge. Dim RoguesPurse `walkBack` stays deferred; the ABI still invokes a supplied callback rather than a foreign planner.

## Hotspot

`crates/script/src/load.rs` is a shared isolate register/lifecycle file. This card owns only `__rs2b0t_death_recovery` registration and Pause/Resume/ResetSession/hold clock hooks in that file. Concurrent Flax adapter work owns `client_adapter.js` plus `tests/client_adapter_local_walk.rs`; those paths were not committed here. Concurrent brief 70 owns `crates/api/src/snapshot.rs`; this card did not edit it. Ordered chat already posts `chat_lines` sequences.

## Verification

Exact export `.superpowers/review-exports/death-recovery-root-8fd0f138` and empty target `death-recovery-root-8fd0f138-target`. Raw logs in `docs/compat/evidence/recovery-capabilities/`. Overlay hashes:

- `crates/script/src/death_recovery.rs` `482a8a6a81a74906485cb0f989f5d7fa459bb53bcf5ca7ef4e240c30517a5b1f`
- `crates/script/src/lib.rs` `ae162e323c892cc4719baf13c5c4f79f8530cdbea2e54db10dcd6013b7a31d33`
- `crates/script/src/load.rs` `a5f76b3488f59e85c29ec9872a4531d796a1478d6af634c220583e97ac807fa6`
- `crates/script/src/shim/death_recovery.js` `4e735ed26a0300a41bedd1259eb0b653913457327c211fbee2e4fd407383541c`
- `crates/script/tests/death_recovery.rs` `6174ae06f65c4ae77f731ec1b85e6940435fe642cbed32a366e56758a496d4a3`

- `cargo test -p script --lib death_recovery`: 6 passed (bounds, seeded/duplicate/old chat, wait+walk+once recovered, walkBack, Pause/reset stale token, respawn timeout still advances).
- `cargo test -p script --test death_recovery`: 7 passed (idle zero sends, seeded/duplicate/old chat, ChickenKiller-shaped new death through script ABI to observed walk then actual-position recover once, WildyAgility walkBack, needs unsupported, Pause/session abort, Guardian hold).
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored.
- `cargo clippy -p script --no-deps -- -D warnings`: pass.

## Limits

Root owns LIVE death cells, frontend integration and whole-branch review. Queued walk is not a successful return. Foreign AcquireTask is not implemented. No client, nav-router, snapshot.rs, client_adapter.js, LIVE or gate change.
