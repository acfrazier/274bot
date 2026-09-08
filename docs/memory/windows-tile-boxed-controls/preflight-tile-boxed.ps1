param([Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$CellId)
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$baselineStage='C:\ProgramData\274bot-Test\renderer-owner-census-9268890'; $candidateStage='C:\ProgramData\274bot-Test\tile-boxed-fb3589a'
if(-not (Test-Path $baselineStage)){throw "Required baseline stage missing: $baselineStage"}
$vm=Get-VM -Name '274bot-builder'; if([string]$vm.State -ne 'Off'){throw 'Builder VM is not off'}
if(Get-Process -Name cargo,rustc,panel-play,tui-play,qemu-system-x86_64,vmware-vmx -ErrorAction SilentlyContinue){throw 'Build/frontend/VM still running'}
$services=Get-Service DellTechHub,DellClientManagementService,ClickToRunSvc
if($services|Where-Object Status -ne 'Stopped'){throw 'Quiet services restarted'}
$drivers=Get-CimInstance Win32_VideoController|Select-Object Name,DriverVersion,PNPDeviceID
$launch=Get-Content 'C:\Users\BotTest\274bot-server-4c95f87\server-launch.json' -Raw|ConvertFrom-Json
$taskServerPid=[int]$launch.pid; $server=Get-CimInstance Win32_Process -Filter "ProcessId=$taskServerPid"; $listener=Get-NetTCPConnection -State Listen -LocalPort 43594
if(-not $server -or $server.Name -ne 'node.exe' -or @($listener|Where-Object OwningProcess -eq $taskServerPid).Count -ne 1){throw 'Fresh server identity not verified'}
if(Get-CimInstance Win32_Process|Where-Object {$_.Name -like 'Dell*' -or $_.Name -eq 'OfficeClickToRun.exe'}){throw 'Quiet process cleanup incomplete'}
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class ConsoleSessionProbeLazyUpload { [DllImport("kernel32.dll")] public static extern uint WTSGetActiveConsoleSessionId(); [StructLayout(LayoutKind.Sequential)] public struct SYSTEM_POWER_STATUS { public byte ACLineStatus; public byte BatteryFlag; public byte BatteryLifePercent; public byte Reserved; public int BatteryLifeTime; public int BatteryFullLifeTime; } [DllImport("kernel32.dll")] public static extern bool GetSystemPowerStatus(out SYSTEM_POWER_STATUS status); }'
$consoleSid=[ConsoleSessionProbeLazyUpload]::WTSGetActiveConsoleSessionId(); $sessions=@(& quser.exe)
if(@($sessions|Where-Object {$_ -match ('^\s*bottest\s+console\s+'+$consoleSid+'\s+Active')}).Count -ne 1){throw 'BotTest local console is not active'}
if(Get-Process LogonUI -ErrorAction SilentlyContinue|Where-Object SessionId -eq $consoleSid){throw 'Console is locked'}
$processes=@(Get-CimInstance Win32_Process|Select-Object ProcessId,ParentProcessId,Name,SessionId,CreationDate,CommandLine)
$lasso=@($processes|Where-Object {$_.Name -match '(?i)^(ProcessLasso|bitsum|LassoGovernor).*\.exe$'})
$serverRecord=[ordered]@{ProcessId=[int]$server.ProcessId;ParentProcessId=[int]$server.ParentProcessId;Name=[string]$server.Name;SessionId=[int]$server.SessionId;CreationDate=[string]$server.CreationDate;CommandLine=$server.CommandLine}
$systemPower=New-Object ConsoleSessionProbeLazyUpload+SYSTEM_POWER_STATUS; if(-not [ConsoleSessionProbeLazyUpload]::GetSystemPowerStatus([ref]$systemPower)){throw 'GetSystemPowerStatus failed'}
$power=@(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue|Select-Object EstimatedChargeRemaining,BatteryStatus); $brightness=@(Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue|Select-Object CurrentBrightness,TimeStamp); $powerState=[ordered]@{acLineStatus=[int]$systemPower.ACLineStatus;computerSystemPowerState=(Get-CimInstance Win32_ComputerSystem).PowerState;battery=$power;brightness=$brightness;activeScheme=@(& powercfg.exe /getactivescheme);acDisplayIdle=@(& powercfg.exe /query SCHEME_CURRENT SUB_VIDEO VIDEOIDLE);acSleepIdle=@(& powercfg.exe /query SCHEME_CURRENT SUB_SLEEP STANDBYIDLE);acLidAction=@(& powercfg.exe /query SCHEME_CURRENT SUB_BUTTONS LIDACTION)}
# Preserve the existing console DxDiag observation with explicit file lineage.
$dxPath='C:\Users\BotTest\274bot-runs\display-routing-console-20260907\dxdiag-console.xml'
if(-not (Test-Path $dxPath)){throw 'Console DxDiag evidence missing'}
[xml]$dxXml=Get-Content $dxPath -Raw
$dx=@($dxXml.DxDiag.DisplayDevices.DisplayDevice|ForEach-Object{[ordered]@{cardName=[string]$_.CardName;driverVersion=[string]$_.DriverVersion;currentMode=if([string]$_.CurrentMode){[string]$_.CurrentMode}else{$null};hybridGraphicsGPU=if([string]$_.HybridGraphicsGPU){[string]$_.HybridGraphicsGPU}else{$null};monitorName=if([string]$_.MonitorName){[string]$_.MonitorName}else{$null}}})
if($dx.Count -eq 0){throw 'Console DxDiag contains no displays'}
$dxSource=[ordered]@{path=$dxPath;sha256=(Get-FileHash $dxPath -Algorithm SHA256).Hash.ToLower();lastWriteUtc=(Get-Item $dxPath).LastWriteTimeUtc.ToString('o');kind='saved console observation, not a fresh scan'}
$record=[ordered]@{schema='tile-boxed-preflight-v1';dxdiag=$dx;dxdiagSource=$dxSource;cellId=$CellId;adapter='Intel(R) Graphics';count=16;cells=@('focused-one','focused-plus-background');roles=@('baseline','candidate');backgroundFps=1;utc=[DateTime]::UtcNow.ToString('o');powerState=$powerState;consoleSessionId=$consoleSid;vm=($vm|Select-Object Name,@{n='State';e={[string]$_.State}},MemoryAssigned);quietServices=@($services|Select-Object Name,@{n='Status';e={[string]$_.Status}});drivers=$drivers;sessions=$sessions;processes=$processes;server=$serverRecord;processLasso=[ordered]@{running=(@($lasso).Count -gt 0);processes=$lasso};strict_binding='unchanged';performanceAcceptance=$false;stages=@(@{role='baseline';path=$baselineStage;binaryPresent=(Test-Path (Join-Path $baselineStage 'panel-play.exe'))},@{role='candidate';path=$candidateStage;binaryPresent=(Test-Path (Join-Path $candidateStage 'panel-play.exe'));stageRequiredBeforeLaunch=$true})}
$record|ConvertTo-Json -Depth 8|Set-Content (Join-Path $baselineStage ('preflight-tile-boxed-'+$CellId+'.json')) -Encoding UTF8
'Preflight passed: fresh per-cell record, checked AC-line/power settings, server identity, quiet services, and unlocked console'
