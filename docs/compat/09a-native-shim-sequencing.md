# Native shim sequencing ownership

Brief 123 moves the bank-open, Baker stall, and autocast orchestration introduced by the compatibility shims into Rust-owned begin/next/result controllers. JavaScript now supplies posted observations, dispatches only the existing verbs returned by Rust, invokes the existing cake callback when instructed, and returns the native terminal result. It no longer selects gameplay phases, target/retry policy, or deadlines for these paths.

Bank opening retains the public `openBooth`, named `openNearest`, and `openNearestWorld` APIs. Rust owns the selected booth identity across approach, rejects replacement or missing identities, preserves the existing `walk-near`, `walk-nearest-bank`, and `open-booth` packets, requires a fresh loaded bank generation after an open command, and uses the native 60-second approach and 5-second readiness bounds. A queued open command is not reported as success.

The bounded `stealCakes` ABI remains one attempt. Rust uses `api::cake_stall::{select_baker_stall, needs_cake_restock, counts_as_stall_food}` for target and restock classification, owns movement/steal/result phases and the 60-second/2400-millisecond bounds, and returns only `stocked`, `combat`, `aborted`, or `no-progress`. `onSteal` is requested only after observed new stall food and remains timed before the final combat/stocked classification. No restock loop, alternate stall, additional steal, loot policy, or `classifySteal` implementation was added.

Autocast retains the existing API and selected-world generated controls. Rust owns combat-tab, chooser, spell, toggle, and armed-state ordering with the existing 2-second tab and 3-second phase bounds. Missing controls still throw in the shim; unknown spells, missing staff layout, missing observations, timeouts, and aborted work return false without inventing success.

All three controllers are wired into the existing isolate Pause/Resume, guardian hold, and session-reset lifecycle. Pause and hold freeze native deadlines; reset/logout invalidates tokens. Existing slot Stop/restart teardown destroys the isolate and its thread-local state, preventing late commands or results.

Verification used exact source `cd49e147ade94bc5b06e7798e08ed4ad05a5d245`, exact client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`, the overlay at `.superpowers/review-exports/native-shim-sequencing-t_fbfa61d2`, and the exclusive target `.superpowers/review-targets/native-shim-sequencing-t_fbfa61d2-target`:

- API cake-stall unit tests: 5 passed.
- Script library tests, including token/reset/timeout and slot Stop/restart coverage: 83 passed.
- Named bank multi-phase tests: 10 passed.
- Baker stall mapping and callback tests: 7 passed.
- Autocast ordering/lifecycle tests: 8 passed.
- Host-play snapshot tests: 8 passed.
- Strict Clippy passed for API, script library and three affected script integration tests, and host-play.
- `rustfmt --check` on owned Rust and scoped `git diff --check` passed.

A broader workspace `cargo test -p script` sweep reached one unrelated source-baseline failure: `declared_abi_fixture_matches_local_dts` reports the frozen fixture lacks the already-declared `Execution.noteProgress` member. The same exact test fails on the unmodified `cd49e147` source export, so it is not introduced or repaired by this bounded card.

No LIVE run, client/engine change, navigation change, scenario/catalog fixture change, or new opcode was performed. Root owns targeted bank/cake/autocast live requalification.
