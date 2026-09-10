# Bank matching and observed fill capability

## Candidate

Task `t_41b2f50e` is implemented by source commits `06077fe9` and `23a30524` on `codex/rs2b0t-multirevision`. The generated `crates/script/host-js/index.d.ts` refresh is the two-line diff from `f2b04198` to `bf8452ec`; it landed in that concurrent commit because the shared branch advanced between the scoped source commit and the generated-file amend. No concurrent source was removed or restored. The client remains at `56d80272bcbda3eb1e22db096c1c5e21d3497de4`.

## Rust-owned matching

`api::content` publishes the lowercase common-loot terms and `matches_common_bank_loot(name, id)`. The Load isolate registers one native callback and the Banking shim only maps that callback into `matchesCommonBankLoot` and `depositMatcher`. The script-supplied predicate runs first, so a true custom match short-circuits the native call; `includeCommon=false` excludes the Rust arm.

The numeric casket exception is `405`. It is not inferred from a name: both selected content sources map `405=casket` in `content/pack/obj.pack`:

- revision 274 source `0b6a0cb7d11667be0828ccec95ef02057763b768`, pack SHA-256 `358e01a1f2e3751e3d1f8c6704b2f248f8f160567a73eddfc53feeaaf49c7f77`;
- revision 289 source `c236b0e578373af3d7d0097eebb2d89ab2bb95d1`, pack SHA-256 `b5e97146856661a96b2f78690da7f366fed1e871637a0982d4a5ebfa30ac2bf7`.

The corresponding selected cache manifests hash to `d2a67e1b52bbb8ccf5a9b8c764d5c11bd9740dc9ba2d23818a98c5a84417a2f4` (274) and `d9be46828fbaeaaa2a8a9ffcfb882847e931994ae145f1395cfe4a6ce47c773e` (289).

## Observed withdrawLoad

`Bank.withdrawLoad(name)` requires a ready bank and a fresh positive stock row. A known-full inventory returns true without sending. Other requests cross the isolate boundary as one `withdraw-load` request carrying the observed name and bank generation.

Rust recomputes capacity, stock and operation availability from the current snapshot. It selects an exact `Withdraw N` operation first, `Withdraw All` only when the bounded fill consumes current stock, then the existing host-owned Withdraw-X continuation. Unsupported, stale, missing and competing requests post false on the withdrawLoad result channel.

The shared pending operation captures inventory slots, target-item count and bank stock before send. Settlement is true only after observed inventory quantity/slot growth or a decrease on a still-present bank row. A vanished stock row by itself remains pending and cannot produce a false success. Dialog and settlement retain the existing 3000/4000 ms monotonic bounds.

The snapshot schema appends withdrawLoad result sequence/value fields. JavaScript waits only for that host result or a bank session change; it has no competing wall-clock timeout.

## Pause, hold and abort behavior

The parent Withdraw-X path now also treats the host outcome as authoritative. The composed host test advances a controlled isolate clock beyond the former 8000 ms JavaScript bound, runs a Guardian-held frame, resumes without a host outcome, and verifies the promise remains pending before the dialog and observed inventory complete it successfully.

Pause/hold freeze the Rust deadline. Disconnect and `reset_session_work` complete the current pending result as false before clearing it, including when a later bank reuses the same generation. Stop destroys the isolate and drops pending work; it cannot publish a late result into a new isolate lifetime.

## Verification

Checks were run against the separate exact source export at `.superpowers/task-exports/t_41b2f50e-06077fe9`, not by stashing or restoring the shared worktree. Raw outputs are under `docs/compat/evidence/bank-matching-fill/`.

- `cargo test -p api common_bank_loot_matches_names_and_verified_casket_id`: pass.
- common-loot isolate callback/short-circuit test: 1 passed.
- withdrawLoad isolate tests: 2 passed.
- prior isolate bank family: 17 passed.
- pending Pause/reset unit test: 1 passed.
- withdrawLoad host selection/composition tests: 2 passed.
- controlled-clock Pause/hold composed Withdraw-X test: 1 passed.
- `cargo test -p host-play --lib`: 122 passed.
- withdraw gold stub: 1 passed.
- generated host-js checks: 2 passed, 1 regen test ignored.
- script and host-play clippy with `--no-deps -D warnings`: pass.
- `cargo fmt --all -- --check`: pass on the exact export.

A broad `cargo test -p script` on the shared worktree still encounters pre-existing catalog gaps outside this task (`STAFF_RUNES` imports in Alcher/Superheater and the existing silent-fakes expectation). The focused bank, generated API and host checks are green. No live client was launched; root still owns revision 274/289 live acceptance, frontend integration and final whole-branch review.
