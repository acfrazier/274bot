param()
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$stage='C:\ProgramData\274bot-Test\panel-36825a9'
if(-not (Test-Path $stage)){throw 'Frozen stage missing'}
$vm=Get-VM -Name '274bot-builder'
if([string]$vm.State -ne 'Off'){throw 'Builder VM is not off'}
if(Get-Process -Name cargo,rustc,panel-play,tui-play,qemu-system-x86_64,vmware-vmx -ErrorAction SilentlyContinue){throw 'Build/frontend/VM still running'}
$services=Get-Service DellTechHub,DellClientManagementService,ClickToRunSvc
if($services|Where-Object Status -ne 'Stopped'){throw 'Quiet services restarted'}
$drivers=Get-CimInstance Win32_VideoController|Select-Object Name,DriverVersion,PNPDeviceID
if(@($drivers|Where-Object {$_.Name -match 'NVIDIA' -and $_.DriverVersion -eq '32.0.16.1686'}).Count -ne 1){throw 'Unexpected NVIDIA hotfix driver'}
$launch=Get-Content 'C:\Users\BotTest\274bot-server-4c95f87\server-launch.json' -Raw|ConvertFrom-Json
$taskServerPid=[int]$launch.pid
$server=Get-CimInstance Win32_Process -Filter "ProcessId=$taskServerPid"
$listener=Get-NetTCPConnection -State Listen -LocalPort 43594
if(-not $server -or $server.Name -ne 'node.exe' -or @($listener|Where-Object OwningProcess -eq $taskServerPid).Count -ne 1){throw 'Fresh server identity not verified'}
if(Get-CimInstance Win32_Process|Where-Object {$_.Name -like 'Dell*' -or $_.Name -eq 'OfficeClickToRun.exe'}){throw 'Quiet process cleanup incomplete'}
Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class ConsoleSessionProbeN16 { [DllImport("kernel32.dll")] public static extern uint WTSGetActiveConsoleSessionId(); }'
$consoleSid=[ConsoleSessionProbeN16]::WTSGetActiveConsoleSessionId()
$sessions=@(& quser.exe)
if(@($sessions|Where-Object {$_ -match ('^\s*bottest\s+console\s+'+$consoleSid+'\s+Active')}).Count -ne 1){throw 'BotTest local console is not active'}
if(Get-Process LogonUI -ErrorAction SilentlyContinue|Where-Object SessionId -eq $consoleSid){throw 'Console is locked'}
$processes=@(Get-CimInstance Win32_Process|Select-Object ProcessId,ParentProcessId,Name,SessionId,CreationDate,CommandLine)
$lasso=@($processes|Where-Object {$_.Name -match '(?i)^(ProcessLasso|bitsum|LassoGovernor).*\.exe$'})
$lassoFiles=@()
foreach($p in @('C:\ProgramData\ProcessLasso\config','C:\ProgramData\ProcessLasso\config\prolasso.ini','C:\ProgramData\Bitsum')){if(Test-Path $p){$lassoFiles+=@(Get-ChildItem $p -File -Recurse -ErrorAction SilentlyContinue|ForEach-Object{[ordered]@{path=$_.FullName;length=$_.Length;sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()}})}}
$dxPath='C:\Users\BotTest\274bot-runs\display-routing-console-20260907\dxdiag-console.xml'
$dx=$null
if(Test-Path $dxPath){[xml]$x=Get-Content $dxPath -Raw;$dx=@($x.DxDiag.DisplayDevices.DisplayDevice|ForEach-Object{[ordered]@{cardName=$_.CardName;currentMode=$_.CurrentMode;driverVersion=$_.DriverVersion;hybridGraphicsGPU=$_.HybridGraphicsGPU;monitorName=$_.MonitorName}})}
$record=[ordered]@{adapter='Intel(R) Graphics';count=16;cells=@('focused-one','focused-plus-background');backgroundFps=1;utc=[DateTime]::UtcNow.ToString('o');consoleSessionId=$consoleSid;vm=($vm|Select-Object Name,@{n='State';e={[string]$_.State}},MemoryAssigned);quietServices=@($services|Select-Object Name,@{n='Status';e={[string]$_.Status}});drivers=$drivers;sessions=$sessions;processes=$processes;processLasso=[ordered]@{running=(@($lasso).Count -gt 0);processes=$lasso;files=$lassoFiles};dxdiag=$dx;performanceAcceptance=$false}
$record|ConvertTo-Json -Depth 8|Set-Content (Join-Path $stage 'preflight-panel-n16-intel.json') -Encoding UTF8
'Preflight passed: VM/builds/workloads stopped, quiet services observed, active unlocked BotTest console, Process Lasso sampled dynamically'
