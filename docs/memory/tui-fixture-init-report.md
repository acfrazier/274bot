# TUI live_prepare fixture isolation

## Finding

The BoneBurier and Thiever unit fixtures called `TuiSession::live_prepare_script`,
which intentionally starts every scenario slot after preparing the vault and
scenario runner. The fixture supplied no live service (`127.0.0.1:43594` is only
the dummy endpoint), so teardown entered the real slot worker's retry path and
`Play::drop` waited for that worker. This made the tests depend on an unavailable
service and could stall the test process.

## Change

`crates/tui/src/bin.rs` keeps `live_prepare_script` as the single preparation
implementation. Under `#[cfg(test)]`, `TuiSession` has a per-session
`suppress_slot_spawn` fixture switch, checked only at the `spawn` boundary; the
unit fixtures enable it before calling the real production preparation method.
The production build contains neither the field nor the guard. The tests
therefore exercise temporary vault creation/unlock, scenario setup, live names,
settings injection, catalog discovery/transpilation, script selection, sibling
resolution, and pending catalog-start staging on the path used by `--live`.

The BoneBurier and Thiever tests use that test-only switch. Their substantive
catalog-selection and injected-settings assertions remain intact.
BoneBurier additionally proves that no arm/worker was created and that the
selected card is staged in `pending_script`. The unit fixture covers
vault/catalog/pending staging without slot workers; it does not assert isolate
state, since `script_state` is idle when no worker exists. The meaningful
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
