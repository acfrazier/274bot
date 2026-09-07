# Native Windows first proof — 2026-09-07

Status: platform qualification in progress. This is diagnostic and functional evidence, not accepted matched CPU/RSS savings. Full client suite remains failing. No production shading behavior was changed to obtain a pass.

## Environment and provenance

Windows 11 IoT Enterprise LTSC x64 build 26100, 24-core/24-thread Intel 275HX, NVIDIA RTX 5060 Laptop and Intel Graphics. RDP is the operator's primary Windows usage mode. The initial panel ran in Austen's interactive RDP session 1 using a limited scheduled-task token. Driver reported by the separate matching-request GPU probe: NVIDIA Vulkan 616.56. That probe does not directly identify the adapter inside the live panel.

Native tools installed without restart: VS 2022 Build Tools 17.14.37614.0 / MSVC and Windows SDK (installer exit 0, restartRequired false), Rust/Cargo 1.98.0 x86_64-pc-windows-msvc, NASM 3.02. Python 3.14 and Git were already installed. Builds use four jobs. No WSL features have been enabled. Restart requires explicit operator go-ahead.

Host freezes: 9cdd4a2 (socket parking), c8b9334 (Windows resource counters), 61b7b7d (Windows operator home and null-device test fixtures). Corresponding client refs: 35f1c13, then 4b35300. The standalone client lockfile needed its direct windows-sys dependency recorded; client 2b1af85 / host 952ba22 records that one-line lockfile correction. Earlier host builds used --locked; earlier standalone client tests did not. Final locked client coverage is a separate follow-up.

Raw first-stage artifacts: `diagnostics/windows-native-20260907a/`. Transfer archive SHA256: `d0d3ee8c055be94469764de9052aee600b732e72294cf714f2ab71a3b296c125`. Includes original failures, receipts, full run sidecars, frozen Python sources, and GPU diagnostic probe source/lockfile. Diagnostic artifacts are not an acceptance bundle.

## Build and tests

| Freeze / test | Observed result |
| --- | --- |
| 9cdd4a2 initial release panel | Exit 101: missing NASM |
| Same source after installing NASM | Release panel build exit 0 |
| c8b9334 host unit | 179 passed, 1 ignored |
| c8b9334 host-play unit | 119 passed |
| c8b9334 host-play memory feature | 175 passed, 5 failed opening Unix /dev/null on Windows |
| 61b7b7d host-play memory feature after cfg-specific NUL fixture | 180 passed |
| 61b7b7d panel library resource tests | 6 passed (earlier bin-filter command ran zero tests and is not proof) |
| 61b7b7d script isolated_env | 8 passed |
| Client 4b35300 bot_target | 5 passed in the separate client workspace |
| 61b7b7d panel memory-profile-no-alloc build | Exit 0 |
| Client 35f1c13 full test invocation | Aborted at gpu_texture: 9 passed, 1 failed; later targets were not run |
| Python collector 573aca1, native Python 3.14 | 71 tests, 2 failures, 1 skip; repair in progress |

Linker LNK4098 libcmt default-library conflict warning was present in the native build. It has not been silently suppressed. The Python failures were real CLI bugs: __main__/imported SampleError identity mismatch escaped the required FAIL handler, and a Unicode arrow in argparse help could not encode to Windows cp1252. Mac injected tests had passed before native execution exposed these failures.

## First native panel observation

`rdp-panel-smoke-20260907a`, host 61b7b7d / client 4b35300, PID 8888, interactive session 1. Binary SHA256 `0fc3f79bb9b83c90cd2d41f19446fa0ea8e216a67dde95292f9de388c2a6a424`. Started 14:49:22.8312917 UTC; launcher completion 14:53:30.3737806 UTC. One active Thiever, requested focused-one GPU rendering, 30-second warmup and 120-second observation followed by ordinary teardown. Diagnostics, failure capture and render profiling were enabled; allocator counting, scheduling, GPU-completion and responsiveness profiling were disabled.

The game server ran on the Mac through SSH reverse forwards bound only to Windows loopback ports 80 and 43594. This is explicitly a cross-host server placement. It is not a local Windows server run and does not satisfy managed six-role same-host accounting. Only the server public RSA parameters were transferred; no server private key was copied.

Both qualification boundaries show ingame=true, scene_state=2, Running with no error, and an available matching per-slot GPU renderer, draw=true/full_rate=true. Steals increased 4 to 12, coins 120 to 360, and food 4 to 2 with two recorded eats. Completed script tick increased 97 to 298. There were 119 observation sample rows, all ready=1 and active=1; zero diagnostic failure/slot-error rows across the full run. Boundary-adjacent diagnostic XP increased 101520 to 101894. Bank trips remained zero; this short run does not qualify banking.

