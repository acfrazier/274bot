# Bank outcome corrective review

Reviewer: Hermes profile `reviewer`, model `grok-4.5`, provider `xai-oauth`
(profile default; no task model/provider override).
Date: 2026-09-10. Kanban card `t_d68ee0fc` (corrective gate after premature
`t_41b2f50e` completion without review).
Kind: same-card Grok 4.5 source review of banking matching/fill plus root
count-dialog correction. Round 1 artifact lens, then independent export checks.
Not LIVE acceptance, not brief-31 bank-loop implementation, not whole-branch
`branchreviewer`.

Read: `AGENTS.md`, `docs/execution.md`, briefs `22-bank-matching-fill.md` and
`32-bank-outcome-corrective-review.md`, report
`04-capabilities-bank-matching-fill.md`, `05a-catalog-live-harness.md` headed
Alcher notes, and committed evidence under
`docs/compat/evidence/bank-matching-fill/` plus untracked
`docs/compat/evidence/catalog-headed/count-dialog-*.log` receipts named by the
brief. Branch checked first: `codex/rs2b0t-multirevision` (not `main`). Work was
read-only on product source except this report. No product edits, stash/reset,
index changes, client gitlink movement, or live/native launches.

## Verdict

**APPROVE** host candidate `76d61beb2bc0b93216a43eeaa968c1304e4eabee` with
client pin `56d80272bcbda3eb1e22db096c1c5e21d3497de4` for the scoped banking
matching/fill path and the count-dialog local-dismissal correction.

This is **source approval** only. Root still owns 274/289 live acceptance,
frontend integration, planned off-scene bank routing / ordinary
`Bank.withdraw` result mapping (brief 31), and final whole-branch review.
The open Alcher post-stop interrupted-slow-tick lifecycle finding is **not**
resolved by this candidate and must not be treated as closed.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Review candidate HEAD (named) | `76d61beb2bc0b93216a43eeaa968c1304e4eabee` |
| Matching/fill source | `06077fe90c52d3ed1ed3b139a7c2283c31c84aad` |
| Vanished-row settlement fix | `23a30524a6af9b8d9e58f1aa0fbb98157469eedb` |
| Generated `index.d.ts` two-line refresh | `f2b04198`..`bf8452ec836155ea78632a9f25a82c44580a3ed3` (only `withdraw_load_result_seq` / `withdraw_load_result`) |
| Matching/fill report + raw receipts | `fa23cc60719c88fbebe13abbd15b7fc29f2c4ec4` |
| Count-dialog correction | included in `76d61beb` (`crates/api` only; no client change) |
| Client gitlink at `76d61beb` | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Reviewer model/provider | `grok-4.5` / `xai-oauth` |
| Isolated export used for checks | `.superpowers/task-exports/t_d68ee0fc-76d61beb` |
| Shared worktree HEAD during review | advanced concurrently to docs-only `d90ddb42` after inspection began; product candidate remained `76d61beb` and was reviewed via export, not moving HEAD |

Concurrent untracked catalog-headed evidence and the reviewer’s private
`target-review-t_d68ee0fc/` were left untouched as product WIP. Export checks
did not stash or restore shared files.

## Scope map (acceptance → evidence)

### 1. Rust-owned common-loot matching

- `api::content::COMMON_BANK_LOOT`, `RANDOM_EVENT_CASKET_ID = 405`, and
  `matches_common_bank_loot` live in Rust (`06077fe9`).
- Load isolate registers `__rs2b0t_matches_common_bank_loot`; host content
  publishes the list/id into `__rs2b0t_host.content`.
- Thin JS `matchesCommonBankLoot` / `depositMatcher` map only.
  `depositMatcher(own, includeCommon)` evaluates `own(name)` first, then the
  Rust arm only when `includeCommon` is true.
- `Bank.depositAllMatching` now passes `(name, id)` so the casket id arm can
  fire when the posted name is unrelated.
- Report’s dual-revision `405=casket` pack hashes are retained in
  `verification.json`; this review did not re-hash external content packs.

### 2. Observed `withdrawLoad` path

- JS: ready bank + positive named stock; full inventory returns true without
  send; else one `withdraw-load` request with bank generation; wait is host
  result / session change with `Execution.delayUntil(..., 0)` (no wall-clock
  timeout).
- Host: recomputes free slots, stock, and ops; selects exact `Withdraw N`, else
  `Withdraw All` when fill equals stock, else host-owned Withdraw-X dialog path
  (`fill_withdraw_action`).
- Pending fill baseline captures used slots, inventory count, and bank stock
  before send. Settlement requires inventory used/count growth **or** a
  still-present bank row with decreased stock (`23a30524`). A vanished bank row
  alone does not settle success.
