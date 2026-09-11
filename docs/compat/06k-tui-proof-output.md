# Keep live proof output from corrupting an active TUI

Task: `t_35ee61ac`
Brief: `docs/compat/briefs/75-tui-proof-output.md`
Branch: `codex/rs2b0t-multirevision`
Kind: TUI output/cleanup ownership. Not LIVE, not a harness rewrite.

## Result

`--live` PASS/FAIL text is still the same machine-readable lines and the same
exit codes (PASS → 0, FAIL → 1). Headed `tui-play` no longer prints them onto
the Ratatui alternate screen. Concord B1 PTY capture showed `PASS: live alcher`
JSON leaking across script paint/chat rows while stderr was already redirected;
that was stdout during `EnterAlternateScreen`.

Headless/no-TTY still prints immediately. Headed holds the lines until after
`LeaveAlternateScreen` / raw-mode restore, then writes them. `BUDGET_S` soak
still keeps pumping after PASS; the PASS line is produced once and not
reprinted into later frames.

## Ownership

- `live_proof` is the `--live` report seam: PASS is one stdout line, FAIL is
  two stderr lines plus exit 1. `TuiSession::live_status` no longer prints.
- Headed `run_loop` collects proof lines, drops the terminal, restores, then
  flushes. A `TerminalGuard` restores on early headed errors; `process::exit`
  on the memory-profile path restores first because Drop does not run.
- Headless pump writes each proof line as it appears (CI / no TTY).
- Scenario predicates, Start/Pause/Stop, input ownership, and soak deadlines
  are unchanged.

## Verification

Peer WIP was not restored or stashed. Checks ran on an exact Git-blob export
of committed HEAD plus this card's `crates/tui/src/bin.rs` overlay:

`.superpowers/review-exports/t35ee61ac-tui-proof`

Target dir (fresh empty): `.superpowers/review-exports/t35ee61ac-tui-proof-target`

- Host `1d202ef8dd72cb6f2dd24e0aab77ccb78ceb97b3`
- Client `9d090ed04957e4efc254f073cda97bc5510ca72b`
- Overlay `crates/tui/src/bin.rs` sha256 `17a65a60995e813a245265bc231162d5e8bcc280beee933c74c9e98e518e6fe4`
- `cargo test -p tui --lib live_`: 6 passed (new proof tests + existing live_prepare)
- `cargo test -p tui --lib`: 98 passed
- `rustfmt --check crates/tui/src/bin.rs`: passed
- `cargo clippy -p tui --lib --tests -- -D warnings`: 4 pre-existing
  `clippy::type_complexity` hits in frontend slot helpers / `frontend_fixture`
  (not in the proof/restore change)
- `cargo clippy -p tui --lib --tests -- -D warnings -A clippy::type_complexity`: passed

No LIVE run. Root owns the Concord TUI rerun.

## Out of scope

Harness architecture, scenario/runtime/API files, logger credentials, and
actual headed PTY re-proof.
