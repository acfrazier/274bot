# Integrated world, first banking and frontend session review

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 01:30 UTC. Kanban card `t_dde220ef`, run 1180.
Kind: independent Grok 4.6 coherent integration review of combined world,
first-banking, matching/fill, routing, frontend-session, 289 permit, catalog
harness, script Stop, panel startup, selected metadata, loading progress and
first-run Browse. Not a same-card Grok 4.5 task review, not LIVE/native
acceptance, not all-card/all-frontend qualification, not the final
whole-campaign `branchreviewer` pass.

Read once: `AGENTS.md`, `docs/execution.md`, brief
`docs/compat/briefs/30-world-banking-frontend-integration-review.md`,
`02-host-boundary.md`, `03-world-capabilities.md`,
`04-capabilities-banking.md`, `04-capabilities-bank-matching-fill.md`,
`04-capabilities-bank-routing.md`, `05a-catalog-live-harness.md`,
`06a-frontend-session-observations.md`, `06d-panel-startup-preparation.md`,
`06e-panel-startup-review.md`, `06f-panel-startup-native-proof.md`,
`04-item-metadata.md`, `04a-item-metadata-review.md`,
`evidence/browse-ux/README.md`, and
`evidence/catalog-harness/concurrent-restore-audit.json`.
Branch checked first: `codex/rs2b0t-multirevision` (not `main`).

Work was read-only except this report. No product edits, stash/reset/checkout,
index changes, client gitlink movement, live/native launches, new agents,
merge, remotes or release work. Shared dirty files were left untouched.
Checks used a separate `git archive` of committed host HEAD plus the exact
gitlink client, with a private `CARGO_TARGET_DIR`. The earlier shared-source
restore method was not repeated.

## Verdict

