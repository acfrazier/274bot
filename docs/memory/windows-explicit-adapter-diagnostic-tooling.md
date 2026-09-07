# Windows explicit-adapter console diagnostic tooling

Prepared 2026-09-07 for root execution only. This is operator tooling, not production code, and no Windows transfer, task launch, live workload, or live result is claimed from this card.

## Frozen inputs

Both runners use the same staged binary:

- `C:\ProgramData\274bot-Test\panel-36825a9\panel-play.exe`
- SHA-256: `87c24665563ef8a55751244a52f7d25c7edf68e3117e84f2ff464add596c71fb`
- host source: `36825a9f0a07a19610439c691f6dfe3d2e1cbf48`, source manifest SHA-256 `cf0175c549c573a37ebd8af31242b2eb817946a716f76b2785d3d4d8a76f3bca`
- client source: `5ee9b6efb2342452ceeb7958f864fd68d0daadd1`, source manifest SHA-256 `f71f99afabf29025a91179554792c35343093643a1f386cf1676d9bf2dedca3d`
- fixture, navigation pack/flags, script catalog, server identity, profiles, and settings match the prior console runner.

The panel implementation records `adapter_name` and `requested_adapter_name` in its attribution `startup` event. The poll helper requires both to equal the requested exact name before a final result is accepted.

## Files staged locally

These files are in `/tmp/274bot-windows-setup-20260907/` and intentionally remain operator-local rather than tracked repository tooling:

- `run-managed-panel-36825a9-console-nvidia.py` — independent NVIDIA N1 runner; requests `NVIDIA GeForce RTX 5060 Laptop GPU`, using the preserved `C:\Users\BotTest\274bot-workspaces\e3188e2\host` tooling root.
- `run-managed-panel-36825a9-console-intel.py` — independent Intel N1 runner; requests `Intel(R) Graphics`, using the preserved `C:\Users\BotTest\274bot-workspaces\e3188e2\host` tooling root.
- `preflight-console-36825a9.ps1` — parameterized native preflight. It observes the live process list and Process Lasso processes/config files; it does not hardcode `running=true`. It records optional Process Lasso config hashes when files exist.
- `launch-console-36825a9.ps1` — copies one selected runner to the frozen stage and registers/starts one uniquely named interactive BotTest task.
- `poll-console-36825a9.ps1` — reads task/completion state and startup attribution; use `-Final` to require exit 0 and exact actual/requested adapter proof.

SHA-256 hashes:

```text
d21eb61cb84af448fe524fb26ef97c31e2dfc7b4df9708d0afb5e3f392294b12  run-managed-panel-36825a9-console-nvidia.py
9d4db815aaf81a6575c00d804fb64567958ef4be4e41694c5eb7512a13ca2b4c  run-managed-panel-36825a9-console-intel.py
1bde2c289e2800f13e721fc7f0c616a4224665a2de9e5e572cae99cfc0fd7834  preflight-console-36825a9.ps1
d4c6bab4950da03d53450e598bcf1aa16ae9be6fa16f47f11714ce3e42e352b7  launch-console-36825a9.ps1
ee09df93d1d39d6fe29118bbf11fa300669be7267b00fbc7484ceefb3673f55d  poll-console-36825a9.ps1
```

## Root transfer and execution

Use the already approved key-only SSH administrator (Austen) path/alias to the Windows host; do not put credentials in this report. From the Mac operator checkout, transfer the five files (replace `WINDOWS_SSH_TARGET` with the configured host alias):

```sh
scp /tmp/274bot-windows-setup-20260907/run-managed-panel-36825a9-console-nvidia.py WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/run-managed-panel-36825a9-console-intel.py WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/preflight-console-36825a9.ps1 WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/launch-console-36825a9.ps1 WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/poll-console-36825a9.ps1 WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
```

On the Windows host through the existing SSH administrator (Austen), after verifying the transferred hashes and preserving the existing server/session state, run the preflight and launch helpers. They inspect/register the limited `BotTest` task; they must not be run as limited `BotTest` because VM, process, firewall/session, and scheduled-task operations require administrator rights. The helpers deliberately use explicit `C:\Users\BotTest` paths for BotTest output/logs rather than the administrator's `USERPROFILE`:

```powershell
Set-Location C:\Users\BotTest\Downloads
.\preflight-console-36825a9.ps1 -Adapter nvidia
.\launch-console-36825a9.ps1 -Adapter nvidia -RunnerPath .\run-managed-panel-36825a9-console-nvidia.py
.\poll-console-36825a9.ps1 -Adapter nvidia
# Wait for the task's normal 30 s warmup + 120 s observe + 60 s teardown.
.\poll-console-36825a9.ps1 -Adapter nvidia -Final
```

Then, only after the NVIDIA task has completed and its uniquely named task/process is stopped (preserve all NVIDIA raw output; do not delete or require the prior output directory to be gone):

```powershell
.\preflight-console-36825a9.ps1 -Adapter intel
.\launch-console-36825a9.ps1 -Adapter intel -RunnerPath .\run-managed-panel-36825a9-console-intel.py
.\poll-console-36825a9.ps1 -Adapter intel
# Wait for the task's normal 30 s warmup + 120 s observe + 60 s teardown.
.\poll-console-36825a9.ps1 -Adapter intel -Final
```

Do not reuse task names or output directories. Inspect the final `startup` record before comparing: `adapter_name` and `requested_adapter_name` must both be the exact requested string. A missing startup record, a name mismatch, or an unsuccessful completion is a diagnostic failure, not a fallback to default HighPerformance selection.

## Preserved conditions and known diagnostic context

- Diagnostic arguments preserve `30` warmup, `120` observe, `60` teardown, focused-one, render/GPU completion, navigation captures, scheduling, responsiveness/fine responsiveness, and failure capture.
- Native preflight records VM Off, Dell/Office services stopped, active unlocked local console, current process observations, server identity, drivers, optional Process Lasso process/config evidence, and optional existing dxdiag data. It does not change startup types, power plans, BIOS, display routing, or Process Lasso configuration.
- Existing dxdiag context remains diagnostic only: completed 0, BotTest console session 2, Intel drive/internal 2560x1600@240; NVIDIA mode was previously unknown. The helper records any available XML but does not promote those facts into adapter-selection proof.
- The controller holds the display/system awake for its lifetime with bounded `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)` and releases it in `finally`; no persistent power-policy change is made.
- No result from either requested adapter exists yet. Root must preserve raw receipts and separately diagnose any adapter-selection, fixture, binding, or workload failure.

## Local verification

Both Python runners pass `python3 -m py_compile` on macOS. Their embedded Windows-only imports/API calls were not executed locally. PowerShell syntax/runtime and both native diagnostics remain for root verification on Windows. The poll helper follows each managed-cell `receipt.json` `run_dir` to the raw diagnostic directory and parses both pure JSON and `[panel-render-attribution] {json}` lines in `run.log`; it does not assume the raw log is under managed output.
