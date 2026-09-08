# TUI live_prepare fixture isolation

## Finding

The BoneBurier and Thiever unit fixtures called `TuiSession::live_prepare_script`,
which intentionally starts every scenario slot after preparing the vault and
scenario runner. The fixture supplied no live service (`127.0.0.1:43594` is only
the dummy endpoint), so teardown entered the real slot worker's retry path and
`Play::drop` waited for that worker. This made the tests depend on an unavailable
service and could stall the test process.

## Change

`crates/tui/src/bin.rs` now keeps the production path unchanged: the normal
`live_prepare_script` delegates to the shared preparation implementation with
slot spawning enabled. A `#[cfg(test)]` fixture entry point delegates with slot
spawning disabled. The shared implementation still performs the real test
preparation: temporary vault creation/unlock, scenario setup, live names,
settings injection, catalog discovery/transpilation, script selection, sibling
resolution, and pending catalog-start staging.

The BoneBurier and Thiever tests use that test-only entry point. Their
substantive catalog-selection and injected-settings assertions remain intact;
BoneBurier also continues to assert that preparation has not put the isolate in
`Running` state. No production timeout, worker lifecycle, client source, or
host-play code changed.

## Verification

Attempted:

- `cargo test -p tui --bin tui-play live_prepare_bone_burier_selects_the_rs2b0t_card_without_starting -- --exact --nocapture`
- `cargo test -p tui --bin tui-play live_prepare_thiever_posts_guard_target_when_schema_empty -- --exact --nocapture`

Both commands were blocked before compilation because this checkout's client
submodule is absent: `vendor/fr-client-rust/crates/client/Cargo.toml` could not
be read. No native or live launch was attempted.