**APPROVE the integrated source candidate** at frozen host
`1e17c07a98a41f34f7c167005f1ba6a99e46aba4` with reviewed client pin
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`.

This is **source approval** of the combined host/world/banking/frontend/startup
seam after the named parent reviews. It is **not** live acceptance, not native
Mac/Windows/Linux frontend acceptance, not all 180 ledger rows/options, and
not whole-branch review. Root still owns those actions.

The named Pause/hold competing-clock counterexample is closed on this
candidate. Script-requested Stop no longer leaves a live isolate for later
ticks. 289 operations are permitted only after the selected host/world proofs
already recorded; public fixture cheats and cache-mismatch metadata remain
fail-closed.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Frozen host candidate | `1e17c07a98a41f34f7c167005f1ba6a99e46aba4` |
| Client gitlink at freeze and at report time | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Shared HEAD when inspection began | `1e17c07a98a41f34f7c167005f1ba6a99e46aba4` |
| Shared HEAD at report time | `6d585fd92daa7a14003aa7d22871ece0790d95af` |
| Brief SHA-256 | `eb7e969907cc3adb34dd5fb6d1c1f515de9d0a41e2dc65b159ef943913299296` |
| Export | `/tmp/t_dde220ef-1e17c07a-export` |
| Private target dir | `/tmp/t_dde220ef-target` |
| Reviewer model/provider | `grok-4.6` / `xai-oauth` |

Confirmed: `git branch --show-current` = `codex/rs2b0t-multirevision`.
`git ls-tree 1e17c07a vendor/fr-client-rust` is `56d80272`. Client
`56d80272` subject is `Track inventory freshness by component`.

### Named product commits inside the freeze

| Lane | Commit | Subject |
|---|---|---|
| Host action/session | `fede3c0dd5d5fbdc7fd7def10f2eb43630b8f16f` | Restore retained scene collision after reconnect reset |
| Selected-world nav | `807207846b93637c830fc05781290138326870d8` | Bind navigation bakes to selected world inputs |
| First-bank host | `8a60eef77e9c4e74c3ee81332e495fb9331c046a` | Make banking freshness and Withdraw-X bounded |
| Named bank / Withdraw-X | `75514ee38874f4b394ce1748d354ae936f6b86b9` | Complete named bank open and Withdraw-X outcomes |
| Matching/fill | `06077fe9` / `23a30524` / `fa23cc60` | Observed fill, vanished-row refusal, report |
| Count-dialog | `76d61beb2bc0b93216a43eeaa968c1304e4eabee` | Dismiss submitted count prompts |
| Bank routing | `2f9099f703e243a00713d99a63e93afeb83a8408` | Complete Bone Burier bank loop |
| Frontend session | `5eeff8f0` / `c9521d2f` | Clear frontend state at session boundaries |
| 289 permit | `87084cbc0b98b9a5649a9cae5bc45e588c17d07a` | Permit revision 289 after host/world qualification |
| Catalog harness | `78c8cf21ec27eae4cd88023fc0df2ef3e1dfb1ad` | Frozen catalog production live harness |
| Actor display names | `c933f37c5bec108d752e28aa850c46bccd405d3b` | Compare catalog baseline actors using client display names |
| Script Stop | `1947741fb3326ce187cf7ca44f126345c4100155` | Propagate script-requested Stop through host slot cleanup |
| Panel startup | `814e5293fec426b722e450580c514b9cffca826f` | Prepare startup resources off UI thread |
| Selected metadata | `7852b5a053fd27e292221be78ea0783e805e3454` | Load selected-revision game metadata |
| BankFletcher witness | `6c6bb2d5c25d51f5b339d6bb5f73d68b3b7296cb` | Require full bank-fletching loop |
| Schema-3 generator | `91769217` / `bba5179c` / `2530891b` | Consumption/thieving facts + hash refresh |
| Loading progress | `79f86973` / `0bfb28ad` / `d321908c` / `30193c3b` | Classic text bar and monotonic stages |
| First-run Browse | `1a2f2dcf047de80e8793cb430c8044134163c7ac` | Explain and foreground the first-run script picker |
| Alcher ordered negative | `41d7a1e3dec688a538e49ae3dd801a216d55917d` | Require both ordered items in witness |

### Concurrent motion after freeze (not reviewed)

Shared HEAD advanced to `6d585fd9` while this export was in use. Later
commits were **not** in the archive and are **not** part of this approval:

- `5f86dc87` custom-item Alcher fixtures
- `d4f1809e` / `cd393879` seed-bank `Use-quickly` helper in `crates/scenario/src/lib.rs`
- `677dd383` / `6d585fd9` navigation-validation design docs

Uncommitted/untracked catalog-headed evidence and
`docs/compat/evidence/catalog-harness/core-results.json` were left untouched.

## Prerequisite actual-model reviews

| Lane | Card | Receipt |
|---|---|---|
| Matching/fill + count-dialog | `t_d68ee0fc` | Grok 4.5 approved `76d61beb` / client `56d8027` |
| Bank routing | `t_90b60f12` | Grok 4.5 approved `2f9099f7` |
| Frontend session | `t_887b29b5` | Grok 4.5 approved `5eeff8f0` / `c9521d2f` |
| Catalog harness | `t_14db340e` | Grok 4.5 approved `78c8cf21` |
| Alcher ordered witness | `t_a384fe64` | Grok 4.5 approved `41d7a1e3` |
| Panel startup source | `t_4bd3fe69` | Grok 4.5 approved `814e5293`; report `8c10ac3a` |
| Selected metadata | `t_e6ec9679` | Grok 4.5 approved `7852b5a0`; report `8c6689c1` |
| Loading progress + Browse | `t_42263bfd` | Grok 4.5 approved `79f86973`+`0bfb28ad`+`d321908c` and picker `1a2f2dcf` |

Same-card parent verdicts were treated as claims. This pass inspected the
combined source and re-ran focused export checks.

## Combined boundary

### Pause / hold competing clock (required counterexample)

`75514ee3` added an isolate `delayUntil` around Withdraw-X. Root found an
8000 ms wall clock that kept advancing under Pause/hold while Rust froze.

On this candidate the JS wait is `Execution.delayUntil(..., 0)` for
`withdrawX`, `withdrawLoad`, and ordinary `bankOp`
(`crates/script/src/shim/bank.js`). `timeoutMs > 0` is the only path that
sets `timeoutAt`; zero yields `timeoutAt: null`
(`crates/script/src/shim/execution.js`).

Host pending records freeze monotonic deadlines on Pause/Guardian hold and
resume remaining time (`PendingWithdrawX` 3000 ms dialog / 4000 ms
settlement; `PendingBankOp` 2000 ms deposit / 4000 ms withdraw in
`crates/script/src/slot.rs`). `script_observe` freezes before expiry.

Composed test `withdraw_x_composes_script_host_dialog_and_posted_inventory_result`
advances isolate `performance.now` to 9001 under Guardian hold, asserts the
promise stays pending, then completes after dialog + inventory. Session reset
posts explicit false, including generation reuse
(`pending_withdraw_x_pause_freezes_while_stop_and_reconnect_abort`).

The named counterexample is closed.

### Freshness, named identity, Withdraw-X bounds

Client `56d80272` tracks inventory freshness per component. Snapshot
`rebuild_bank` bumps `bank_session_generation` on modal generation delta or
component identity change, and sets `bank_loaded` only when the withdraw
component has a transmitting full observation newer than the modal's
`closed_observation` (`crates/api/src/snapshot.rs`). Empty vs loaded, close
then reopen in one drain, and wrong/stopped containers therefore cannot keep
the previous session identity.

Named booth dispatch preserves requested name/op; a missing op does not fall
back (`dispatch_script_interact` OpenBooth named arm). Stale generation
refuses settlement.

### Matching / fill and routing

Rust owns `COMMON_BANK_LOOT` / casket id 405; JS `depositMatcher` still
short-circuits on the caller predicate. `withdrawLoad` selects exact amount,
then eligible All, then host-owned Withdraw-X. Settlement requires inventory
growth or a still-present decreased bank row; a vanished row alone cannot
succeed (`23a30524`).

`Bank.withdraw` / `withdrawById` return the `bankOp` promise (no
unconditional true). `Bank.openNearestWorld` walks via `walk-nearest-bank`.
`nearest_bank_booth` prefers same-plane packed `Booth` stands and excludes
NPC stands. Ordinary withdraw false is posted to the isolate.

BoneBurier core witness rejects first-burial-only and inventory rise without
an observed bank transfer. BankFletcher requires first-pack deposit, fresh
same-generation withdrawal, then another product/XP (`6c6bb2d5`); first
product or empty stock does not qualify.

### Frontend session ownership

`Host::publish_frontend_snapshot` reuses `Pump::drain_client` +
`publish_snapshot`. Panel/TUI publish before local-player/Guardian returns.
`session_boundary` (`session_changed || !ingame`) clears external `WalkArm`
(including `BankBudget`) and the tick latch. Panel spawn+stop and TUI spawn
call `reset_frontend_slot_lifetime` so same-name reuse cannot inherit the
previous cursor. Scene rebuild and Guardian hold do not clear armed work.

### 289 permit and selected-world proof

`require_bot_operation` now returns `Ok` for `R274 | R289` after the
recorded host/world cells (`87084cbc`). Unsupported future revisions remain
an exhaustive match. Resource mismatches still fail bind/validate. Public
fixture cheats still refuse after mutable config poke
(`bound_public_client_refuses_fixture_cheats_after_mutable_config_changes`).
Synthetic caches do not receive selected 289 game data. Script Start/load
stay revision-agnostic; the unarmed 289 slot test starts a native tick card
without login.

Eight scoped world cells remain hashed receipts in `03-world-capabilities.md`
(nav_full, door, lamp Guardian, fixed-transfer bank-return on both
revisions). Bank-return used fixed transfers and does not qualify Withdraw-X
or named shim options. This review did not relaunch those cells.

### Catalog harness, Stop, combat style

`catalog_boundary_live` uses production `Play` / `ScriptStartHandle`, frozen
catalog identity, `ScenarioRunner::with_world(template.world())`,
`engine_speed_ms = None`, post-Start baselines and independent core deltas.
Actor baseline comparison uses client display names (`c933f37c`).
`Game.combatStyleResolution` returns
`{ requested, effective, mode, label }`; `describeCombatStyle` reads
`resolution.requested`. Broader style fallback remains later combat work.

Script-requested Stop: isolate sends `ThreadMsg::Stopped` after the completed
tick, logs `script requested stop on tick N; isolate stopping`, and breaks.
`SlotScript::drain_logs` folds that terminal state and calls `stop()`
(Idle, drop isolate, bump `work_epoch`, retain diagnostics) **before**
`script_observe` advances pending bank continuations. Later ticks on a
stopped isolate produce no slow-tick logs
(`isolate_script_runner_stop_stops_the_isolate`). Committed export receipts:
`evidence/catalog-headed/stop-source.json` (host base `b4d44e47`, client
`56d80272`), `stop-slot-export.log`, `stop-isolate-export.log`.

### Panel startup, loading, Browse, metadata

Prepare (bind/load) and final `validate_for_play_with_progress` run on
sequential detached workers. The UI thread consumes a private-field
`ValidatedTemplate` immediately before Play assembly. Stale generation drops
without installing vault/Play/slot. Loading UI is a 20-cell `[#...-]` bar
with `theme::ACCENT` on description/filled cells; partial work cannot print
100% (`loading_text_fits_the_rail_and_never_rounds_partial_work_to_complete`).
Browse opens the file dialog after the browse window so the picker is
foreground; purpose text and `&=` close/`Not now` are present.

Selected metadata: schema-3 JSON is deserialized once per revision into a
process-static `Arc`. `for_optional_profile` publishes only when the
immutable cache identity matches; otherwise empty. `content_json` is
evaluated before `load_modules`. Handwritten `FOOD_HEALS` / pickpocket tables
are gone.

## Independent checks (exact export)

All commands ran under `/tmp/t_dde220ef-1e17c07a-export` with
`CARGO_TARGET_DIR=/tmp/t_dde220ef-target`. Shared WIP was not restored.

| Check | Outcome |
|---|---|
| `cargo test -p script --lib pause_freezes` | 2 passed |
| `cargo test -p script --lib script_requested_stop` | 1 passed |
| `cargo test -p script --test load_isolate isolate_script_runner_stop_stops_the_isolate` | 1 passed |
| `cargo test -p script --test load_isolate isolate_bank_withdraw_load` | 2 passed |
| `cargo test -p script --test load_isolate isolate_common_bank_loot` | 1 passed |
| `cargo test -p host-play --lib --features memory-profile withdraw_x_composes` | 1 passed |
| `cargo test -p host-play --lib --features memory-profile nearest_bank_booth` | 1 passed |
| `cargo test -p host-play --lib --features memory-profile withdraw_load` | 2 passed |
| `cargo test -p host-play --lib --features memory-profile deposit_and_ordinary_withdraw` | 1 passed |
| `cargo test -p host-play --lib --features memory-profile ordinary_withdraw_refusal` | 1 passed |
| `cargo test -p host-play --test session_profile --features memory-profile qualified_revision_289_accepts_an_unarmed_slot` | 1 passed |
| `cargo test -p host-play --test session_profile --features memory-profile bound_public_client_refuses_fixture_cheats` | 1 passed |
| `cargo test -p host-play --test catalog_boundary_live --features memory-profile bone_burier_rejects_first_burial alcher_option_witnesses` | 2 passed |
| `cargo test -p host-play --test catalog_boundary_live --features memory-profile bank_fletcher` | 3 passed |
| `cargo test -p panel --lib frontend_logout / frontend_response_15 / same_name_lifetime` | 3 passed |
| `cargo test -p panel --lib normal_unlock_waits_for_worker_validation` | 1 passed |
| `cargo test -p panel --lib stale_profile_preparation_is_dropped` | 1 passed |
| `cargo test -p panel --lib loading_text_fits_the_rail` | 1 passed |
| `cargo test -p panel --lib load_then_browse_still_prompts_without_rs2b0t_root` | 1 passed |
| `cargo test -p tui --lib frontend_logout / frontend_response_15 / same_name_lifetime` | 3 passed |
| `cargo test -p api --lib common_bank_loot` | 1 passed |

LIVE was not run. Broad crate suites were not rerun solely to restate
receipts.

### Export file hashes (frozen tree)

| Path | SHA-256 |
|---|---|
| `crates/script/src/shim/bank.js` | `31d0380098d9c9d1b759447ce832461c3dd9b52069ceb9b6bd96d5181cfb1aeb` |
| `crates/script/src/shim/execution.js` | `6395e4b43cdd8cccf079a82a3eef02b8228ee0e042f55dfef197c40b70a0ee05` |
| `crates/script/src/slot.rs` | `3fb015e4d019605edd8ad3d67ab8a42f2c4320cda25811eced9e01182e7ad599` |
| `crates/host-play/src/profile.rs` | `36dd3455d49315e33883c8aae4beb29b422f15d1a484ef4d9185674bbe41a92d` |
| `docs/compat/evidence/catalog-headed/stop-source.json` | `2d9f5d10c7cd5a929a79581b4d20fa164900eec529fd77a552a280da24cdf2a3` |
| `docs/compat/evidence/catalog-headed/stop-slot-export.log` | `d11988dc601ad41fdc10e625df9b0c90e80f9b5c16fdb0a4e1605bb7f19cd505` |
| `docs/compat/evidence/catalog-headed/stop-isolate-export.log` | `2e632c96b5d62941498a57b5ba87d6d150a813c91565ebf8ea181c8348ea4b1b` |
| `docs/compat/evidence/catalog-harness/concurrent-restore-audit.json` | `c2f4dc4a44d6cdd762185fdb6d2911c2cc602af5c9da17ac88e18b76255111a1` |

## Findings

### Blocking

None.

### Non-blocking / residual

1. **LIVE and native proof remain root-owned.** Eight world cells, headed
   catalog loops, Mac/Windows startup captures, and screenshot-vs-gameplay
   separation are not re-accepted here. Ledger rows stay PARTIAL until
   supported settings and integrated/frontend proofs complete.
2. **Remaining isolate wall-clock waits.** `Bank.close` (3000 ms),
   `waitReady` (5000 ms), travel `delayUntil` (120 s / 60 s) and the 1200 ms
   empty `bank_side` wait still use `performance.now`. They are skipped while
   the pump is held, but can expire immediately on resume if pause outlasts
   the timeout. They are not the named 8000 ms Withdraw-X counterexample.
3. **TUI has no `stop_slot` lifetime reset.** Same-name reset is on spawn
   only (existing TUI surface).
4. **Operator Stop drops pending withdrawals without posting.** The isolate
   is joined, so JS cannot observe a late result. Session replacement posts
   false; that is the generation-reuse path the brief requires.
5. **Concurrent HEAD motion.** `d4f1809e` seed-bank helper and `5f86dc87`
   custom Alcher fixtures landed after freeze. Dependents must not treat them
   as covered by this approval.
6. **`for_profile` mismatch label order** remains slightly inverted
   (already recorded on `t_e6ec9679`); optional-profile publication is the
   production path and is fail-closed on cache mismatch.

## Conclusion

The combined candidate at `1e17c07a` / client `56d8027` closes the
Pause/hold Withdraw-X clock, posts abort across session replacement including
generation reuse, routes nearest packed booths with honest withdraw results,
clears frontend travellers at logout/reconnect without dropping Guardian/scene
work, permits 289 only after the selected proofs, reaps script Stop before
host bank continuations, keeps final validation off the UI thread, and
publishes selected-revision metadata once before module evaluation. Focused
export checks passed. **Approve this frozen source for the bounded
integration gate.** Root still owns live/native actions and the final Grok
4.6 whole-branch review.
