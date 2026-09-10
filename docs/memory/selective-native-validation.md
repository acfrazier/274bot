# Selected native validation

Execution September 9–10, 2026 (local/UTC boundary). This validates the approved
selected combination with existing native environments. It is functional and
regression evidence, not a memory/CPU comparison or whole-platform certification.

**Current status:** Windows and Linux affected tests/builds and N1 active smokes
and the separately requested Windows Stop/restart lifecycle cell are complete.
The known GPU texture shade test remains failing on both platforms.

## Frozen source and inputs

- Host `9527cc636187fdfaaca29c3e4af14819dd40dbe9`, archived directly from the clean
  selected integration checkout.
- Client `daccb4ba3ff5f8d1fce5b3b9487a1f0576ad50ac`, separately archived from the
  exact pinned submodule revision.
- Fresh selected-baked navpack SHA-256
  `c9dca67bcdc8d0c8a598fb316279851658c1113e56bbb8dd5d81356cdf773f17`.
- Navflags SHA-256
  `92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb`.

The source/nav transfer archive was SHA-verified on Windows, Concord and the
existing Hyper-V builder. [Transfer manifest](evidence/native/9527cc636187/transfer-manifest.json)
records every input and the archive hash. No remote Git operations, source edits,
new VM, package/tool installation, private-key transfer or machine restart were
performed. Source archives contain no Git metadata, so the panel's embedded title
shows `unknown`; the exact source and executable hashes provide its provenance.

## Environments and build results

Windows uses the existing MSVC Rust 1.98.0 installation, four build jobs and
`C:\Users\Austen\274bot-campaign\target-native`. All ordinary TUI/panel and
`memory-profile-no-alloc` harness builds exited 0 with `--locked --release`.
The test runner copied each successful pair to immutable stage directories
before the next feature build could overwrite the cache outputs.

The operator clarified that Linux compilation belongs in the existing
`274bot-builder` Hyper-V VM because Concord has limited resources. The VM's
installed Rust 1.98.0 and development dependencies were reused with two jobs and
`/home/builder/274bot-campaign/target`. All six compilation stages exited 0,
including separate client integration tests, ordinary frontends and harness
frontends. No tests are counted as passed merely because their `--no-run`
compilation succeeded. Exact executable copies were transferred to Concord,
hash-verified there, and then executed natively.

Before that clarification, one bounded local Concord compilation exited 101:
`pkg-config` and OpenSSL development prerequisites were absent. The failed
[setup attempt](evidence/native/9527cc636187/linux-build-tests/results.json) is
preserved. It was resolved by using the intended existing builder, not by
provisioning Concord or substituting older binaries. Concord remains a 1.9 GiB
Linux runtime with no swap; it had approximately 11 GiB disk free after staging.

| Executable | Windows SHA-256 | Linux SHA-256 |
|---|---|---|
| Ordinary TUI | `d43803a145389f9301bb47d2b383cf88e3b571f1d86c6360b6e4a18148db6bc1` | `b291db09dcb4e3e1b6748c426822ecdc27699c0f52c05b3bb6bcc79e77caaabb` |
| Ordinary panel | `5419deb79ae4172e115449d114550c5ff864541ee596129771fca423232520ef` | `1b1c424439673b38a6ed1119dbe4ff1ce203d67ccca9c5636a4ecb0897827edf` |
| Harness TUI | `faee3822006650d3ed8bb976a79d3acaaa5c6c1656a0d656404f86646142f21a` | `c9a78dee88384a7ef30375f8fb60f84a1107551f8b922695688e553ccd3be089` |
| Harness panel | `58591a306af89d588307ae4d555a12c507b488a52ff3e435eafc04bd006d2cd8` | `9b41e6f70aabab8e37baf5de0e3062f10c1e2ac892f731b847a71ce512f430b6` |

Build/test argv, durations, source labels, copied executable sizes/hashes and
real exits are in [Windows receipts](evidence/native/9527cc636187/windows-test-evidence/)
and [Linux native receipts](evidence/native/9527cc636187/linux-test-evidence/).
Full builder compilation logs are retained in
`evidence/native/9527cc636187/linux-builder-logs.tar.gz`.

## Native test results

