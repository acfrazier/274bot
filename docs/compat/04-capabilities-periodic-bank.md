# Periodic banking with observed deposit and return

## Candidate

Task `t_51452e47` implements brief 28 on `codex/rs2b0t-multirevision`. Client remains `56d80272bcbda3eb1e22db096c1c5e21d3497de4`. No live client was launched. Acceptance checks used a new empty Cargo target `target-t_51452e47` (not the shared campaign target).

## Service

Non-Off PeriodicBank is a small Rust-owned service (`crates/script/src/periodic_bank.rs`). The shim keeps the frozen class/options ABI and marshals settings plus script callbacks. Off construction and `validate() === false` still send nothing.

Rust parses labels `Off` / `Loot count` / `Time` / `Either` and also maps the shim token `loot` and the source token `items` onto the same Items strategy without treating those strings as equal. Due evaluation uses posted loot count, the items/time/either contract (`lootCount > 0`), combat suppression, and a Pause/Guardian-frozen 180000 ms failure backoff. Clocks are Instant-based with a generation/token; they are not game ticks.

Sequence, using existing family-1 operations (60_000 ms walk, 4_000 ms bank wait; not the foreign 120_000 ms router):

1. Walk supplied destination (exact stand) or nearest packed booth.
2. Named/nearest `open-booth` with loc identity; missing access fails closed.
3. Wait fresh `snapshotReady` (generation advanced).
4. Observed `depositAllMatching` with the caller's deposit/keep predicate and `commonJunk` (default true).
5. Script `afterDeposit`, acknowledged before the service continues.
6. Close.
7. Walk `returnTo` radius 6.

A queued open, deposit or walk is not completion. `PendingBankFetch` is untouched. NPC `npcAccess` stays explicit missing access. DeathRecovery is out of family.

Pause/Guardian hold freeze clocks and do not send. Stop, isolate drop and session replacement bump the run token so an old afterDeposit/close cannot land on a new operation.

## Callers

ChickenKiller (`parseBankStrategy` + `depositAllExcept` keep list + `afterDeposit` restock + null destination + return-to-anchor). Ardy/RockCrab pass `commonJunk` and optional destination tile. Tests include the ChickenKiller call shape and a RockCrab-shaped exact `bankTile`.

## Hotspot

`crates/script/src/load.rs` is a shared isolate register/lifecycle file. This card owns only `__rs2b0t_periodic_bank` registration and Pause/Resume/ResetSession/hold clock hooks in that file. Commit `41cf8eac` also landed `__rs2b0t_tile_distance` helpers/registration from concurrent brief 57 / `t_701d17f4` work; PeriodicBank does not call that path. Per orch (2026-09-10), those Tile helpers stay in the working tree for the Tile card and must not be stripped by this task. Root follow-up `922dac2481dfa42619c3edf74f36e06d93e908a4` separates those 68 lines from the committed PeriodicBank composition with a private index, preserving the working tree and ordinary index. Complete Tile work is separately committed at `6401d3a4`; it must not be removed from later working source. Review PeriodicBank at exact `922dac24`.

## Verification

First-round service checks used `target-t_51452e47`; the first Grok 4.5 review confirmed the results below but requested source-ownership correction. The interrupted `target-t_51452e47-corrective` attempt did not complete qualification. Corrective review 1211/session `20260910_231549_6a11a1` subsequently approved exact `922dac24` and repeated the checks in new empty `target-review-t_51452e47-r1789096618`. The completed review receipt is `evidence/periodic-bank-capabilities/review-round2.json`. Raw logs and the interrupted verification record remain preserved; original committed logs remain at `41cf8eac`.

- `cargo test -p script --lib periodic_bank`: 6 passed (label/token mapping, shouldBankNow, combat/Off, backoff + Pause freeze, dest no-fallback, npcAccess).
- `cargo test -p script --test periodic_bank`: 8 passed (Off zero sends, combat suppress, ChickenKiller loot-count deposit/afterDeposit/close/return, Either+shim `loot` token, exact destination, commonJunk true/false, missing-access backoff, Pause/session abort).
- `cargo test -p script --test gold_stubs`: 12 passed, 1 ignored (withdraw-X and already-adjacent bank access retained).
- `cargo clippy -p script --no-deps -- -D warnings`: pass.

## Limits

Root owns 274/289 live ChickenKiller/Ardy/RockCrab bank-cycle cells, frontend integration and whole-branch review. Queued open is not a successful bank trip. Foreign 120 s return timeout is not adopted. `shouldBankNow` JS still throws `not impl`; enabled callers go through PeriodicBank. No loadout, DeathRecovery, client, nav-router, LIVE or gate change.
