# Selective merge: native validation preflight

Read-only checks on September 9, 2026 local time, during approved extraction.
No builds, live workloads, service starts/restarts, provisioning, review dispatch,
or Git mutations were performed by this preflight. Historical receipts below
identify reusable mechanisms; they do not validate the extracted build.

## Available environments now

| Environment | Verified current state | Reusable paths and limits |
|---|---|---|
| macOS, current machine | Rust 1.98.0; server Node PID 1852, cwd `/Users/acfrazier/experiments/Server/engine`, listening on 80/43594; about 480 GiB disk free | Current campaign `target` and primary `/Users/acfrazier/experiments/274bot/target` caches exist. Current campaign has debug/release outputs. Public cache `.../Server/engine/data/pack/client/versionlist`, `/Users/acfrazier/.274bot/274bot.navpack`, `.navflags`, JS catalog/cache and `/Users/acfrazier/experiments/rs2b0t` exist. Server listeners are wildcard, not loopback-only; do not describe their binding as isolated. |
| Native Windows/MSVC | SSH reachable at `austen@10.0.0.205`; machine DESKTOP-SL99R6C. BotTest has active console session 1. Native Rust/Cargo 1.98.0 work. No node, panel-play, tui-play, cargo or rustc returned by process inventory; no 80/43594 listeners. About 387 GiB free | Cache `C:\Users\Austen\274bot-campaign\target-native`; registry `C:\Users\Austen\.cargo\registry`; NASM `C:\Users\Austen\274bot-tools\nasm-3.02`. Existing fixture `C:\Users\BotTest\274bot-server-4c95f87`; cache `data\pack\client`; BotTest `.274bot` navpack/navflags exist. Server is stopped. |
| Concord native Linux | `ssh concord` succeeds; Linux 6.8.0-139-generic; Node 24.20.0. `274bot-concord-test-server.service` active, PID726, cwd `/home/acfrazier/274bot-campaign/server-4c95f87`, command `/usr/bin/node --import tsx src/app.ts`; 80/43594 bound to 127.0.0.1. No TUI/panel/compiler processes or login desktop sessions observed | `/home/acfrazier/.274bot/274bot.navpack`, `.navflags`, fixture cache/versionlist exist. Default Cargo/rustc fails because no default rustup toolchain is configured. Existing explicit 1.98 toolchain works (below). Only 13 GiB free; no compiler target cache found in the two named original workspace snapshots. Do not assume a warm native build cache. |

Windows SSH uses the already configured pinned key/known-host settings from
the `274bot-builder` ProxyCommand in `/Users/acfrazier/.ssh/config`:

```sh
ssh -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -o UserKnownHostsFile=/Users/acfrazier/.ssh/274bot_windows_20260907_known_hosts \
  -o IdentitiesOnly=yes -i /Users/acfrazier/.ssh/274bot_windows_20260907 \
  austen@10.0.0.205
```

No key content was inspected. Concord's existing SSH alias supplies port 22111.
The builder VM was not inspected: native Concord is the named Linux validation
environment, and no additional environment is needed for this bounded check.

For Concord, select the installed compiler without changing the default:

```sh
export PATH=/home/acfrazier/274bot-campaign/nav-toolchain-1.98.0-01/1.98.0-x86_64-unknown-linux-gnu/bin:$PATH
export RUSTC=/home/acfrazier/274bot-campaign/nav-toolchain-1.98.0-01/1.98.0-x86_64-unknown-linux-gnu/bin/rustc
```

Both direct executables returned 1.98.0. The installation receipt also records
the existing rustup name `274bot-1.98.0`; explicit binary selection was what this
preflight verified. Linux has Xvfb and Xorg installed but neither a current
desktop session nor DISPLAY/WAYLAND_DISPLAY. An Xvfb run would be an offscreen
backend check, not proof of a user's Linux desktop panel. Do not provision one.

At the local inventory, an active Git process used about one CPU core, alongside
ordinary desktop applications. Recheck jobs immediately before the clean pair;
this preflight did not stop any process. Windows has many old scheduled tasks in
Ready/Disabled state; none was executed. Their existence is not active workload
or permission to resume campaign jobs.

## Existing Windows launch mechanisms

The existing `274bot-BotTest-native-server-20260907a` task is **Ready**, with:

- Executable: Windows PowerShell.
- Arguments: `-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "C:\Users\BotTest\274bot-server-4c95f87\run-native-server.ps1"`.
- Working directory: `C:\Users\BotTest\274bot-server-4c95f87`.

