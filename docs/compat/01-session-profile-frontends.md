# Session profile frontend wiring

Status: implementation complete; native macOS proof remains with the root integrator.

## What changed

- `panel-play`, `tui-play`, and `host-play` now parse the shared profile surface (`--profile`, `--revision`, `--prod`, endpoint, asset, cache, nav, content, vault, catalog, and manifest overrides) before frontend-specific flags.
- All three production entry paths resolve and bind one immutable `ServerProfile`, load one `SharedClientTemplate`, and reuse it for initial clients and later slot spawns.
- Revision 289 remains constructible offline but is refused at the host boundary before memory preparation, temporary-vault creation, vault open/create, or account upsert.
- Target-sensitive vault paths, live passphrases, generated passwords, and catalog roots come from the selected/bound profile rather than mutable ambient process state.
- Panel exposes the effective server and revision before session binding. The single persisted revision field migrates to 274. Explicit CLI/environment choices retain precedence. Once bound, revision changes and rebinding return restart-required errors, including after vault lock.
- Panel captures profile environment once, resolves explicit pre-bind catalogs instead of ambient `RS2B0T`, binds the selected navflags path, and records full catalog identity when cards load. Binding refuses root or source-hash changes, including same-root edits after warmup, without clearing custom file cards.
- Panel and TUI stress scatter capture the selected shared template and use its world-specific scatter seed.
- Both frontends exercise their real parser-to-profile-to-template-to-client path against the synthetic 274 and 289 manifests, each with a private copy of all eight JAGs and explicit public RSA values. Both assert the default game host is `127.0.0.1`.
- `host-play` no longer mutates global target state while parsing and now loads checked assets before touching its vault.

## Verification

- `cargo fmt --all -- --check` — pass.
- `cargo check -p host-play -p panel -p tui --all-targets` — pass.
- `cargo clippy -p panel -p tui --all-targets -- -D warnings` — pass.
- `cargo clippy -p host-play --bin host-play -- -D warnings` — pass.
- `cargo test -p host-play` — pass; production live tests remain intentionally ignored unless `LIVE=1`.
- `cargo test -p panel --features memory-profile` — 384 passed, 0 failed.
- `cargo test -p tui --features memory-profile` — 89 passed, 0 failed.
- `git diff --check` over source/docs excluding raw Cargo logs — pass. The three
  Cargo test logs preserve Cargo's exact trailing blank line at EOF.

Raw logs: `docs/compat/evidence/session-profile/frontends/`.

## Remaining integrated proof

The root integrator owns native macOS UI/visual validation with the prepared proof vault. No live gameplay was launched by this frontend task, and offline construction tests are not represented as 289 gameplay acceptance.