| Coverage | Windows | Concord Linux |
|---|---:|---:|
| Host library, including socket parking/control wakes and focused login | 124 passed | 124 passed |
| Host-play library with system-allocator harness, including resource/fixture/seed/lifecycle tests | 150 passed | 146 passed |
| TUI library with system-allocator harness | 86 passed | 87 passed |
| Panel library | 373 passed with local fixture server up | Only affected GameView subset run |
| Lazy CPU upload/GameView subset | 11 passed, also within full panel suite | 11 passed |
| Client operator-home selector | 5 passed | 5 passed |
| Client `iface_model`, including opaque modal and frozen-scene modal | 9 passed | 9 passed |
| Client `inject` | 7 passed with correct fixture cache | 7 passed |
| Client `login` | 10 passed | 10 passed |
| Client `player_info` | 2 passed | 2 passed |
| Client `seq_delay` | 1 passed | 1 passed |
| Client `zone` | 45 passed | 45 passed |
| Client `gpu_texture` | 9 passed, 1 failed | 9 passed, 1 failed |

The sole remaining selected GPU-suite failure on both platforms is the already
documented `gpu_textured_shade_scales_texel_brightness`: shade 16 expects about
223 but returns 255. It was not weakened, skipped or called a suite pass. The
Windows CRC unreachable-server deadline test was not rerun in this bounded
affected-target selection; its historical approximately 21-second result against
a five-second test bound remains unresolved and is a separate limitation.

Two setup failures and one invocation correction are retained:

1. Initial Windows full panel suite stalled at
   `flat_model_spawns_every_member_as_a_client` with the server off. Its comments
   assume detached startup workers, but existing `Session`/`Play` drops join them.
   Root's separate source review confirmed this predates the selected changes on
   main. The test process was stopped after the bounded 324-second stage. One
   recheck with the existing fixture server up passed all 373 tests in 136 seconds.
   This resolves the environment for this run; it does **not** make the suite
   hermetic or change the production HTTP timeout.
2. Client `inject::real_cache_round_trip` initially found a partial default Austen
   cache (versionlist present, `main_file_cache.dat` absent). One recheck with
   `ENGINE_DIR` pointing to the installed native fixture passed all seven tests.
3. A continuation command placed `--no-fail-fast` before Cargo's `test` subcommand.
   Cargo rejected it before running tests. The corrected argv and fresh output
   directory are separately preserved; this is an orchestration error, not a
   source failure or another gameplay attempt.

## Native active workload and visuals

The retained Rust harness ran directly with N1, active Thiever, sustain enabled,
30-second warmup, 120-second observation and ordinary 60-second teardown. Both
frontends used the system allocator; diagnostics were enabled. These are fresh
selected binaries, not campaign controller binaries. Runtime source/settings,
fresh navpack paths and executable hashes are recorded in each run's spec.

| Run | Actual runtime/frontend | Result |
|---|---|---|
| Windows TUI | BotTest console session 1; real ConPTY 120×40 | Exit 0, no timeout, 225.03 s; 60 terminal input writes with no transport error |
| Windows panel | BotTest console session 1; real native window, focused-one default GPU request | Exit 0, no timeout, 226.19 s; native window capture exit 0 |
| Concord TUI | Real Linux PTY 120×40; pre-existing local fixture service | Exit 0, no timeout, 240.73 s; 60 terminal input writes with no transport error |

All three runs observed ready/active N1 throughout their sampled observation
windows (115 Windows TUI, 118 Windows panel and 117 Concord samples), with no
diagnostic failures or slot errors. Qualification boundaries were Running,
`ingame=true`, `scene_state=2`. Each performed a bank trip and published exactly
22 Lobsters with the bank closed, then issued a new Pickpocket request after the
return WalkNear. The sampled details distinguish renewed requests from success:

| Run | Boundary steals | First bank-closed inventory 22 | Renewed Pickpocket request | Successful steal after return |
|---|---|---|---|---|
| Windows TUI | 3→9 | 136.679 s | 153.410 s | Not observed before the short window ended |
| Windows panel | 3→10 | 132.107 s | 148.310 s | Steal 10 at 150.350 s |
| Concord TUI | 5→9 | 148.047 s | 163.493 s | Not observed before the short window ended |

