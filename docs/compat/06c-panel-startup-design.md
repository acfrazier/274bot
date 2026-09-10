# Responsive panel startup preparation design

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded pre-implementation design/source review of
`docs/compat/briefs/33-panel-startup-design.md` against frozen
1947741f / client 56d8027 and the 06b startup trace. Not runtime
evidence, not a release-performance claim, not measured hashing
savings, not frontend/preservation acceptance, not campaign release.

Read once: `AGENTS.md`, `docs/execution.md`, plan step 8 in
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
`docs/compat/01-session-profile.md`, `docs/compat/06b-panel-startup-trace.md`,
and the named brief. Branch checked first: `codex/rs2b0t-multirevision`
(not `main`). Work was read-only except this report. No product edits,
tests on moving WIP, LIVE, fixtures, STATE, matrix, subagents, commit,
stash, reset, restore, checkout, merge, remotes, gitlink, or release
actions. Concurrent banking/catalog WIP in `host-play` / `script` /
`api` was not used as product.

## Verdict

**Proceed to a bounded panel preparation/validation ownership change**
under the constraints below. Design approval is not source acceptance
and not native proof.

The stall is not login, wgpu, or the slot thread. After the first blank
present, the winit/imgui thread runs `boot_execute` →
`Session::bind_profile_with_env` / `unlock_at` → `start_play` →
`run_with_template`. Those three calls hash the selected nav pack and
flags (73,441,977 + 260,571,161 bytes) via `hash_file` (whole-file
read + SHA-256). Keep every identity check. Move the hashing and
template decode off that thread. Install the completed
`Arc<ServerProfile>` + `Arc<SharedClientTemplate>` on the UI thread.
Re-run `validate_resources` off the UI thread immediately before Play
construction. Do not move `Session`, vault UI, IsolatedEnv, catalog
transpile, GPU inject, or slot spawn onto the worker.

Do not treat pathname/mtime as identity. Do not delete bind,
`SharedClientTemplate::load`, or the last changed-resource check. Do
not open a startup/cache/performance campaign. Do not change login or
script lifecycle bounds.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Named host product (source inspected) | `1947741fb3326ce187cf7ca44f126345c4100155` |
| Named client | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Campaign HEAD at write-up | `6335e9c7302426ae2014499872afc24017169d3b` (docs/combat-scope; not this source) |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `0058a6bd5eeaf14425ad0c323bf7c137e59d598e568e6f78cbc70a7639b6c651` |
| Trace SHA-256 | `904c5fcd8a699f7a0007f29b3c3d2f78d20665c24946e29f7fc4c1af030d5be0` |
| Trace inventory | `docs/compat/evidence/catalog-headed/startup-trace-f2b04198.json` |
| Binary in that trace | `f2b04198396aaf1132f9d0a1386a822188d69d1f` (profile startup unchanged at 1947741f) |
| Kanban card | `t_9ee02af6` |

Panel `app.rs` / `session.rs` / `window.rs` and `host-play/src/profile.rs`
match 1947741f. `crates/host-play/src/lib.rs` has concurrent unreviewed
banking WIP; `run_with_template` / `SharedClientTemplate::load` /
`Play::from_template` were read from 1947741f only.

## What source and the trace agree on

`run_panel` configures the profile, then defers boot until after the
first present so GPU `inject_device` wins (`app.rs` `run_panel`,
`presented` flag). That first present is working: the window exists.
Frame 1 then calls `boot_execute` on the same `ui_frame` closure.

`boot_execute` always `ensure_profile_bound` then:

- `Boot::Unlock`: `Session::unlock` (BOT_VAULT_PASS headed path)
- `Boot::Live`: `live_prepare_*` → `unlock_at` / `start_vault`
- interactive with no pass: `boot_for` is `None`; Unlock click later
  still hits `start_vault` → `ensure_profile_bound` on the UI thread

