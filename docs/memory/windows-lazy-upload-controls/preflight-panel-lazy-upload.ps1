param()
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$baselineStage='C:\ProgramData\274bot-Test\panel-36825a9'; $candidateStage='C:\ProgramData\274bot-Test\panel-3118e966'
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
Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class ConsoleSessionProbeLazyUpload { [DllImport("kernel32.dll")] public static extern uint WTSGetActiveConsoleSessionId(); }'
$consoleSid=[ConsoleSessionProbeLazyUpload]::WTSGetActiveConsoleSessionId(); $sessions=@(& quser.exe)
if(@($sessions|Where-Object {$_ -match ('^\s*bottest\s+console\s+'+$consoleSid+'\s+Active')}).Count -ne 1){throw 'BotTest local console is not active'}
if(Get-Process LogonUI -ErrorAction SilentlyContinue|Where-Object SessionId -eq $consoleSid){throw 'Console is locked'}
$processes=@(Get-CimInstance Win32_Process|Select-Object ProcessId,ParentProcessId,Name,SessionId,CreationDate,CommandLine)
$lasso=@($processes|Where-Object {$_.Name -match '(?i)^(ProcessLasso|bitsum|LassoGovernor).*\.exe$'})
$serverRecord=[ordered]@{ProcessId=[int]$server.ProcessId;ParentProcessId=[int]$server.ParentProcessId;Name=[string]$server.Name;SessionId=[int]$server.SessionId;CreationDate=[string]$server.CreationDate;CommandLine=$server.CommandLine}
$record=[ordered]@{schema='lazy-upload-preflight-v1';adapter='Intel(R) Graphics';count=16;cells=@('focused-one','focused-plus-background');roles=@('baseline','candidate');backgroundFps=1;utc=[DateTime]::UtcNow.ToString('o');consoleSessionId=$consoleSid;vm=($vm|Select-Object Name,@{n='State';e={[string]$_.State}},MemoryAssigned);quietServices=@($services|Select-Object Name,@{n='Status';e={[string]$_.Status}});drivers=$drivers;sessions=$sessions;processes=$processes;server=$serverRecord;processLasso=[ordered]@{running=(@($lasso).Count -gt 0);processes=$lasso};strict_binding='unchanged';performanceAcceptance=$false;stages=@(@{role='baseline';path=$baselineStage;binaryPresent=(Test-Path (Join-Path $baselineStage 'panel-play.exe'))},@{role='candidate';path=$candidateStage;binaryPresent=(Test-Path (Join-Path $candidateStage 'panel-play.exe'));stageRequiredBeforeLaunch=$true})}
$record|ConvertTo-Json -Depth 8|Set-Content (Join-Path $baselineStage 'preflight-panel-lazy-upload.json') -Encoding UTF8
'Preflight passed: paired stages, checked server identity, quiet services, and unlocked console'