Observed current Windows WorkingSetSize ranged 476188672–482426880 bytes during observation. Those are diagnostic resident samples, not a saving. PeakWorkingSetSize is recorded separately. No matched baseline, instrumentation-overhead acceptance, full latency proof, or managed accounting was obtained here.

stderr contains `PASS: memory panel observation complete`, the process ended without hitting the timeout, and the scheduled task result was 0. **Launcher limitation:** completion.json contains exitCode=null because the PowerShell Process wrapper did not retain the exit-code handle; task result 0 is not independent proof of binary exit 0. Keep this original run as functional evidence and correct exit-code capture for later runs. Do not retrofit its receipt.

## GPU failure classification

Exact isolated native test also failed: shade 16 expected 223, got 255. The same unchanged test passes on the Mac. A separate frozen diagnostic reproduces mixed adjacent brightness bands at boundary shades (16/32/48/64/80/96/112) while adjacent non-boundary shades (17/33/49/65/81/97/113) are uniform. Mesh input shades are exact. This agrees with the previously documented Linux boundary failure and supports floating interpolation/truncation as the cause; direct fragment float values were not instrumented. It does not support a claim that the whole scene is fully bright or that readback is stale.

See `windows-gpu-shade-diagnosis.md` and the probe artifacts. No tolerance widening, skipped assertion, production shader redesign or full-suite green claim.

## Dedicated account preparation

At operator invitation, created local standard user BotTest, added Users and Remote Desktop Users, verified absent from Administrators. The account is disabled pending a password entered locally by the operator. Activation script prompts using Read-Host -AsSecureString. It neither signs out nor restarts the existing session. Shared frozen panel binary and public test assets are under C:\ProgramData\274bot-Test with SYSTEM/Administrators full control and Users read/execute; private per-user output and operator assets are initialized only inside BotTest. No FFXIV or startup service changes were made.

A separate account isolates user startup configuration. It does not by itself remove machine-wide services or processes in another signed-in session. Any sign-out needed for a quieter measurement must wait for operator permission. Fresh-account live proof, TUI native build/live, complete locked client target coverage, Windows matched measurements and final whole-branch Grok-4.6 review remain outstanding.


## 15:05 UTC locked coverage follow-up

Host 952ba22 / client 2b1af85: TUI memory-profile-no-alloc release build exited 0; TUI library tests passed 91/91. Frozen TUI SHA256 `5addcb8bcdf1211834d0934e3ae10db4cfc0792183df1eeaa8034731f29cde17`, also staged read-only for BotTest.

Separate client `cargo test --locked --release -p client --no-fail-fast -- --test-threads=1` completed 66 result blocks: 763 passed, 3 failed, 0 ignored, exit 101. All targets finished; the failures are `gpu_texture::gpu_textured_shade_scales_texel_brightness` (shade16 expected223 got255), `iface_model::gpu_main_modal_rect_is_opaque_over_the_scene` (expected 0x336699, got black), and `maininit::crc_unreachable_sets_error_loading_in_bounded_time` (20.9751447s exceeds test bound). The two newly exposed failures are under isolated native reproduction and source diagnosis (t_0fbec043). They have not been waived, modified or classified as harmless.

Raw follow-up logs/receipt were copied into the same first-proof artifact directory as separate files prefixed `test-952ba22-` and `native-tests-952ba22.jsonl`. The earlier transfer archive hash covers only the original first-stage bundle; these later files were transferred separately. The first smoke remains unchanged. A separate Windows launcher probe now correctly captures child exit codes 0 and 7 by retaining the process handle before waiting; that does not repair the original null exit-code receipt.


## 15:08 UTC collector repair and isolated failures

Python repair e425799 received per-task Grok-4.5 approval and passed the actual Windows Python3.14 run: 73 tests, 1 skip, exit0. The skip is the non-Windows-only backend rejection test; all three live Windows tests (own PID, missing PID, routed system backend) ran and passed. Sources, native log and SHA receipt are preserved separately under `python-collector-e425799/`; original573aca1 sources/failure log remain intact.

Both additional client failures reproduce in exact isolated fresh-process runs without concurrent native build/live load. Modal test: exit101, 2.94s, black instead of 0x336699; executable SHA256 `acf3389f8fe3cf169f6f3922d044a69e38a95b3fd49f0fec2f32a876a3e8f73f`. CRC test: exit101, 20.9661454s measured, executable SHA256 `8bc750fe6cfd3200164bd7cd5029ece91af293d1484b2c546a917d600bfa65d1`. Original isolated logs and receipt are preserved separately in the artifact directory.

Grok-4.5 approved the root home/NUL/standalone-lockfile changes and first-panel evidence in task t_bc2fc7da (report `windows-native-first-review.md`). That review preceded the later full-target/collector follow-up additions here. It is not a final whole-branch Grok-4.6 review or full Windows qualification.
