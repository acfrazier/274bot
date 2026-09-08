param([Parameter(Mandatory=$true)][ValidateSet('baseline','candidate')][string]$BuildRole,[Parameter(Mandatory=$true)][ValidateSet('focused-one','focused-plus-background')][string]$Mode,[Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$CellId)
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$stage=if($BuildRole -eq 'baseline'){'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'}else{'C:\ProgramData\274bot-Test\tile-boxed-fb3589a'}
$prefix="$BuildRole-$Mode-"; if(-not $CellId.StartsWith($prefix)){throw "CellId must start with $prefix"}
$runner=Join-Path $PSScriptRoot 'run-tile-boxed-focused-one.py'
$preflight='C:\ProgramData\274bot-Test\renderer-owner-census-9268890\preflight-tile-boxed-'+$CellId+'.json'
if(-not (Test-Path $runner)){throw 'Controller missing'}; if(-not (Test-Path (Join-Path $stage 'panel-play.exe'))){throw 'Role binary missing'}; if(-not (Test-Path $preflight)){throw 'Fresh per-cell preflight missing'}
$out=Join-Path 'C:\Users\BotTest\274bot-runs' ('managed-tile-boxed-'+$CellId); if(Test-Path $out){throw 'Cell output exists; choose a new cell id'}
Copy-Item $runner (Join-Path $stage ("run-"+$CellId+'.py')) -Force
$launcher=Join-Path $stage ("launch-"+$CellId+'.ps1'); $log=Join-Path 'C:\Users\BotTest\274bot-runs' ('managed-tile-boxed-'+$CellId+'-launcher.log')
@"
`$ErrorActionPreference='Continue'; `$ProgressPreference='SilentlyContinue'
`$env:TILE_BOXED_BUILD_ROLE='$BuildRole'; `$env:TILE_BOXED_MODE='$Mode'; `$env:TILE_BOXED_CELL_ID='$CellId'
& 'C:\Program Files\Python314\python.exe' '$stage\run-$CellId.py' *> '$log'; exit `$LASTEXITCODE
"@|Set-Content $launcher -Encoding UTF8
$name='274bot-BotTest-tile-boxed-'+$CellId; if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'Task exists; preserve prior output'}
$action=New-ScheduledTaskAction -Execute 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$launcher+'"') -WorkingDirectory $stage
$principal=New-ScheduledTaskPrincipal -UserId 'DESKTOP-SL99R6C\BotTest' -LogonType Interactive -RunLevel Limited; $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 18)
Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings|Out-Null; Start-ScheduledTask -TaskName $name
[ordered]@{task=$name;cell_id=$CellId;build_role=$BuildRole;mode=$Mode;count=16;adapter='Intel(R) Graphics';backgroundFps=if($Mode -eq 'focused-plus-background'){1}else{$null};runnerSha256=(Get-FileHash $runner -Algorithm SHA256).Hash.ToLower();startedUtc=[DateTime]::UtcNow.ToString('o');performanceAcceptance=$false}|ConvertTo-Json
