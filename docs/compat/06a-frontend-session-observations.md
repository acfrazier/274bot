# Frontend session observation boundary

Task: `t_887b29b5`
Branch: `codex/rs2b0t-multirevision`
Implementation base observed during final verification: `659347e17f7c1d8916e8993d35c9bf671d40a4f5`

## Change

Panel and TUI now publish frontend-owned snapshots through a minimal host helper built from the production `Pump::drain_client` and `publish_snapshot` path. Each frontend keeps a per-slot `ClientGens` publication cursor so logout and successful login/reconnect boundaries reset snapshots at the actual session watermark before local-player, scenario, or Guardian-hold early returns. Panel rebuilds `WorldState` whenever the resulting snapshot changes.

At a session boundary, both frontends remove the username's external `WalkArm` (including any `BankBudget` continuation) and player tick latch. Explicit slot spawn/removal lifecycle paths also remove the publication cursor, snapshot/facts, armed work, and latch, so same-name client replacement cannot inherit the previous slot lifetime. Ordinary scene rebuilds and Guardian holds do not clear armed work.

No client, API, script, or `host-play/src/lib.rs` source was changed by this task.

## Focused behavior receipts

The six new tests passed individually with exact filters before concurrent banking commits advanced the shared checkout:

- `cargo test -p panel session::tests::frontend_logout_clears_facts_armed_work_and_tick_latch -- --exact --nocapture` — 1 passed
- `cargo test -p panel session::tests::frontend_response_15_replacement_waits_for_post_grant_player_packet -- --exact --nocapture` — 1 passed
- `cargo test -p panel session::tests::same_name_lifetime_resets_but_scene_change_and_guardian_hold_preserve_work -- --exact --nocapture` — 1 passed
- `cargo test -p tui bin::tests::frontend_logout_clears_facts_armed_work_and_tick_latch -- --exact --nocapture` — 1 passed
- `cargo test -p tui bin::tests::frontend_response_15_replacement_waits_for_post_grant_player_packet -- --exact --nocapture` — 1 passed
- `cargo test -p tui bin::tests::same_name_lifetime_resets_but_scene_change_and_guardian_hold_preserve_work -- --exact --nocapture` — 1 passed

Current-base checks:

- `cargo fmt -p host -p panel -p tui -- --check` — passed
- `git diff --check -- crates/host/src/lib.rs crates/panel/src/session.rs crates/tui/src/bin.rs` — passed
- `cargo test -p host` — 136 passed; doc tests passed

## Shared-checkout limit

A later `cargo test -p panel -p tui` attempt on `659347e1` stopped in concurrently modified banking-owned `crates/host-play/src/lib.rs`: `InteractReq::WithdrawX` and `InteractReq::OpenBooth` patterns did not yet cover newly added fields. A workspace-wide format check likewise reported only concurrent formatting diffs in `crates/host-play/src/lib.rs` and `crates/script/src/isolate_fb.rs`. This task did not edit those files or work around the ownership boundary. No LIVE or external fixture actions were run.
