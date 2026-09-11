# Native hint and local overhead facts

Implemented brief 117 by projecting the client’s existing local-player overhead text and normalized coordinate hint through `GameSnapshot`, the isolate FlatBuffer snapshot/delta transport, and the compatibility reader. `reader.selfChat()` returns only the current local actor text, while `reader.hintTile()` returns `{x, z}` only for native coordinate hints. Both clear to `null` when absent, including expiry, non-coordinate hint types, logout/session reset, and delta transitions.

The two native facts are refreshed on every snapshot read because local chat send/expiry and R274 `HINT_ARROW` do not have dedicated generation invalidations. The overhead allocation is retained while unchanged; hint coordinates are copied scalars. Existing snapshot callers remain source-compatible, old buffers decode with absent defaults, and reused isolate buffers carry changes and clears without retaining stale values.

`actions.setRun(on)` is a thin adapter over the existing `set-run` isolate command; it preserves queued command order and delegates execution to the existing Rust interaction path. No client opcode, parser, game-policy state machine, navigation behavior, or foreign runtime was added.

Verification used the dedicated `.superpowers/review-exports/native-hint-overhead-t_6bd482fb-target` target:

- `cargo test --locked --offline -p api --test native_hint_overhead -- --test-threads=1` — 2 passed.
- `cargo test --locked --offline -p api --test snapshot -- --test-threads=1` — 62 passed.
- `cargo test --locked --offline -p script --test native_hint_overhead -- --test-threads=1` — 4 passed.
- `cargo test --locked --offline -p script --lib -- --test-threads=1` — 76 passed.
- `cargo test --locked --offline -p host-play --lib script_snapshot -- --test-threads=1` — 8 passed.
- Strict Clippy passed for the affected API library/test, script library/test, and host-play library.
- `rustfmt --check` on owned Rust and scoped `git diff --check` passed.

No LIVE run was performed, as required by the brief. Root retains the native/headless Duel and Brimhaven acceptance proofs.