`bind_profile_with_env` is one UI-thread call that does both
`ProfileSelection::bind` and `SharedClientTemplate::load`.
`start_play` then calls `run_with_template` with an empty profile
list, which hashes again before `Play::from_template`. Slot threads
start only after that Play exists (`focus_first_profile` / live
`load` / `ensure_slot`). The sample's first slot log at +37.471 s
is therefore after three hashing passes, not a network wait.

The three passes (keep all three; just not on the event thread):

1. `ProfileSelection::bind` → `CacheManifest::capture` (eight JAGs) +
   `validate_nav` (`hash_file` pack, optional flags) + CRC reread of
   the JAGs + RSA. Frozen hashes live on `ServerProfile`.
2. `SharedClientTemplate::load` → `validate_resources` (re-capture
   cache, re-hash pack/flags vs frozen) then `load_template_checked`
   and `NavWorld::load_pack`.
3. `run_with_template` → `validate_resources` again, then Play.

`hash_file` is `std::fs::read` plus `Sha256::digest` of the whole
buffer (`crates/nav/src/manifest.rs`). The 06b raw sample is
functional diagnosis of a debug build's software SHA-256 on the UI
thread. It is not a release timing budget.

Interactive Unlock without BOT_VAULT_PASS is the same bind+load+Play
hash, only later, on the Unlock button (`vault_unlock_prompt`). Both
paths need the same ownership change.

TUI binds and loads before its event loop (`crates/tui/src/bin.rs`
`run`). That is a terminal pause, not a native beachball. Leave TUI
on `run_with_template`. Do not expand this task into TUI async.

## Ownership

### Worker may

Capture `ProfileOptions`, `ProfileEnvironment`, and the current saved
revision (Copy/owned values only). Then:

1. `resolve_with_env`
2. `ProfileSelection::bind`
3. `SharedClientTemplate::load`
4. On the Unlock / live-start edge: `template.profile().validate_resources()`

Use `std::thread` + `mpsc` (buffer 1). No new runtime. The worker
must not touch winit, imgui, wgpu, `inject_device`, IsolatedEnv,
`JsLibrary`, catalog transpile, vault files, `picker::set_navflags_path`
/ `set_pack`, or `Play` / slot threads.

### UI thread must

Keep `Session`. After GPU init and the first present:

1. Spawn the prepare worker if `profile_options` is set and nothing
   is bound. Keep painting. `RedrawMode::WaitUntil { fps: 50.0 }`
   already wakes the loop; also `request_redraw` while a job is in
   flight so Mac/Windows Wait modes cannot stall the poll.
2. `try_recv` at the top of `ui_frame` (same place as today's
   `boot.take()`).
3. On `Ok(template)` with a matching generation: install
   `server_profile` + `template`, copy connection fields into
   `PlayOptions`, call `picker::set_navflags_path` here (not on the
   worker). Then continue the deferred boot without a second bind.
4. On `Err(msg)`: see error policy below. Do not install a half
   profile.
5. Vault open, `start_play` Play assembly, `per_frame` closure,
   focused/live slot spawn, scenario runner, catalog transpile, and
   `login_all` stay here, after GPU init, as they do today.

`Session::new` stays on the UI/test thread (`IsolatedEnv::ensure_thread`
is `cfg(test)` only; production JS store is constructed there).

### Generation, user actions, shutdown

Hold a monotonic `prep_generation` on the panel state.

- `set_server_revision` / `configure_profile` before bind: increment
  generation and drop any in-flight result. Start a new worker with
  the new inputs. Do not install a stale 289 template over a 274
  click (or the reverse).
- Unlock click while preparing: stash the passphrase; do not call
  `bind_profile` on the UI thread. When the matching generation
  lands, run the last validate worker then `start_vault` without
  bind.
- Unlock click after template install: last-validate worker, then
  vault + Play. The user may have sat on the prompt; files may have
  changed. Bind hashes from first present are not the slot check.
- Live boot: wait for the matching template, last-validate, then
  existing `live_prepare_*` without bind. Failure remains fatal.
- CloseRequested / `run_panel` return: drop the receiver. Do not
  `join` the worker on the UI thread. A late `send` fails; the
  worker drops the Arcs. Never install Play after shutdown.
