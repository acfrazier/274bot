# Panel startup preparation source review

Reviewer: Hermes profile `reviewer`, model `grok-4.5`, provider `xai-oauth`.
Date: 2026-09-10. Kind: corrective same-card source review after implementer
`t_779f58e5` completed without the mandated review handoff. Not native
responsiveness proof, not LIVE, not release acceptance.

Read once: `AGENTS.md`, brief `docs/compat/briefs/42-panel-startup-preparation.md`,
accepted design `docs/compat/06c-panel-startup-design.md`, implementer report
`docs/compat/06d-panel-startup-preparation.md`, and
`docs/compat/evidence/panel-startup-preparation/verification.md`. Branch
checked first: `codex/rs2b0t-multirevision` (not `main`). Work was read-only
except this report. No product edits, LIVE, fixtures mutation of operator
resources, stash/reset/checkout, merge, remotes, gitlink, or release actions.

## Verdict

**Approve frozen source `814e5293`.** The bounded panel preparation/validation
ownership change matches 06c and brief 42. No blocking defects found. Native
Mac/Windows responsiveness and disposable full-size changed-resource proof
remain root-owned and are not claimed here.

This approval is source/contract acceptance of the frozen implementation and
its unit evidence. It is not a beachball budget, hashing speedup, or TOCTOU
elimination claim.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen implementation | `814e5293fec426b722e450580c514b9cffca826f` |
| Implementation parent / base | `ca7d4c24c301898bbdf19e51d34f23755e76d3b1` |
| Implementer report | `6eba96ac12d1da5c55aa1d6524b09dcd3a2ed239` |
| Campaign HEAD at review start | `a3dbc4d37b48e0ff8e5060cb436a376f81d84363` |
| Branch | `codex/rs2b0t-multirevision` |
| Kanban card | `t_4bd3fe69` (corrective; gates `t_f1572cf0`, `t_dde220ef`) |
| Prior implementer card | `t_779f58e5` |
| Design acceptance | `06c-panel-startup-design.md` (Grok 4.6 run 1142) |
| Brief SHA-256 | `71dc4de2010d409b81378fdada7cceea4856dff58e70455c0e11fde2cf99b7bf` |
| Design SHA-256 | `6560e21e24e35a0ce60f00efda7f75e7abbb5c222dbe418b0afaf0f6df470455` |
| Report SHA-256 (pre-review) | `00979340a76b5582af38981bb38a1fd5086526d5f7b752ccf74643ecb92c4cc4` |
| Verification SHA-256 | `d49d1ebad116e971aee2c7e5cf6a931de054b0ce5448163b983c332ab798a335` |

Exact export of the five product files at `814e5293` reproduced the
implementer content hashes:

```text
c04b072e4a2cfdec3df14646e8e2063f3f40728bba44e04b160ab8c2ffc3fc66  crates/host-play/src/lib.rs
98bbc40fed97755fa8d6906fd74070a26630eef641467c44e6033ddbcd424204  crates/host-play/src/profile.rs
1ba6ccef5b4b1a49a55c7f0804586a5b9969814e96e706b3a6601335b63f5548  crates/host-play/tests/session_profile.rs
8af2c5477dfc9ee0b54a52c1d0a29b809b4dd093943ada99ee61df6b37700c57  crates/panel/src/app.rs
81a44f0780bf1bb2e7fac470f6cf9c3fb49f1ce01d2a8cff20e26495e24822d4  crates/panel/src/session.rs
```

`git show --stat 814e5293` is only those five paths (652 insertions, 56
deletions). At review time the five paths still matched `814e5293` on the
campaign branch (no later product edits to the same files).

## Contract map (brief 42 / 06c → source)

### Validation handoff

- `SharedClientTemplate::validate_for_play` is the only constructor of
  `ValidatedTemplate`. The ticket field is private; no `Clone`/`Default`.
- `run_with_template` still validates, then delegates to
  `run_prepared_template`.
- `run_prepared_template` consumes the ticket by value and does not call
  `validate_resources` / hash again.
- Panel `install_validated_template` requires `Arc::ptr_eq` against the
  already-installed template before storing the ticket.
- `start_play` prefers `validated_template.take()` → `run_prepared_template`;
  the `run_with_template` fallback remains for tests/legacy unbound paths and
  is not the deferred Unlock/live path after `drive_startup`.

This matches the brief’s one-use preparation ticket (not a second identity
model) and the design’s split of checked convenience vs prepared construction.

### Three resource passes, final pass off UI

1. Worker: `ProfilePreparation::run` → `prepare_template` =
   `bind` + `SharedClientTemplate::load` (load still calls
   `validate_resources` — pass 2 after bind’s pass 1).
2. Worker: final `template.validate_for_play()` immediately before Play
   (pass 3), sequential after install, not parallelized with pass 1/2.
3. UI: install ticket then `run_prepared_template` with empty profile list.