The orchestrator can inspect that already existing script before starting this
same fixture when ready for the authorized native smoke. This preflight did not
read private configuration or start it. Check world-ready and listener ownership
after any start. The native-server historical report identifies this as the
isolated server, with its own local RSA key and runtime-content prerequisites.

Native build setup is preserved in
`diagnostics/windows-native-20260907a/orchestrator-scripts/native-tests-952.ps1`:
prepend Austen `.cargo\bin`, NASM, VS BuildTools CMake bin, and Python314 to PATH;
set `CARGO_TARGET_DIR=C:\Users\Austen\274bot-campaign\target-native` and
`CARGO_BUILD_JOBS=4`. Use fresh selected source/output directories, not that
script's historical archives or fixed source labels. Its test argv uses
`--locked --release` and serial tests. Keep the compiler cache separate from
fresh evidence and copied immutable executables.

The active BotTest console can run panel and TUI through an Interactive,
Limited scheduled-task principal, as in
`windows-lazy-upload-controls/launch-panel-lazy-upload.ps1`. Use the existing
launcher pattern with the new binary and a fresh task/output ID; the old script
hardcodes old binaries, N16 controllers and historical preflight hashes, so do
not run it verbatim. `native-platform-functional-report.md` points to the working
ConPTY path (120x40); real TUI input writes alone do not establish UI latency.
Retain the process handle before waiting so exitCode is real, not the original
first-panel launcher's null receipt.

## Bounded test and live runbook

1. Build the actual selected sources with the affected tests listed in the
   approved assessment. Ordinary tests precede live runs; client integration
   targets must run separately from host tests. On Windows include socket park,
   host/profile home selection, process metrics, NUL fixture, host-play memory
   feature and TUI tests. Existing focused commands include
   `cargo test -p script --lib isolated_env::` and standalone client
   `cargo test -p client --lib bot_target::`. Resolve feature names against the
   extracted manifests rather than copying excluded profiling feature flags.
2. Bake a fresh pack **on the selected source**, to fresh output paths. Current
   defaults and explicit CLI are `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR]
   [CONFIG_JAG]`. On macOS verified inputs are
   `/Users/acfrazier/experiments/Server/content/maps`, sibling
   `scripts/doors/configs`, and engine `data/pack/config`. Use:

   ```sh
   ENGINE_DIR=/Users/acfrazier/experiments/Server/engine \
   NAV_PACK=/absolute/fresh-output/274bot.navpack \
   NAV_FLAGS=/absolute/fresh-output/274bot.navflags \
   cargo run --locked --release -p nav --bin nav-pack
   ```

   Preserve input identities and resulting hashes; do not overwrite operator
   defaults. Campaign example `crates/nav/examples/check_bank_return.rs` checks
   that Door1530 no longer jumps `(2656,3292)` to `(2651,3292)` and bank return
   remains routable. Run this existing campaign checker against the fresh pack
   (`NAV_PACK=... cargo run --release -p nav --example check_bank_return`), while
   recording that the **baker**, not merely the checker, was selected code.
3. The exact existing contested-door proof is:

   ```sh
   LIVE=1 BOT_TARGET=local NAV_PACK=/absolute/fresh-output/274bot.navpack \
   NAV_FLAGS=/absolute/fresh-output/274bot.navflags \
   cargo test -p e2e --test nav_door -- --ignored --test-threads=1 --nocapture
   ```

   Repeat only the approved opposite login order with
   `BOT_NAV_DOOR_REVERSE_LOGIN=1`. This test mints per-run accounts, waits for
   both players `ingame && scene_state==2`, keeps the closer active each update,
   and uses an unchanged 180-second terminal bound. The headed counterpart is
   `panel-play --live script_nav_door`. **Native path caveat:** current
   `crates/e2e/tests/common/mod.rs::options()` still constructs
   `HOME/experiments/Server/engine/data/pack/client`; ENGINE_DIR does not override
   that particular test helper. macOS has this layout; do not assume the same
   direct test command works against Windows/Concord fixture paths.
