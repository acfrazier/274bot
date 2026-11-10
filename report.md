# Parity 3B quantity report

Branch: `fr377/parity-3b-quantity`. Bases: content `rs2-r377` (`240cba8f6`), engine `rs2-r377` (`28ddc28f`), vault `vault-reorg/main` (`248728e8`).

## Checks

- `cargo fmt --all -- --check`: passed (inherited receipt `fmt-final.log`).
- `cargo test -p scenario`: 478 passed, 0 failed (inherited `scenario-full-final.log`).
- Full `host-play` suite with `test-support` and `BOT_NAV_BUILD=skip`: 1,613 passed, 9 failed, 115 ignored. Failures require absent R289/R377 and v16 navigation assets, local content/catalog, or allocation-profile environment; see `host-play-full-final.log`. A prior full run also failed while required local navigation staging was absent.
- `cargo clippy -p host-play -p scenario --features host-play/test-support --all-targets -- -D warnings`: pending.

## Live receipts and merge status

No live receipt was found for any changed path. Do not merge these changes until live proof exists (`NOT MERGEABLE until live proof`):

- `CHANGELOG.md`
- `crates/api/data/`
- `crates/host-play/tests/receipt_alloc.rs`
- `crates/host-play/tests/revision_377_live.rs`
- `crates/host-play/tests/revision_377_live/quantity.rs`
- `crates/scenario/src/catalog.rs`
- `crates/scenario/src/lib.rs`
- `crates/scenario/src/proof.rs`
- `crates/scenario/src/runner.rs`
- `crates/scenario/src/runner_tests.rs`
- `crates/scenario/src/scenarios/mod.rs`
- `crates/scenario/src/scenarios/quantity.rs`
- `crates/scenario/src/ui_actions.rs`

No additional live attempt was made.