- Snapshot schema appends `withdraw_load_result_seq` / `withdraw_load_result`
  (FlatBuffer + generated d.ts). Results post on the withdrawLoad channel;
  rejected/stale/competing requests complete false.

### 3. Parent Pause / hold / reset correction

- Withdraw-X and withdrawLoad share pending machinery with typed result
  channel (`PendingWithdrawResult`).
- JS outer 8000 ms race removed (`timeoutAt: null` when timeoutMs is 0).
- Pause/hold freezes monotonic deadlines; resume restores remaining time.
- `on_is_up(false)` / `reset_session_work` complete the current withdrawal as
  false before clear, including generation-reuse cases; Stop cannot publish a
  late result into a new isolate lifetime.
- Composed host test advances a controlled isolate clock past the former 8000
  ms bound under Guardian hold, resumes without outcome (still pending), then
  dialog + inventory complete success.

### 4. Root count-dialog correction (`76d61beb`)

- Headed Alcher at frozen `f2b04198` withdrew 27 noted chainbodies then refused
  Nature runes while 200 remained visible; amount prompt stayed open.
- Cause: packet writer sent `RESUME_P_COUNTDIALOG` without the client keyboard
  path’s local dismissal (`dialog_input_open = false`, `redraw_chat = true` at
  client `client.rs` enter-amount handler).
- Fix: `Driver::count_dialog_submitted` default no-op for recorders; real
  `Client` clears the prompt and requests chat redraw; free `answer_count`
  invokes it only after a successful write. Refused invalid counts preserve the
  prompt.
- Regression covers both revisions’ opcodes and a later server re-open.
- No client packet/source change. Interrupted-slow-tick after ScriptRunner.stop
  remains an open lifecycle finding outside this patch.

## Independent checks (exact export)

Commands run under
`.superpowers/task-exports/t_d68ee0fc-76d61beb` with
`CARGO_TARGET_DIR=.../target-review-t_d68ee0fc` (separate from shared WIP).

| Check | Outcome |
|---|---|
| `cargo test -p api --lib common_bank_loot` | pass (1) |
| `cargo test -p api --test interact answer_count_closes_real_client_prompt_before_next_withdrawal` | pass |
| `cargo test -p api --test interact` | 65 passed |
| `cargo test -p script --test load_isolate isolate_common_bank_loot_uses_rust_predicate_and_preserves_short_circuit` | pass |
| `cargo test -p script --test load_isolate isolate_bank_withdraw_load` | 2 passed |
| `cargo test -p script --test load_isolate isolate_bank` | 17 passed |
| `cargo test -p script --lib pending_withdraw` | pass (pause freeze + reset abort) |
| `cargo test -p host-play --lib withdraw_load` | 2 passed (selection + vanished-row then fill) |
| `cargo test -p host-play --lib withdraw_x_composes` | pass (clock beyond former JS bound + hold) |
| `cargo test -p host-play --lib` | 122 passed |
| `cargo test -p script --test gold_stubs withdraw` | pass |
| `cargo test -p script --test host_js` | 2 passed, 1 ignored (regen) |
| `cargo clippy -p api --all-targets --no-deps -- -D warnings` | pass |
| `cargo clippy -p script --all-targets --no-deps -- -D warnings` | pass |
| `cargo clippy -p host-play --lib --no-deps -- -D warnings` | pass |

Committed implementer receipts under `evidence/bank-matching-fill/` and
untracked `count-dialog-final.log` (65 interact tests) align with the export
re-run. LIVE was not run (brief/root owned).

## Findings

### Blocking

None.

### Non-blocking / residual (do not reopen this card)

1. **Open lifecycle:** repeated interrupted-slow-tick messages after Alcher
   ScriptRunner.stop remain unresolved; count-dialog does not close that item.
2. **Live acceptance deferred:** no corrected headed Alcher or catalog matrix
   cell was required or run for this source gate.
3. **Brief 31 remains downstream:** nearest-bank travel, ordinary
   `Bank.withdraw` boolean mapping, and full BoneBurier bank loop are not
   claimed here. Concurrent `bf8452ec` BoneBurier/scenario docs beyond the two
   generated d.ts lines were not this candidate’s product surface.
4. **Shared branch motion:** during review the shared HEAD gained docs-only
   `d90ddb42` (brief 31 note). Candidate product remains `76d61beb`; dependents
   must not treat later docs commits as part of this approval range.

## Conclusion

The premature `t_41b2f50e` close left root acceptance without an independent
matching/fill verdict. This corrective review inspected the named commits cold,
verified the isolate → Rust capability → posted-result path, the vanished-row
settlement rule, Pause/hold/reset completion, and the real-client count-prompt
dismissal against the client keyboard path, then re-ran focused export checks
green. **Approve `76d61beb` / client `56d8027` for this scoped source gate.**