No pathname/mtime shortcut, no `hash_file` edit, no fourth UI-thread hash on
the prepared path. The hash-to-construction TOCTOU class is explicitly
retained (and unit-tested).

### Generation / cancellation

- `configure_profile` and pre-bind `set_server_revision` bump
  `profile_generation`.
- In-flight prepare jobs whose generation no longer matches are dropped by
  clearing the receiver (`prepare = None`); the hasher is never joined on the
  UI thread.
- `finish_profile_preparation` returns `Stale` and installs nothing when the
  generation mismatches.
- Stale validate results are discarded the same way after `try_recv`.
- CloseRequested exits the event loop; `StartupPreparation` drops with the
  closure — receivers drop, late `send` fails, no join.

### Deferred / live and interactive Unlock

- First UI frame presents chrome with `presented == false` so
  `drive_startup` does not run until after first present; `inject_device`
  remains in `on_gpu_init` before any slot path.
- `boot_for` still maps live/smoke and `BOT_VAULT_PASS` Unlock; interactive
  without pass stays `None` until the prompt.
- Interactive Unlock uses `request_unlock` only (no direct UI-thread
  `bind_profile` / `unlock` from the button). `drive_startup` stashes or
  replaces the pending Unlock passphrase, including while prepare/validate
  workers run.
- After template install, Unlock and live wait for the final validate worker;
  `boot_execute` runs only after ticket install.
- Bound profile refuses revision/config change (“restart to change”).
- Live/smoke failures stay fatal (`FAIL:` + exit 1). Unlock failures stay
  non-fatal (`session.error`, stay on prompt).

### GPU / UI ownership

Worker threads only run `prepare_template` / `validate_for_play` and send
Arcs/errors. UI retains Session, vault open, Play assembly, slot spawn,
scenario, catalog/JS stores, `picker::set_navflags_path` on install,
`picker::set_pack` after Play, and `request_redraw` while jobs are in flight.
No winit/imgui/wgpu objects on the worker.

### Honest failure / no partial session

Prepare `Err` leaves `server_profile` / template / vault / Play / slots
absent and preserves the actual error string. Final validate `Err` uses the
same fail path before vault/Play. Live remains fatal; Unlock remains
non-fatal. Passphrases stay on the UI-side `Boot::Unlock` / request queue —
not sent to workers or written into proof artifacts.

### Scope

Only the five allowed files changed. No client, nav format/`hash_file`, TUI
event loop, banking/script dispatch body beyond the narrow host-play
template/Play split, login timeout, fixtures, gitlink, or new runtime.

## Independent checks run

Exact frozen paths (still identical to `814e5293` on the branch):

- `cargo test -p host-play --test session_profile` — 11 passed, 0 failed.
- Focused panel lib filters — 6 passed, 0 failed:
  - `boot_is_deferred_and_maps_live_smoke_and_vault_pass`
  - `normal_unlock_waits_for_worker_validation_then_uses_prepared_profile`
  - `live_boot_stays_deferred_while_final_validation_is_in_flight`
  - `stale_profile_preparation_is_dropped_without_partial_session_state`
  - `profile_preparation_failure_keeps_vault_play_and_slots_absent`
  - `validated_profile_unlock_uses_the_ticket_without_rebinding`

These cover final-validate refusal on cache/flags/nav mutation, checked
`run_with_template` still hashing, consuming ticket not re-hashing, stale
generation drop, failure without partial session, Unlock without rebind, and
live remaining vault/Play/slot-free while final validation is in flight.

No LIVE. No native headed Mac/Windows run in this review.

## Non-blocking notes

1. **Root-owned native proof still required.** 06c’s Mac/Windows paint-while-
   preparing and disposable changed-resource refusal are not closed by unit
   evidence. 06d already states this; this review agrees.
2. **Unlock control is not `begin_disabled` during prepare.** The prompt stays
   interactive (typing + queue Unlock), which matches 06c’s interactive Unlock
   requirement more closely than a hard disable. Banner text
   `Preparing server profile…` is present.
3. **`start_play` still has a checked `run_with_template` fallback** when no
   ticket is installed. Production deferred Unlock/live installs the ticket
   first; the fallback is appropriate for tests and non-prepared callers and
   does not reopen a bare unchecked Play API.
4. **`run_with_template` / `run_prepared_template` both call
   `require_bot_operation` when profiles are non-empty.** Harmless double
   check on the checked entry; panel empty-profile construction does not
   exercise it.

None of these are acceptance blockers for the frozen source.

## Defects

None blocking. Gate may proceed for downstream cards that depend on this
source acceptance; native responsiveness remains a separate root proof.

## Out of scope (explicit)

Native Mac/Windows event-loop proof, LIVE harnesses, frontend/preservation
Browse/Start/Stop work, TUI async, streaming `hash_file`, banking/script
dispatch, commit amend/merge/remote/gitlink/release.