- Already bound: keep today's "restart to change revision" error.
  Do not start a second process profile.

### Final validation

The slot-start contract is disk bytes versus the frozen hashes on
`ServerProfile`, not a cached path, mtime, or in-memory decode
digest.

- Keep `bind` as the identity establishment (pass 1).
- Keep `SharedClientTemplate::load`'s `validate_resources` (pass 2)
  for CLI/TUI and for the worker. Do not delete it to save a pass.
- Keep a last `validate_resources` immediately before Play (pass 3).
  Today's `run_with_template` does that on the UI thread, so a
  panel-only move of bind+load still beachballs for ~one pass
  (~334 MB). That is not a sound correction.

Add a host sibling that constructs Play **without** hashing, and
call it only after a just-finished `validate_resources` on that
same `Arc<ServerProfile>`:

```text
run_with_template        = validate_resources + run_prepared_template
run_prepared_template    = Play::from_template + per_frame + optional spawns
```

Panel last-validate worker: `template.profile().validate_resources()`.
Next UI frame: `run_prepared_template` (empty profile list, same
`per_frame` as today). TUI/CLI keep `run_with_template`.

The TOCTOU window is then "worker Ok → next ui_frame Play", the
same class as today's in-function gap between hash and
`from_template`. Do not add a fourth UI-thread hash.

`require_bot_operation` stays on the UI install / live path as
today (`boot_execute` already calls it). Empty-profile Play
construction does not send.

### Error without a partial session

| Path | On prepare/validate failure |
|---|---|
| Interactive Unlock / BOT_VAULT_PASS Unlock | `session.error = actual message`; stay on the vault prompt; `play`/`vault`/`slots` remain None; do not set `server_profile` without a template |
| Live / smoke | existing `FAIL: {e}` + `exit 1`; no slot thread |
| Bind mismatch (`cache/profile mismatch`, `navigation/profile mismatch`) | same strings as today; no vault create |
| Changed after bind (`cache changed`, `navigation changed`, `flags changed`) | last validate fails; no Play |

Do not wrap I/O or hash errors into a timeout. Do not spawn a slot
to display the error.

## Why not move Session

`Session` owns imgui-facing buffers, `Focus`, vault, Play, per-frame
closures over those Arcs, scenario runner, JS library, and (in tests)
IsolatedEnv. Slot renderers are created lazily at first paint and
must see `inject_device` from `on_gpu_init`. Moving Session, or
spawning slots on the worker, crosses GPU ownership and
thread-local store isolation. The worker result is two Arcs plus
an error string.

## Windows and Mac

Both frontends use the same panel `window::run` winit
`ApplicationHandler`. Blocking `ui_frame` freezes AppKit (beachball)
and Win32 (unresponsive / Not Responding). The fix is identical:
do not hash on that thread; keep the 50 fps wait-until loop pumping;
`try_recv` each frame.

Do not use GCD, Win32 worker-window APIs, or COM init on the hasher.
Do not create winit/wgpu objects on the worker. `EventLoop::new`
stays on the process UI thread.

## Reusable host value

Do not invent a second identity type. `Arc<SharedClientTemplate>`
already carries the immutable profile, decoded cache/ifaces, and
shared `NavWorld`. Add a thin helper on the stable profile module:

```text
ProfileSelection::prepare_template(&self) -> Result<Arc<SharedClientTemplate>, String>
  = bind() then SharedClientTemplate::load
```

Panel and any later caller share that. It does not skip validation.

## Allowed files

| File | Change |
|---|---|
| `crates/panel/src/app.rs` | After first present, spawn/poll prepare; keep chrome painting; run live/Unlock only after install; banner/disable Unlock while preparing; generation; no `std::process::exit` on Unlock hash (live FAIL unchanged) |
| `crates/panel/src/session.rs` | Install prepared Arcs; Unlock/live must not re-bind; last-validate hook before `start_play`; stash passphrase; refuse install on generation mismatch; keep `bind_profile_with_env` for tests |
| `crates/host-play/src/profile.rs` | `prepare_template` helper only |
| `crates/host-play/src/lib.rs` | **hotspot:** split `run_with_template` / `run_prepared_template`. Touch only that pair and `from_template` visibility. Do not edit banking/script dispatch |

