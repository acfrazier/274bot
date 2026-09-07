param([Parameter(Mandatory=$true)][ValidateSet('focused-one','focused-plus-background')][string]$Mode)
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$stage='C:\ProgramData\274bot-Test\panel-36825a9'; $botHome='C:\Users\BotTest'
$runner=Join-Path $PSScriptRoot ("run-panel-n16-"+$Mode+'-36825a9-intel.py')
if(-not (Test-Path $runner)){throw 'Controller missing'}
if(-not (Test-Path (Join-Path $stage 'panel-play.exe'))){throw 'Frozen binary missing'}
if(-not (Test-Path (Join-Path $stage 'preflight-panel-n16-intel.json'))){throw 'Run matching preflight first'}
$label='n16-'+$Mode+'-console-intel'; $staged=Join-Path $stage ("run-"+$label+'.py'); Copy-Item $runner $staged -Force
$launcher=Join-Path $stage ("launch-"+$label+'.ps1'); $log=Join-Path $botHome ('274bot-runs\managed-panel-36825a9-'+$label+'-launcher.log')
@"
`$ErrorActionPreference='Continue'
`$ProgressPreference='SilentlyContinue'
& 'C:\Program Files\Python314\python.exe' '$staged' *> '$log'
exit `$LASTEXITCODE
"@|Set-Content $launcher -Encoding UTF8
$name='274bot-BotTest-managed-panel-36825a9-'+$label
if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'Task exists; preserve raw output and do not reuse this name'}
$action=New-ScheduledTaskAction -Execute 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$launcher+'"') -WorkingDirectory $stage
$principal=New-ScheduledTaskPrincipal -UserId 'DESKTOP-SL99R6D\BotTest' -LogonType Interactive -RunLevel Limited
$settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 18)
Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings|Out-Null
Start-ScheduledTask -TaskName $name
[ordered]@{task=$name;mode=$Mode;count=16;adapter='Intel(R) Graphics';backgroundFps=if($Mode -eq 'focused-plus-background'){1}else{$null};runnerSha256=(Get-FileHash $runner -Algorithm SHA256).Hash.ToLower();startedUtc=[DateTime]::UtcNow.ToString('o');performanceAcceptance=$false}|ConvertTo-Json
