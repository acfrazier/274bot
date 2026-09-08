# TUI live_prepare fixture isolation

## Finding

The BoneBurier and Thiever unit fixtures called `TuiSession::live_prepare_script`,
which intentionally starts every scenario slot after preparing the vault and
scenario runner. The fixture supplied no live service (`127.0.0.1:43594` is only
the dummy endpoint), so teardown entered the real slot worker's retry path and
`Play::drop` waited for that worker. This made the tests depend on an unavailable
service and could stall the test process.

## Change

`crates/tui/src/bin.rs` keeps the production `live_prepare_script` body on its
original path, including slot spawning. A separate `#[cfg(test)]` fixture entry
point performs the same preparation with slot spawning omitted. Both paths
perform the real preparation: temporary vault creation/unlock, scenario setup,
live names, settings injection, catalog discovery/transpilation, script
selection, sibling resolution, and pending catalog-start staging. The test-only
duplication avoids adding a production-compiled spawn-control seam.

The BoneBurier and Thiever tests use that test-only entry point. Their
substantive catalog-selection and injected-settings assertions remain intact.
BoneBurier additionally proves that no arm/worker was created and that the
selected card is staged in `pending_script`; its `script_state != Running`
assertion is retained only as a non-vacuous boundary check. The unit fixture
covers vault/catalog/pending staging without slot workers. The meaningful
"Start waits for StartScript" isolate-state assertion remains live-harness
coverage. No production timeout, worker lifecycle, client source, or host-play
code changed.

## Verification

Attempted:

- `cargo test -p tui --bin tui-play live_prepare_bone_burier_selects_the_rs2b0t_card_without_starting -- --exact --nocapture`
- `cargo test -p tui --bin tui-play live_prepare_thiever_posts_guard_target_when_schema_empty -- --exact --nocapture`

Both commands were blocked before compilation because this checkout's client
submodule is absent: `vendor/fr-client-rust/crates/client/Cargo.toml` could not
be read. No native or live launch was attempted.
