# Standard-user Windows live proof — 2026-09-07

BotTest is a local standard account in Users and Remote Desktop Users, not Administrators. Both native interfaces ran in its interactive RDP session 3 with `isAdministrator=false`, completed the active N=1 Thiever observation, and exited 0 without reaching the launcher timeout. The prior Austen session remained signed in. This is functional/diagnostic evidence, not a quiet-machine claim or matched performance acceptance.

## Frozen runs

| | Panel | TUI |
| --- | --- | --- |
| Run name | bottest-panel-20260907a | bottest-tui-20260907a |
| Host/client source | 61b7b7d / 4b35300 | 952ba22 / 2b1af85 |
| PID / RDP session | 8600 / 3 | 20300 / 3 |
| Start UTC | 15:23:01.2860357 | 15:28:10.1315322 |
| End UTC | 15:26:52.6071922 | 15:31:54.8360390 |
| Exit / timeout | 0 / false | 0 / false |
| Binary SHA256 | 0fc3f79bb9b83c90cd2d41f19446fa0ea8e216a67dde95292f9de388c2a6a424 | 5addcb8bcdf1211834d0934e3ae10db4cfc0792183df1eeaa8034731f29cde17 |
| Observation boundaries (harness seconds) | 47.0683838–167.0993882 | 44.4326139–164.4575371 |
| Observation sample rows | 117 | 118 |
| All observed ready/active counts | 1 / 1 | 1 / 1 |
| Steals at boundaries | 1 → 8 | 3 → 15 |
| Eats at boundaries | 0 → 2 | 0 → 1 |
| Bank trips at boundaries | 0 → 0 | 0 → 1 |
| Completed script ticks | 70 → 270 | 69 → 269 |
| Resident WorkingSetSize range in observation, bytes | 481275904–484151296 | 179339264–185384960 |

The resident ranges describe distinct interfaces/workloads and are not a baseline/candidate comparison or a saving. Both builds use `memory-profile-no-alloc`; diagnostics, failure capture and render profiling were on. Scheduling, GPU-completion and responsiveness profiles were off. All diagnostic rows had null failure and null slot error. Both boundary clients were ingame=true and scene_state=2.

Panel matching per-slot renderer evidence was GPU, present, draw=true, full_rate=true. TUI matching per-slot renderer evidence was renderer_present=false, backend=null, draw=false, full_rate=false; its visible native console is the frontend. No actual panel adapter model is inferred from the separate adapter-selection probe.

## Banking evidence

The TUI diagnostic request history contains `Deposit { name: "Coins" }` and `Withdraw { name: "Lobster", action: "Withdraw X" }`. At observe-end the actual inventory contains 22 separate Lobster entries and 30 Coins; script paint says one bank trip, 15 steals and 22 food, state Running back near the guards. This supports one observed deposit/Withdraw-X/return cycle and subsequent steal. It is not a long-run banking/high-N qualification; final bank cache is closed/unloaded.

## Launcher and environment

Frozen binaries and public fixture assets were staged under `C:\ProgramData\274bot-Test`, read/execute for Users and full control for SYSTEM/Administrators. Each run wrote only inside `C:\Users\BotTest\274bot-runs`. Operator assets were initialized into BotTest's own `.274bot`, refusing overwrite. Password setup was completed locally by the operator; no credential is included in these artifacts.

Panel uses a visible normal window. TUI inherits a real Windows console from the scheduled PowerShell action (`-NoNewWindow` for its child; stdout stays attached to the console, stderr goes to file). These are direct diagnostic launchers, not the Unix-PTY managed runner. No ConPTY or measured input-latency claim.

The revised launchers retain the process handle before waiting, capture the actual child exit code, and have an eight-minute observation/process backstop plus task limit. They record source, binary hash, user, session, administrator status and timeout. The earlier Austen panel's null-exit-code receipt remains unchanged and is not used as this proof.

Both runs used the existing Mac game server via loopback-only SSH reverse forwards on Windows ports80/43594. No native Windows game_server PID existed for these runs. The Mac server and original user session were left running. No restart or user sign-out was performed.

## Cross-user sampler capability

Austen's SSH collector, source e425799 / native Python3.14, successfully sampled the exact BotTest panel PID8600: ten samples over two seconds, stable `windows_creation_filetime:134332681812860357`, nonzero current WorkingSetSize, GetProcessTimes CPU, summary ok, exit0. This proves OpenProcess access for the tested account/token/process combination only. It is not managed six-role accounting, universal cross-user permission, or overhead qualification.

The first attempted collector command used incorrect `--pid`/`--output` flags for a positional CLI and exited2 before sampling. Its receipt/log remain under `cross-user-sampler-8600`; the corrected positional invocation is separately preserved under `cross-user-sampler-8600b`. No failure was overwritten.

## Artifacts and remaining work

Directory: `diagnostics/windows-bottest-20260907a/`; file hashes in `artifact-sha256.json`.

- Panel + collector transfer archive SHA256: `5fcd3ab8d009d0cdc1bd9d052405f6736f69d75538d8aafda2d31c7da850c1cb`.
- TUI transfer archive SHA256: `b7bae58ed0152ad536470a096785cf03ac6094214d9e3665b7302c414c6c1abd`; extracted separately under `tui-bundle/`.

Original JSONL, qualification and diagnostics sidecars, stderr, launch/completion JSON and launcher scripts are preserved. Account setup receipts in the archive are creation-time snapshots (disabled then); actual successful run identity is in launch.json.

Native full client suite still has three unresolved failures as detailed in `windows-native-first-proof.md` and `windows-native-additional-failures.md`. Windows managed stop/ownership/PTY/module binding and local-server bring-up remain in progress. No matched savings, full Windows green, cross-host accounting acceptance, or final whole-branch Grok-4.6 approval is claimed.