4. Direct retained frontend harness uses `LIVE=1`, `BOT_TARGET=local`,
   `ENGINE_DIR` to the appropriate fixture root, `RS2B0T` to the actual script
   checkout, fresh `BOT_MEMORY_OUTPUT`, `BOT_MEMORY_N`, `BOT_MEMORY_WORKLOAD`,
   warmup/observe settings, and sustain for active workload. Existing native
   settings use N1, 30-second warmup, 120-second observation and normal teardown;
   the separate clean N1/N32 comparison should freeze one matched workload,
   allocator/render setting, duration and cache/pack before either role runs.
   Public RSA values may be supplied using existing LOGIN_RSAN/LOGIN_RSAE if
   the runtime cannot read its own local fixture key. Never transfer private keys.
5. Campaign `docs/memory/run_diagnostic.py` supports
   `tui 1 active --binary ... --sustain --warmup 30 --observe 120 --no-diagnostics`
   (and N32); `--build-manifest ... --build-role reference|candidate` binds
   immutable binaries/fixture hashes. Default TUI uses a real PTY. Reuse only
   features the selected executable retains. This current launcher has owner/
   accounting imports and profile flags excluded from shipping, so check its
   availability separately and do not add those dependencies to product merely
   to satisfy the launcher. Main lacks the new harness: an identical, justified
   test-only driver/fixture is needed before calling main/candidate comparable.
   A bank-stalling main run is correctness evidence, not a valid CPU baseline.
6. Exact bank evidence needs inventory 22 after withdrawal, bank close and
   subsequent pickpocket/progress after returning; a bank-trip counter alone is
   insufficient. The historical native-server report describes `3 -> 22` food,
   `AnswerCount 19`, close, then renewed steals. Useful progress, current RSS and
   process CPU are required for comparison; keep peak RSS separate. Input probes
   alone cannot resolve response-time concerns. Run simple keyboard/focus/Stop/
   restart observations and record their limits if detailed latency fields were
   deliberately excluded.
7. Panel diagnostics support `--focused-one`, `--nav-captures`, and separately
   `--cpu-fallback`. Inspect actual screenshots for scene/minimap freeze, modal
   pixels and GPU→CPU→GPU switching in the active native desktop; unit readback
   and PNG existence do not replace visual inspection. macOS desktop and current
   Windows BotTest console are available mechanisms. Linux desktop proof is not
   presently available. A TUI run with render policy none proves no GPU path.

## Explicit gaps and known failures

- **Controlled stun:** no dedicated live stun scenario/test was found in the
  existing scenario/e2e/host-play test sources. `stun-recovery.md` says its fleet
  had zero StunDeferred/StunResumed events. Unit tests prove eleven distinct
  player-update ticks and one recovery bound; a normal new fleet run cannot
  establish this path unless it actually triggers. A small targeted existing
  harness exercise remains integration work, or the claim stays unproven.
- Windows server must be started before native gameplay; it is installed but
  currently stopped. Linux default compiler selection and disk/cache limits
  need handling with the already installed compiler. Linux native desktop panel
  remains unavailable without a session; Xvfb is only an offscreen option.
- Historical Windows/Linux GPU shade-boundary failure remains unresolved.
  Windows unreachable-CRC test historically exceeds its five-second assumption
  (~21 s). The selected modal fix addresses the formerly black modal; do not
  report that older modal failure as current without running selected tests.
- Existing source/binary labels, old screenshots, old scheduled tasks and old
  test logs cannot stand in for the extracted combined build's proof.

## Final Grok review mechanism

Verified `/Users/acfrazier/.hermes/profiles/branchreviewer/config.yaml` contains
default `grok-4.6`, provider `xai-oauth`, reasoning `medium`. Existing wrapper
`/Users/acfrazier/.local/bin/branchreviewer` executes `hermes -p branchreviewer`;
its `chat --help` succeeds. A fresh, same-profile one-shot can be run without
model/provider overrides:

```sh
/Users/acfrazier/.local/bin/branchreviewer chat --oneshot \
  --in /absolute/selected/checkout --query-file /absolute/frozen-review-brief.md
```

Give exact main/selected host and client SHAs, frozen diffs, the approved scope,
tests/live evidence and unresolved gaps. Verify actual model/provider in the
completed receipt, and require a verdict on this combination. No review was
dispatched here. `docs/execution.md` retains the separate whole-branch board-card
workflow if root chooses Kanban. The user made delegation optional for this
extraction; preserve final Grok review regardless of dispatch mechanism.

Generic Hermes help emitted a stale-gateway-module warning; the profile wrapper
help succeeded. Do not restart a gateway as part of this task. A fresh one-shot
process avoids assuming that an already-running gateway has current modules;
actual review completion must still be checked.
