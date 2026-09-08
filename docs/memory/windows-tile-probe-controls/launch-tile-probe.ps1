param([Parameter(Mandatory=$true)][ValidatePattern('^native-render-owner-census-focused-plus-background-[A-Za-z0-9_.-]+$')][string]$CellId)
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$stage='C:\ProgramData\274bot-Test\renderer-owner-census-9268890'; $runner=Join-Path $PSScriptRoot 'run-panel-tile-probe-focused-one.py'
if(-not(Test-Path (Join-Path $stage 'panel-play.exe'))){throw 'Frozen 9268890 binary missing'}
$preflight=Join-Path $stage ('preflight-tile-probe-'+$CellId+'.json'); if(-not(Test-Path $preflight)){throw 'Fresh per-cell preflight missing'}
$out=Join-Path 'C:\Users\BotTest\274bot-runs' $CellId; if(Test-Path $out){throw 'Cell output exists; choose a new cell id'}
Copy-Item $runner (Join-Path $stage ('run-'+$CellId+'.py')) -Force
Copy-Item (Join-Path $PSScriptRoot 'run-panel-tile-probe-focused-one.py') (Join-Path $stage 'run-panel-tile-probe-focused-one.py') -Force
Copy-Item (Join-Path $PSScriptRoot 'run-panel-tile-probe-focused-plus-background.py') (Join-Path $stage 'run-panel-tile-probe-focused-plus-background.py') -Force
$launcher=Join-Path $stage ('launch-'+$CellId+'.ps1'); $log=Join-Path 'C:\Users\BotTest\274bot-runs' ($CellId+'-launcher.log')
@"
`$ErrorActionPreference='Continue'; `$env:RENDER_OWNER_CENSUS_CELL_ID='$CellId'
& 'C:\Program Files\Python314\python.exe' '$stage\run-$CellId.py' *> '$log'; exit `$LASTEXITCODE
"@|Set-Content $launcher -Encoding UTF8
$name='274bot-BotTest-tile-probe-'+$CellId; if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'Task exists; preserve prior output'}
$action=New-ScheduledTaskAction -Execute 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$launcher+'"') -WorkingDirectory $stage
$principal=New-ScheduledTaskPrincipal -UserId 'DESKTOP-SL99R6C\BotTest' -LogonType Interactive -RunLevel Limited; $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 18)
Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings|Out-Null; Start-ScheduledTask -TaskName $name
[ordered]@{task=$name;cell_id=$CellId;stage=$stage;mode='focused-plus-background';count=16;adapter='Intel(R) Graphics';backgroundFps=1;runnerSha256=(Get-FileHash $runner -Algorithm SHA256).Hash.ToLower();startedUtc=[DateTime]::UtcNow.ToString('o');performanceAcceptance=$false}|ConvertTo-Json