Times are elapsed from each run's origin. Windows sampling did not catch the
intermediate bank-open inventory-22 state; Concord caught it at 147.020 seconds.
The TUI boundary steal increases include successes before banking and are not
claimed as successful post-return steals. Each run's final teardown sample had
active slots, live V8 isolates, V8 used bytes and snapshot inflight bytes/capacity
all zero. [Active run summary](evidence/native/9527cc636187/active-run-summary.json)
links these conclusions to the retained raw run directories.

The [actual Windows panel capture](evidence/native/9527cc636187/observation-start.png)
was retrieved and read. It shows the Ardougne scene, minimap, inventory, script
overlay and native panels intact; the status panel says ingame scene 2 with one
running bot. The capture binds PID 8924, start time, session 1 and the selected
panel hash, and has PNG SHA-256
`c945307c03809e6d106e859881da9ba2a989a9cee1aec5e14a20e5ba0740e6a6`.
Samples show nonzero tracked GPU allocation while the panel runs. The actual
adapter/backend name was not independently recorded, so this does not establish
a particular NVIDIA/Intel or Vulkan/DX12 path.

There was no open modal in this capture, no controlled scene-freeze transition,
and no Windows live raster-switch stimulus. Those claims are not inferred from
the picture. Native modal and CPU/GPU upload readback tests passed; root performs
the selected macOS live transitions. Visible thieving stun in the picture also
does not prove the Traveller's bounded navigation-stun recovery path.

Concord had no existing desktop session. Its wgpu tests are offscreen readbacks,
and the live TUI had no renderer. No headed Linux panel, desktop scanout or GPU
frame-rate claim is made. Terminal input writes demonstrate transport activity;
they are not measured visible response times.

## Lifecycle and cleanup

The approved additional Windows lifecycle N1 run uses the same selected harness
TUI through ConPTY, 10-second warmup and 150-second observation, with the existing
Stop at observation +60 seconds and restart at +120 seconds, followed by normal
60-second teardown. It used the ordinary Thiever fixture without sustain and
without input probes. **Exit 0**, no timeout, 243.498 seconds total, completed
2026-09-10 03:09:24 UTC. There were no diagnostic failures or slot errors.

Observation started at 33.302 seconds. The script became Idle at 93.642 seconds
and Running again at 153.385 seconds, matching the scheduled +60/+120 actions.
Every sampled point during the stopped interior interval had active slots,
live V8 isolates, V8 used bytes and snapshot inflight bytes/capacity all zero.
The per-isolate snapshot peak-capacity metric reset from 282,136 to zero; after
restart a fresh isolate reported 284,552 and script ticks advanced 2→50 through
the remaining observation interval. Normal final Stop returned Idle at 184.216
seconds. These are lifecycle and ownership diagnostics, not measured RSS savings;
the affected regression test separately covers releasing retained builder
storage. [Lifecycle summary](evidence/native/9527cc636187/lifecycle-summary.json)
and [raw evidence](evidence/native/9527cc636187/windows-lifecycle-evidence/lifecycle/)
preserve the completion receipt and sample transitions.

Only the task-owned Windows fixture was started: Node PID 14340 in BotTest session
1, with fresh output under `selective-server-9527cc636187`. Its listeners were
verified on loopback 80/43594 and startup reported world ready with 7,322 static
NPCs. After all cells completed, its exact PID/start/path/session identity was
rechecked and its task was stopped. Independent process and TCP checks at
03:15:38 UTC confirmed PID 14340 absent and no owned TCP rows. The
[cleanup receipt](evidence/native/9527cc636187/windows-server-cleanup.json)
includes the launch identity and fresh world-ready logs. PowerShell's initial
`Stop-Process` returned a NullReferenceException; the exact scheduled-task stop
removed the server but its command remained blocked, so that task-owned cleanup
command was terminated after independent verification. This is intentional
fixture cleanup after successful frontend exits, not a gameplay failure.

The pre-existing Concord `274bot-concord-test-server.service` was neither started
nor restarted and remains running. Compiler caches, previous runs, old scheduled
tasks and VMs were not deleted or stopped by this work.