Not in this task: `hash_file` streaming, nav pack format, client,
TUI event loop, `window.rs` unless a one-line redraw bug appears,
script/api, login, catalog, IsolatedEnv.

`lib.rs` is concurrent banking WIP. Implementers should take a
narrow slice or wait until that file is free rather than merge
across banking.

## Concrete risks

1. **Leaving pass 3 on the UI thread.** Bind+load off-thread still
   beachballs at Unlock / `start_play`. Incomplete.
2. **Skipping last validate because the template is "fresh".** Unlock
   can be minutes after first present. Violates changed-resource
   refusal.
3. **Calling `run_prepared_template` from CLI/TUI.** Footgun. Keep
   `run_with_template` as the default checked entry.
4. **Joining the worker on CloseRequested.** Reintroduces the stall
   at shutdown. Detach.
5. **Installing a stale generation** after a revision click.
6. **`picker::set_navflags_path` / `set_pack` from the worker.**
   Those are process UI/nav globals; set them on install.
7. **Spawning slots or `login_all` on the worker** before
   `inject_device` / first present. Restores the original GPU-order
   bug that deferred boot exists to prevent.
8. **Transpile / `fill_rs2b0t_cards_once` on the hasher.** IsolatedEnv
   / JS store stay with Session. Live catalog load after Play is a
   different pause; not this trace.
9. **Parallelizing the three hash passes.** Peak RSS would hold
   multiple 261 MB buffers. Sequential on one worker is enough.
10. **Editing `hash_file` or adding mtime short-circuits.** Out of
    scope; fails the identity rule.

## Existing regressions to extend

Keep, do not weaken:

- `host-play` `cache_and_nav_mismatch_are_rejected_before_creating_resources`
- `invalid_explicit_rsa_fails_without_fallback_and_binding_detects_resource_changes`
  (`cache changed` after bind; no vault file)
- flags-changed `validate_resources` after a bound 289 pack
- panel `explicit_profile_wins_saved_revision_and_bound_session_refuses_changes`
- panel `boot_for` mapping (Unlock / live / smoke / none)

Add focused tests (small host-play fixture, not 1 GB packs):

- `prepare_template` then file change then `validate_resources` still
  errors with `cache changed` / `flags changed` / `navigation changed`
- `run_with_template` still refuses a changed disk; documents that
  `run_prepared_template` does not hash
- panel: in-flight generation mismatch is dropped; `server_profile`
  stays None
- panel: prepare/validate `Err` sets `session.error`, `play` is None,
  no slot map entry
- panel: Unlock after successful prepare does not call bind again
  (`already bound` must not fire)

No new LIVE harness and no tests against unreviewed banking WIP.

## Bounded native proof

Root-owned, after implementation, on the frozen-plus-fix pair. Not
this review. Not a release-performance acceptance.

1. Debug native panel, local Mac 289, same headed launch shape as
   06b. Window must paint and accept move/resize/close while
   preparation is in flight. No multi-second beachball after first
   present. Slot-thread log must not appear before install.
2. After a successful paint, replace or truncate the selected nav
   pack or a cache JAG and then Unlock / live-start. The process
   must refuse with the existing changed/mismatch message and must
   not log a slot thread.
3. Windows native panel: the window must not freeze / Not Responding
   during the same prepare window. Same changed-resource refusal.
4. Interactive Unlock (no BOT_VAULT_PASS): first present shows the
   prompt; typing works during prepare; Unlock after change still
   refuses.

Use actual event-loop responsiveness and actual refusal. Do not
quote 06b's 37 s as a budget or claim a hashing speedup.

## Out of scope

Plan step 8 frontend/preservation (Browse, Start/Stop, N=2 isolation,
N=32 Thiever, CPU/GPU overlay) is not this change. Login, reconnect,
script Stop, and catalog proof stay on their owners. TUI blocking
before `run_loop` is accepted for this task.
