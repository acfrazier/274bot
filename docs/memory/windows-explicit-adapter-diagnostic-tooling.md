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

- `run-managed-panel-36825a9-console-nvidia.py` — independent NVIDIA N1 runner; requests `NVIDIA GeForce RTX 5060 Laptop GPU`.
- `run-managed-panel-36825a9-console-intel.py` — independent Intel N1 runner; requests `Intel(R) Graphics`.
- `preflight-console-36825a9.ps1` — parameterized native preflight. It observes the live process list and Process Lasso processes/config files; it does not hardcode `running=true`. It records optional Process Lasso config hashes when files exist.
- `launch-console-36825a9.ps1` — copies one selected runner to the frozen stage and registers/starts one uniquely named interactive BotTest task.
- `poll-console-36825a9.ps1` — reads task/completion state and startup attribution; use `-Final` to require exit 0 and exact actual/requested adapter proof.

SHA-256 hashes:

```text
4d10d4c2058ab13443a298b6788dad1c5a00c77c3a3c5264498e52610c056ad9  run-managed-panel-36825a9-console-nvidia.py
6ba20f9ed0af44764e2cd0c2c76d3244f38372f181c45054c8eb1b7898a25214  run-managed-panel-36825a9-console-intel.py
1bde2c289e2800f13e721fc7f0c616a4224665a2de9e5e572cae99cfc0fd7834  preflight-console-36825a9.ps1
bf9356aaeaf3082fd2dcdce320a52d5a48ddad38845e74fd1d9bdf577417b54e  launch-console-36825a9.ps1
73e2cf97184b338e5c1818fd74e7b870c792c74cc06f0c435528c872b1b090c0  poll-console-36825a9.ps1
```

## Root transfer and execution

Use the already approved key-only SSH path/alias to the Windows host; do not put credentials in this report. From the Mac operator checkout, transfer the five files (replace `WINDOWS_SSH_TARGET` with the configured host alias):

```sh
scp /tmp/274bot-windows-setup-20260907/run-managed-panel-36825a9-console-nvidia.py WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/run-managed-panel-36825a9-console-intel.py WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/preflight-console-36825a9.ps1 WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/launch-console-36825a9.ps1 WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
scp /tmp/274bot-windows-setup-20260907/poll-console-36825a9.ps1 WINDOWS_SSH_TARGET:/C:/Users/BotTest/Downloads/
```

On the local BotTest console, after verifying the transferred hashes and preserving the existing server/session state:

```powershell
Set-Location C:\Users\BotTest\Downloads
.\preflight-console-36825a9.ps1 -Adapter nvidia
.\launch-console-36825a9.ps1 -Adapter nvidia -RunnerPath .\run-managed-panel-36825a9-console-nvidia.py
.\poll-console-36825a9.ps1 -Adapter nvidia
# Wait for the task's normal 30 s warmup + 120 s observe + 60 s teardown.
.\poll-console-36825a9.ps1 -Adapter nvidia -Final
```

Then, only after the NVIDIA task has completed and its uniquely named task/process/output is gone:

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
- No persistent power change is made. `SetThreadExecutionState` is not used by these scripts.
- No result from either requested adapter exists yet. Root must preserve raw receipts and separately diagnose any adapter-selection, fixture, binding, or workload failure.

## Local verification

Both Python runners passed `python3 -m py_compile` on macOS. Their embedded Windows-only imports/API calls were not executed locally. PowerShell syntax/runtime and both native diagnostics remain for root verification on Windows.
